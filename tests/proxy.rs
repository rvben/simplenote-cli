use serde_json::{Value, json};
use simplenote_cli::api::{Api, Http};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    thread,
    time::{Duration, Instant},
};

// Each request runs in a fresh process: proxy/CA environment changes must not
// race other tests or reuse an already-initialized HTTP client/trust store.
#[test]
fn proxy_request_child() {
    let Ok(endpoint) = std::env::var("SIMPLENOTE_PROXY_TEST_ENDPOINT") else {
        return;
    };
    let http = Http::with_urls(&endpoint, &endpoint).unwrap();
    let result = if std::env::var_os("SIMPLENOTE_PROXY_TEST_LOGIN").is_some() {
        http.login(json!({"auth_code":"private-login-code"}))
    } else {
        Api {
            http: &http,
            token: "private-simperium-token".into(),
        }
        .index_page(1, None)
    };
    let value = match result {
        Ok(value) => json!({"ok":value}),
        Err(error) => error.envelope(),
    };
    println!("PROXY_RESULT={value}");
}

fn request(endpoint: &str, env: &[(&str, &str)]) -> Value {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "proxy_request_child", "--nocapture"]);
    for key in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
        "REQUEST_METHOD",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "SIMPLENOTE_PROXY_TEST_LOGIN",
    ] {
        command.env_remove(key);
    }
    command.env("SIMPLENOTE_PROXY_TEST_ENDPOINT", endpoint);
    // Prevent platform settings from routing these isolated tests elsewhere.
    command.env("ALL_PROXY", "http://127.0.0.1:1");
    command.env("NO_PROXY", "");
    if env.iter().any(|(key, _)| *key == "all_proxy") {
        command.env_remove("ALL_PROXY");
    }
    command.envs(env.iter().copied());
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let result = stdout
        .lines()
        .find_map(|line| line.strip_prefix("PROXY_RESULT="))
        .unwrap_or_else(|| panic!("Missing result: {stdout}"));
    for secret in [
        "private-login-code",
        "private-simperium-token",
        "proxy-secret",
    ] {
        assert!(!result.contains(secret), "Leaked secret: {result}");
    }
    serde_json::from_str(result).unwrap()
}

fn server(f: impl FnOnce(TcpStream) + Send + 'static) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    listener.set_nonblocking(true).unwrap();
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "Proxy was never contacted");
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("Accept failed: {e}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        f(stream);
    });
    (address, worker)
}

fn headers(stream: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16384);
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

fn reply(stream: &mut impl Write, body: &str) {
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    stream.flush().unwrap();
}

#[test]
fn http_proxy_environment_routing_and_basic_auth() {
    for variable in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        let (address, worker) = server(|mut stream| {
            let request = headers(&mut stream);
            assert!(request.starts_with("GET http://proxy-target.invalid/note/index?"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("proxy-authorization: basic ")
            );
            reply(&mut stream, r#"{"index":[]}"#);
        });
        let proxy = format!("http://user:proxy-secret@{address}");
        let result = request("http://proxy-target.invalid/", &[(variable, &proxy)]);
        worker.join().unwrap();
        assert_eq!(result["ok"]["index"], json!([]));
    }
}

#[test]
fn https_connect_auth_failure_is_actionable_for_api_and_login() {
    for (variable, login) in [
        ("HTTPS_PROXY", false),
        ("https_proxy", true),
        ("ALL_PROXY", false),
    ] {
        let (address, worker) = server(|mut stream| {
            let request = headers(&mut stream);
            assert!(request.starts_with("CONNECT proxy-target.invalid:443 HTTP/1.1"));
            assert!(!request.contains("private-simperium-token"));
            assert!(!request.contains("private-login-code"));
            stream
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
        });
        let proxy = format!("http://user:proxy-secret@{address}");
        let mut env = vec![(variable, proxy.as_str())];
        if login {
            env.push(("SIMPLENOTE_PROXY_TEST_LOGIN", "1"));
        }
        let result = request("https://proxy-target.invalid/", &env);
        worker.join().unwrap();
        assert_eq!(result["error"]["kind"], "network");
        assert!(
            result["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Proxy authentication required (HTTP 407)")
        );
    }
}

#[test]
fn no_proxy_bypasses_a_broken_proxy() {
    let (address, worker) = server(|mut stream| {
        assert!(headers(&mut stream).starts_with("GET /note/index?"));
        reply(&mut stream, r#"{"index":[]}"#);
    });
    let result = request(&format!("http://{address}/"), &[("NO_PROXY", "127.0.0.1")]);
    worker.join().unwrap();
    assert_eq!(result["ok"]["index"], json!([]));
}

#[test]
fn proxy_failures_preserve_network_exit_code_and_explain_the_cause() {
    for (endpoint, response, expected) in [
        (
            "https://proxy-target.invalid/",
            "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n",
            "Proxy tunnel failed",
        ),
        (
            "http://proxy-target.invalid/",
            "HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
            "Proxy authentication required (HTTP 407)",
        ),
    ] {
        let (address, worker) = server(move |mut stream| {
            headers(&mut stream);
            stream.write_all(response.as_bytes()).unwrap();
        });
        let result = request(
            endpoint,
            &[("ALL_PROXY", &format!("http://user:proxy-secret@{address}"))],
        );
        worker.join().unwrap();
        assert_eq!(result["error"]["exit_code"], 5);
        assert_eq!(result["error"]["kind"], "network");
        assert!(
            result["error"]["message"]
                .as_str()
                .unwrap()
                .contains(expected),
            "{result}"
        );
    }
    // Reserve a port and close it so refusal is local and doesn't require DNS.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let result = request(
        "https://proxy-target.invalid/",
        &[("ALL_PROXY", &format!("http://{address}"))],
    );
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Connection refused"),
        "{result}"
    );
}

