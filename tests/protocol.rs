use clap::Parser;
use mockito::{Matcher, Server};
use serde_json::{Value, json};
use simplenote_cli::{
    api::{Api, Http},
    auth,
    cli::{Cli, execute},
};
fn note(content: &str) -> Value {
    json!({"content": content, "tags": ["work"], "systemTags": ["markdown", "future-flag"], "deleted": false, "creationDate": 100, "modificationDate": 200, "publishURL": "preserve-me"})
}
fn http(server: &Server) -> Http {
    Http::with_urls(
        &format!("{}/1/test/", server.url()),
        &format!("{}/", server.url()),
    )
    .unwrap()
}
#[test]
fn browser_code_exchange_and_private_persistence() {
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/account/complete-login")
        .match_body(Matcher::Json(
            json!({"username": "test@example.invalid", "auth_code": "ABC123"}),
        ))
        .with_body(r#"{"sync_token":"test-secret"}"#)
        .create();
    let token = auth::redeem(&http(&server), "test@example.invalid:abc123\n").unwrap();
    let dir = tempfile::tempdir().unwrap();
    auth::save(dir.path(), token).unwrap();
    assert_eq!(auth::read(dir.path()).unwrap(), "test-secret");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(auth::logout(dir.path()).unwrap());
    assert!(!auth::logout(dir.path()).unwrap());
    mock.assert();
}
#[test]
fn login_failures_never_include_credentials_or_response_secrets() {
    let mut server = Server::new();
    let mock = server
        .mock("POST", "/account/complete-login")
        .with_status(401)
        .with_body("test-secret")
        .expect(1)
        .create();
    let err = auth::redeem(&http(&server), "test@example.invalid:SECRET").unwrap_err();
    assert_eq!(err.kind, "auth");
    assert!(!err.to_string().contains("SECRET"));
    assert!(!err.to_string().contains("test-secret"));
    mock.assert();
}
#[test]
fn get_reads_version_and_preserves_exact_content() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/1/test/note/i/abc")
        .match_header("x-simperium-token", "secret")
        .with_header("X-Simperium-Version", "42")
        .with_body(note("Title\n\nbody\n").to_string())
        .create();
    let http = http(&server);
    let api = Api {
        http: &http,
        token: "secret".into(),
    };
    let got = api.get("abc", None).unwrap();
    assert_eq!(got["version"], 42);
    assert_eq!(got["content"], "Title\n\nbody\n");
    assert_eq!(got["markdown"], true);
    mock.assert();
}
#[test]
fn filtered_pagination_continues_across_empty_matches() {
    let mut server = Server::new();
    // Exact query match distinguishes continuation from the first request.
    let page1 = server
        .mock("GET", "/1/test/note/index?data=true&limit=1")
        .with_body(
            json!({"index": [{"id":"a", "v": 1, "d": note("other")}], "mark": "next+page"})
                .to_string(),
        )
        .create();
    let page2 = server
        .mock(
            "GET",
            "/1/test/note/index?data=true&limit=1&mark=next%2Bpage",
        )
        .with_body(
            json!({"index": [{"id":"b", "v": 2, "d": note("Needle")}], "mark": "final"})
                .to_string(),
        )
        .create();
    let http = http(&server);
    let api = Api {
        http: &http,
        token: "secret".into(),
    };
    let page = api
        .list(1, None, Some("work"), Some("needle"), false)
        .unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["id"], "b");
    assert_eq!(page["next_cursor"], "final");
    page1.assert();
    page2.assert();
}
#[test]
fn malformed_index_is_an_error_not_an_empty_account() {
    let mut server = Server::new();
    let mock = server
        .mock("GET", "/1/test/note/index")
        .match_query(Matcher::Any)
        .with_body(r#"{"unexpected":[]}"#)
        .create();
    let http = http(&server);
    assert_eq!(
        Api {
            http: &http,
            token: "x".into()
        }
        .list(20, None, None, None, false)
        .unwrap_err()
        .kind,
        "invalid_response"
    );
    mock.assert();
}
#[test]
fn stale_edit_does_not_write() {
    let mut server = Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "43")
        .with_body(note("remote").to_string())
        .create();
    let write = server.mock("POST", Matcher::Any).expect(0).create();
    let http = http(&server);
    let api = Api {
        http: &http,
        token: "x".into(),
    };
    assert_eq!(
        api.change("abc", 42, json!({"content":"mine"}), false)
            .unwrap_err()
            .kind,
        "conflict"
    );
    read.assert();
    write.assert();
}
#[test]
fn update_sends_only_patch_against_base_and_returns_server_merge() {
    let mut server = Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "42")
        .with_body(note("base").to_string())
        .create();
    let actual = server
        .mock("POST", "/1/test/note/i/abc/v/42")
        .match_query(Matcher::Any)
        .match_body(Matcher::Regex(
            r#"^\{"content":"mine","modificationDate":[0-9]+\}$"#.into(),
        ))
        .with_header("x-simperium-version", "44")
        .with_body(note("mine plus concurrent edit").to_string())
        .create();
    let http = http(&server);
    let got = Api {
        http: &http,
        token: "x".into(),
    }
    .change("abc", 42, json!({"content":"mine"}), false)
    .unwrap();
    assert_eq!(got["version"], 44);
    assert_eq!(got["content"], "mine plus concurrent edit");
    read.assert();
    actual.assert();
}
#[test]
fn dry_run_and_no_op_make_no_post() {
    let mut server = Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "1")
        .with_body(note("base").to_string())
        .expect(2)
        .create();
    let write = server.mock("POST", Matcher::Any).expect(0).create();
    let http = http(&server);
    let api = Api {
        http: &http,
        token: "x".into(),
    };
    assert_eq!(
        api.change("abc", 1, json!({"content":"mine"}), true)
            .unwrap()["dry_run"],
        true
    );
    assert_eq!(
        api.change("abc", 1, json!({"content":"base"}), false)
            .unwrap()["no_op"],
        true
    );
    read.assert();
    write.assert();
}
#[test]
fn write_errors_are_not_retried() {
    let mut server = Server::new();
    let write = server
        .mock("POST", Matcher::Any)
        .with_status(503)
        .with_body("secret")
        .expect(1)
        .create();
    let http = http(&server);
    let err = Api {
        http: &http,
        token: "x".into(),
    }
    .create("hello", &[], false)
    .unwrap_err();
    assert_eq!(err.kind, "remote");
    assert!(!err.to_string().contains("secret"));
    assert_eq!(err.envelope()["error"]["details"]["operation"], "create");
    assert_eq!(err.envelope()["error"]["details"]["outcome_unknown"], true);
    assert!(
        uuid::Uuid::parse_str(
            err.envelope()["error"]["details"]["note_id"]
                .as_str()
                .unwrap()
        )
        .is_ok()
    );
    write.assert();
}
#[test]
fn ids_cannot_escape_the_note_bucket() {
    let server = Server::new();
    let http = http(&server);
    let api = Api {
        http: &http,
        token: "x".into(),
    };
    for id in ["../tag/i/x", "..", ".", "a/b", "a?x=y", "a%2fb", ""] {
        assert_eq!(api.get(id, None).unwrap_err().kind, "usage");
    }
}
#[test]
fn redirect_does_not_forward_token() {
    let mut server = Server::new();
    let redirect = server
        .mock("GET", "/1/test/note/i/abc")
        .with_status(302)
        .with_header("location", "https://example.invalid/")
        .create();
    let http = http(&server);
    assert_eq!(
        Api {
            http: &http,
            token: "x".into()
        }
        .get("abc", None)
        .unwrap_err()
        .kind,
        "remote"
    );
    redirect.assert();
}
#[test]
fn export_is_private_includes_trash_and_refuses_existing_directory() {
    let mut server = Server::new();
    let mut trashed = note("trashed");
    trashed["deleted"] = json!(true);
    let index = server
        .mock("GET", "/1/test/note/index")
        .match_query(Matcher::Any)
        .with_body(json!({"index":[{"id":"abc","v":1,"d":trashed}]}).to_string())
        .expect(1)
        .create();
    let http = http(&server);
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    auth::save(&config, "test".into()).unwrap();
    let dest = root.path().join("export");
    let cli = Cli::try_parse_from([
        "simplenote-cli",
        "export",
        "--directory",
        dest.to_str().unwrap(),
    ])
    .unwrap();
    let result = execute(&cli, &http, &config).unwrap().unwrap();
    assert_eq!(result["exported"], 1);
    let exported_path = std::fs::read_dir(&dest)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let note: Value = serde_json::from_slice(&std::fs::read(&exported_path).unwrap()).unwrap();
    assert_eq!(note["raw"]["publishURL"], "preserve-me");
    assert_eq!(note["deleted"], true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&exported_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(execute(&cli, &http, &config).unwrap_err().kind, "io");
    index.assert();
}
#[cfg(unix)]
#[test]
fn credential_symlinks_and_loose_permissions_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let other = root.path().join("other.json");
    std::fs::write(&other, r#"{"token":"secret"}"#).unwrap();
    symlink(&other, config.join("auth.json")).unwrap();
    assert_eq!(auth::read(&config).unwrap_err().kind, "auth");
    let private = root.path().join("private");
    auth::save(&private, "secret".into()).unwrap();
    std::fs::set_permissions(
        private.join("auth.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(auth::read(&private).unwrap_err().kind, "auth");
}

#[test]
fn history_pages_versions_and_marks_missing_revisions() {
    let mut server = Server::new();
    let current = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "4")
        .with_body(note("latest").to_string())
        .create();
    let v4 = server
        .mock("GET", "/1/test/note/i/abc/v/4")
        .with_header("x-simperium-version", "4")
        .with_body(note("latest").to_string())
        .create();
    let missing = server
        .mock("GET", "/1/test/note/i/abc/v/3")
        .with_status(404)
        .create();
    let http = http(&server);
    let page = Api {
        http: &http,
        token: "x".into(),
    }
    .history("abc", 2, None)
    .unwrap();
    assert_eq!(page["items"][0]["version"], 4);
    assert_eq!(page["items"][1]["available"], false);
    assert_eq!(page["next_cursor"], "2");
    current.assert();
    v4.assert();
    missing.assert();
}
#[test]
fn repeated_index_cursor_fails_instead_of_looping() {
    let mut server = Server::new();
    let repeated = server
        .mock("GET", "/1/test/note/index")
        .match_query(Matcher::Any)
        .with_body(r#"{"index":[],"mark":"same"}"#)
        .expect(1)
        .create();
    let http = http(&server);
    let error = Api {
        http: &http,
        token: "x".into(),
    }
    .list(20, Some("same"), None, None, false)
    .unwrap_err();
    assert_eq!(error.kind, "invalid_response");
    repeated.assert();
}
#[test]
fn empty_change_412_reads_the_authoritative_note() {
    let mut server = Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "1")
        .with_body(note("base").to_string())
        .expect(2)
        .create();
    let write = server
        .mock("POST", "/1/test/note/i/abc/v/1")
        .match_query(Matcher::Any)
        .with_status(412)
        .expect(1)
        .create();
    let http = http(&server);
    let result = Api {
        http: &http,
        token: "x".into(),
    }
    .change("abc", 1, json!({"content":"mine"}), false)
    .unwrap();
    assert_eq!(result["no_op"], true);
    assert_eq!(result["note"]["content"], "base");
    read.assert();
    write.assert();
}
#[test]
fn missing_write_version_does_not_trigger_a_second_write() {
    let mut server = Server::new();
    let write = server
        .mock("POST", Matcher::Any)
        .with_body(note("saved").to_string())
        .expect(1)
        .create();
    let http = http(&server);
    let error = Api {
        http: &http,
        token: "x".into(),
    }
    .create("saved", &[], false)
    .unwrap_err();
    assert_eq!(error.kind, "invalid_response");
    write.assert();
}
#[test]
fn field_projection_is_applied_and_text_stays_readable() {
    let mut server = Server::new();
    let index = server
        .mock("GET", "/1/test/note/index")
        .match_query(Matcher::Any)
        .with_body(json!({"index":[{"id":"abc","v":1,"d":note("Café\u{1b}[31m")}]}).to_string())
        .create();
    let root = tempfile::tempdir().unwrap();
    auth::save(root.path(), "test".into()).unwrap();
    let cli =
        Cli::try_parse_from(["simplenote-cli", "notes", "list", "--fields", "title"]).unwrap();
    let http = http(&server);
    let result = execute(&cli, &http, root.path()).unwrap().unwrap();
    assert!(result["items"][0].get("id").is_none());
    let mut text = Vec::new();
    simplenote_cli::cli::render(&result, false, &mut text).unwrap();
    let text = String::from_utf8(text).unwrap();
    assert!(text.contains("Café"));
    assert!(!text.contains('\u{1b}'));
    index.assert();
}
#[test]
fn trash_confirmation_refuses_before_network_and_dry_run_only_reads() {
    let mut server = Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "1")
        .with_body(note("base").to_string())
        .expect(1)
        .create();
    let write = server.mock("POST", Matcher::Any).expect(0).create();
    let dir = tempfile::tempdir().unwrap();
    auth::save(dir.path(), "test".into()).unwrap();
    let http = http(&server);
    let cli = Cli::try_parse_from([
        "simplenote-cli",
        "notes",
        "trash",
        "abc",
        "--base-version",
        "1",
    ])
    .unwrap();
    assert_eq!(
        execute(&cli, &http, dir.path()).unwrap_err().kind,
        "confirmation_required"
    );
    let cli = Cli::try_parse_from([
        "simplenote-cli",
        "notes",
        "trash",
        "abc",
        "--base-version",
        "1",
        "--dry-run",
    ])
    .unwrap();
    assert_eq!(
        execute(&cli, &http, dir.path()).unwrap().unwrap()["dry_run"],
        true
    );
    read.assert();
    write.assert();
}
#[test]
fn returned_note_matches_its_discovered_output_schema() {
    let mut server = Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "1")
        .with_body(note("base").to_string())
        .create();
    let http = http(&server);
    let result = Api {
        http: &http,
        token: "x".into(),
    }
    .get("abc", None)
    .unwrap();
    let contract = simplenote_cli::schema::contract();
    let command = contract["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "notes get")
        .unwrap();
    let validator = jsonschema::validator_for(&command["stdout_schema"]).unwrap();
    assert!(validator.is_valid(&result));
    read.assert();
}

#[test]
fn sharing_and_invalid_tags_refuse_before_any_write() {
    let mut server = Server::new();
    let write = server.mock("POST", Matcher::Any).expect(0).create();
    let http = http(&server);
    let api = Api {
        http: &http,
        token: "x".into(),
    };
    for tag in ["person@example.invalid", "", "two words", "a,b", "\u{1b}"] {
        assert_eq!(
            api.create("private", &[tag.into()], false)
                .unwrap_err()
                .kind,
            "usage"
        );
    }
    write.assert();
}
#[test]
fn create_deduplicates_tags_and_sends_expected_fields() {
    let mut server = Server::new();
    let write = server
        .mock("POST", Matcher::Regex(r"^/1/test/note/i/[0-9a-f-]+".into()))
        .match_body(Matcher::PartialJson(
            json!({"content":"hello", "tags":["Work"], "systemTags":["markdown"], "deleted":false}),
        ))
        .with_header("x-simperium-version", "1")
        .with_body(note("hello").to_string())
        .expect(1)
        .create();
    let http = http(&server);
    let result = Api {
        http: &http,
        token: "x".into(),
    }
    .create("hello", &["Work".into(), "work".into()], false)
    .unwrap();
    assert_eq!(result["content"], "hello");
    write.assert();
}
#[test]
fn legacy_notes_can_omit_optional_tag_arrays() {
    let raw = json!({"content":"legacy", "deleted":0});
    let result = simplenote_cli::api::normalize("abc", &raw, 1).unwrap();
    assert_eq!(result["tags"], json!([]));
    assert_eq!(result["markdown"], false);
    for raw in [
        json!({"content":"x", "tags":false}),
        json!({"content":"x", "deleted":{}}),
        json!({"content":"x", "creationDate":[]}),
    ] {
        assert_eq!(
            simplenote_cli::api::normalize("abc", &raw, 1)
                .unwrap_err()
                .kind,
            "invalid_response"
        );
    }
}
#[test]
fn selective_search_returns_a_cursor_instead_of_discarding_progress() {
    let mut server = Server::new();
    let mocks: Vec<_> = (0..20)
        .map(|i| {
            let path = if i == 0 {
                "/1/test/note/index?data=true&limit=1".to_string()
            } else {
                format!("/1/test/note/index?data=true&limit=1&mark={i}")
            };
            server
                .mock("GET", path.as_str())
                .with_body(json!({"index":[],"mark":(i+1).to_string()}).to_string())
                .expect(1)
                .create()
        })
        .collect();
    let http = http(&server);
    let result = Api {
        http: &http,
        token: "x".into(),
    }
    .list(1, None, None, Some("needle"), false)
    .unwrap();
    assert_eq!(result["items"], json!([]));
    assert_eq!(result["scan_limited"], true);
    assert_eq!(result["next_cursor"], "20");
    for mock in mocks {
        mock.assert();
    }
}
#[test]
fn export_preserves_irregular_records_unknown_fields_and_case_distinct_ids() {
    let mut server = Server::new();
    let index = server
        .mock("GET", "/1/test/note/index")
        .match_query(Matcher::Any)
        .with_body(
            json!({"index":[
                {"id":"A","v":1,"d":note("upper")}, {"id":"a","v":1,"d":note("lower")},
                {"id":"con","v":2,"d":{"content":false,"unknown":{"keep":true}}}
            ]})
            .to_string(),
        )
        .create();
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    auth::save(&config, "x".into()).unwrap();
    let dest = root.path().join("export");
    let cli = Cli::try_parse_from([
        "simplenote-cli",
        "export",
        "--directory",
        dest.to_str().unwrap(),
    ])
    .unwrap();
    let http = http(&server);
    assert_eq!(
        execute(&cli, &http, &config).unwrap().unwrap()["exported"],
        3
    );
    let mut ids = std::collections::HashSet::new();
    for path in std::fs::read_dir(&dest).unwrap() {
        let path = path.unwrap().path();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            path.file_name().unwrap().to_str().unwrap().to_lowercase()
        );
        let raw: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        ids.insert(raw["id"].as_str().unwrap().to_owned());
        if raw["id"] == "con" {
            assert_eq!(raw["normalized"], false);
            assert_eq!(raw["raw"]["unknown"]["keep"], true);
        } else {
            assert_eq!(raw["raw"]["publishURL"], "preserve-me");
        }
    }
    assert_eq!(ids.len(), 3);
    index.assert();
}
#[test]
fn terminal_content_escapes_control_and_direction_characters_but_keeps_lines() {
    let safe =
        simplenote_cli::cli::safe_terminal_content("Café\n\ttext\u{1b}]52;clipboard\u{7}\u{202e}");
    assert!(safe.starts_with("Café\n\ttext"));
    assert!(!safe.contains('\u{1b}'));
    assert!(!safe.contains('\u{7}'));
    assert!(!safe.contains('\u{202e}'));
}
#[cfg(target_os = "linux")]
#[test]
fn export_non_utf8_directory_returns_success_without_panicking() {
    use std::os::unix::ffi::OsStringExt;
    let mut server = Server::new();
    let index = server
        .mock("GET", "/1/test/note/index")
        .match_query(Matcher::Any)
        .with_body(r#"{"index":[]}"#)
        .create();
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    auth::save(&config, "x".into()).unwrap();
    let dest = root
        .path()
        .join(std::ffi::OsString::from_vec(vec![b'x', 255]));
    let cli = Cli::try_parse_from([
        std::ffi::OsString::from("simplenote-cli"),
        "export".into(),
        "--directory".into(),
        dest.as_os_str().to_owned(),
    ])
    .unwrap();
    let http = http(&server);
    assert_eq!(
        execute(&cli, &http, &config).unwrap().unwrap()["exported"],
        0
    );
    index.assert();
}

#[test]
fn list_skips_irregular_records_and_matches_tags_case_insensitively() {
    let mut server = Server::new();
    let first = server.mock("GET", "/1/test/note/index?data=true&limit=2")
        .with_body(json!({"index":[{"id":"bad","v":1,"d":{"tags":false}},{"id":"good","v":2,"d":{"content":"needle","tags":["Work"]}}],"mark":"next"}).to_string()).create();
    let second = server
        .mock("GET", "/1/test/note/index?data=true&limit=1&mark=next")
        .with_body(json!({"index":[{"id":"bad2","v":1,"d":{}}]}).to_string())
        .create();
    let http = http(&server);
    let result = Api {
        http: &http,
        token: "x".into(),
    }
    .list(2, None, Some("work"), Some("NEEDLE"), false)
    .unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["id"], "good");
    assert_eq!(result["skipped_irregular"], 2);
    let mut text = Vec::new();
    simplenote_cli::cli::render(&result, false, &mut text).unwrap();
    assert!(
        String::from_utf8(text)
            .unwrap()
            .contains("Skipped malformed records: 2")
    );
    assert_eq!(result["has_more"], false);
    first.assert();
    second.assert();
}