#[test]
fn socks5h_resolves_destination_at_proxy_and_transports_requests() {
    for authenticated in [false, true] {
        let (address, worker) = server(move |mut stream| {
            let mut greeting = [0; 2];
            stream.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0; greeting[1] as usize];
            stream.read_exact(&mut methods).unwrap();
            let method = if authenticated { 2 } else { 0 };
            assert!(methods.contains(&method));
            stream.write_all(&[5, method]).unwrap();
            if authenticated {
                let mut auth = [0; 2];
                stream.read_exact(&mut auth).unwrap();
                assert_eq!(auth[0], 1);
                let mut username = vec![0; auth[1] as usize];
                stream.read_exact(&mut username).unwrap();
                assert_eq!(username, b"user");
                let mut length = [0];
                stream.read_exact(&mut length).unwrap();
                let mut password = vec![0; length[0] as usize];
                stream.read_exact(&mut password).unwrap();
                assert_eq!(password, b"proxy-secret");
                stream.write_all(&[1, 0]).unwrap();
            }
            let mut connect = [0; 5];
            stream.read_exact(&mut connect).unwrap();
            assert_eq!(&connect[..4], &[5, 1, 0, 3]);
            let mut hostname = vec![0; connect[4] as usize];
            stream.read_exact(&mut hostname).unwrap();
            assert_eq!(hostname, b"proxy-target.invalid");
            let mut port = [0; 2];
            stream.read_exact(&mut port).unwrap();
            assert_eq!(u16::from_be_bytes(port), 80);
            stream
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .unwrap();
            let request = headers(&mut stream);
            assert!(request.starts_with("GET /note/index?"));
            assert!(!request.to_ascii_lowercase().contains("proxy-authorization"));
            reply(&mut stream, r#"{"index":[]}"#);
        });
        let credentials = if authenticated {
            "user:proxy-secret@"
        } else {
            ""
        };
        let result = request(
            "http://proxy-target.invalid/",
            &[("ALL_PROXY", &format!("socks5h://{credentials}{address}"))],
        );
        worker.join().unwrap();
        assert_eq!(result["ok"]["index"], json!([]));
    }
}

#[test]
fn tls_intercepting_proxy_requires_a_trusted_ca() {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["proxy-target.invalid".into()]).unwrap();
    let mut bundle = tempfile::NamedTempFile::new().unwrap();
    bundle.write_all(cert.pem().as_bytes()).unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    let config = std::sync::Arc::new(config);
    for trusted in [false, true] {
        let config = config.clone();
        let (address, worker) = server(move |mut stream| {
            assert!(headers(&mut stream).starts_with("CONNECT proxy-target.invalid:443 HTTP/1.1"));
            stream
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .unwrap();
            let connection = rustls::ServerConnection::new(config).unwrap();
            let mut tls = rustls::StreamOwned::new(connection, stream);
            if trusted {
                assert!(headers(&mut tls).starts_with("GET /note/index?"));
                reply(&mut tls, r#"{"index":[]}"#);
            } else {
                assert!(tls.conn.complete_io(&mut tls.sock).is_err());
            }
        });
        let proxy = format!("http://{address}");
        let mut env = vec![("HTTPS_PROXY", proxy.as_str())];
        if trusted {
            env.push(("SSL_CERT_FILE", bundle.path().to_str().unwrap()));
        }
        let result = request("https://proxy-target.invalid/", &env);
        worker.join().unwrap();
        if trusted {
            assert_eq!(result["ok"]["index"], json!([]));
        } else {
            assert_eq!(result["error"]["kind"], "network");
            assert!(
                result["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("TLS certificate verification failed")
            );
        }
    }
}

#[test]
fn both_cli_commands_list_and_login_through_a_trusted_https_proxy() {
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec![
        "api.simperium.com".into(),
        "app.simplenote.com".into(),
    ])
    .unwrap();
    let mut bundle = tempfile::NamedTempFile::new().unwrap();
    bundle.write_all(cert.pem().as_bytes()).unwrap();
    let config = std::sync::Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap(),
    );
    for binary in [
        env!("CARGO_BIN_EXE_simplenote"),
        env!("CARGO_BIN_EXE_simplenote-cli"),
    ] {
        for (login, format) in [
            (false, "json"),
            (true, "json"),
            (false, "text"),
            (true, "text"),
        ] {
            let config = config.clone();
            let (address, worker) = server(move |mut stream| {
                let connect = headers(&mut stream);
                let host = if login {
                    "app.simplenote.com"
                } else {
                    "api.simperium.com"
                };
                assert!(connect.starts_with(&format!("CONNECT {host}:443 HTTP/1.1")));
                assert!(!connect.contains("private-simperium-token"));
                stream
                    .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                    .unwrap();
                let connection = rustls::ServerConnection::new(config).unwrap();
                let mut tls = rustls::StreamOwned::new(connection, stream);
                let request = headers(&mut tls);
                if login {
                    assert!(request.starts_with("POST /account/complete-login HTTP/1.1"));
                    let length: usize = request
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    let mut body = vec![0; length];
                    tls.read_exact(&mut body).unwrap();
                    assert_eq!(
                        serde_json::from_slice::<Value>(&body).unwrap(),
                        json!({"username":"probe@example.invalid", "auth_code":"ABC123"})
                    );
                    reply(&mut tls, r#"{"sync_token":"private-simperium-token"}"#);
                } else {
                    assert!(request.starts_with("GET /1/chalk-bump-f49/note/index?"));
                    assert!(
                        request
                            .to_ascii_lowercase()
                            .contains("x-simperium-token: private-simperium-token")
                    );
                    reply(
                        &mut tls,
                        r#"{"index":[{"id":"fixture-note","v":7,"d":{"content":"Travel plans\n\nA quiet weekend.","tags":["travel"],"systemTags":["pinned"],"deleted":false,"modificationDate":1735779600}}]}"#,
                    );
                }
            });
            let directory = tempfile::tempdir().unwrap();
            let mut command = Command::new(binary);
            for key in [
                "HTTP_PROXY",
                "http_proxy",
                "HTTPS_PROXY",
                "https_proxy",
                "ALL_PROXY",
                "all_proxy",
                "NO_PROXY",
                "no_proxy",
                "SSL_CERT_FILE",
                "SSL_CERT_DIR",
                "SIMPLENOTE_TOKEN",
            ] {
                command.env_remove(key);
            }
            command
                .env("HTTPS_PROXY", format!("http://{address}"))
                .env("NO_PROXY", "")
                .env("SSL_CERT_FILE", bundle.path())
                .env("SIMPLENOTE_CONFIG_DIR", directory.path())
                .args(["--output", format]);
            if login {
                command.args(["auth", "login", "--code-stdin"]);
            } else {
                command
                    .args(["notes", "list", "--limit", "1"])
                    .env("SIMPLENOTE_TOKEN", "private-simperium-token");
            }
            let mut child = command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            if login {
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(b"probe@example.invalid:ABC123\n")
                    .unwrap();
            }
            let output = child.wait_with_output().unwrap();
            worker.join().unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            assert!(!String::from_utf8_lossy(&output.stdout).contains("private-simperium-token"));
            if format == "json" {
                let result: Value = serde_json::from_slice(&output.stdout).unwrap();
                if login {
                    assert_eq!(result["authenticated"], true);
                } else {
                    assert_eq!(result["items"][0]["id"], "fixture-note");
                    assert_eq!(result["items"][0]["title"], "Travel plans");
                }
            } else {
                let text = String::from_utf8(output.stdout).unwrap();
                assert!(!text.contains('\x1b'));
                assert!(text.contains(if login {
                    "Signed in to Simplenote"
                } else {
                    "Travel plans"
                }));
                if !login {
                    assert!(text.contains("fixture-note · v7"));
                    assert!(text.contains("2025-01-02 01:00 UTC"));
                    assert!(text.contains("Tags  travel"));
                    assert!(!text.contains("A quiet weekend."));
                }
            }
            if login {
                assert_eq!(
                    simplenote_cli::auth::read(directory.path()).unwrap(),
                    "private-simperium-token"
                );
            }
        }
    }
}
