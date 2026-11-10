use serde_json::Value;
use std::process::{Command, Stdio};
const BIN: &str = env!("CARGO_BIN_EXE_simplenote-cli");
fn run(args: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    Command::new(BIN)
        .args(args)
        .env_remove("SIMPLENOTE_TOKEN")
        .env("SIMPLENOTE_CONFIG_DIR", dir.path())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
fn parse(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}
#[test]
fn discovery_and_no_credentials_are_safe() {
    let out = run(&["schema"]);
    assert!(out.status.success());
    assert_eq!(parse(&out.stdout)["clispec"], "0.3");
    let out = run(&["auth", "status"]);
    assert_eq!(parse(&out.stdout)["authenticated"], false);
    let out = run(&["auth", "login"]);
    assert!(out.status.success());
    assert_eq!(
        parse(&out.stdout)["login_url"],
        simplenote_cli::auth::LOGIN_URL
    );
}
#[test]
fn parse_failures_are_structured_and_nonzero() {
    for args in [
        vec![],
        vec!["notes", "get"],
        vec!["notes", "list", "--limit", "0"],
        vec!["notes", "update", "abc", "--file", "x"],
    ] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert_eq!(parse(&out.stderr)["error"]["kind"], "usage");
    }
}
#[test]
fn explicit_text_parse_error_has_no_json() {
    for args in [
        ["-o", "text", "unknown"].as_slice(),
        ["-otext", "unknown"].as_slice(),
        ["--output=text", "unknown"].as_slice(),
    ] {
        let out = run(args);
        assert!(String::from_utf8_lossy(&out.stderr).starts_with("usage:"));
    }
}
#[test]
fn missing_auth_is_actionable() {
    let out = run(&["notes", "list"]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(parse(&out.stderr)["error"]["kind"], "auth");
}
#[test]
fn edit_refuses_without_a_terminal_and_declares_that_requirement() {
    let output = run(&["notes", "edit", "abc"]);
    assert_eq!(output.status.code(), Some(12));
    assert!(output.stdout.is_empty());
    assert_eq!(parse(&output.stderr)["error"]["kind"], "tty_required");
    let schema = parse(&run(&["schema"]).stdout);
    let edit = schema["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "notes edit")
        .unwrap();
    assert_eq!(edit["requires_tty"], true);
    assert_eq!(edit["effects"], "non_idempotent");
}
#[test]
fn help_and_completions_work_offline() {
    for args in [
        vec!["--help"],
        vec!["notes", "--help"],
        vec!["completions", "bash"],
    ] {
        let out = run(&args);
        assert!(out.status.success());
        assert!(!out.stdout.is_empty());
        assert!(out.stderr.is_empty());
    }
}

#[test]
fn tui_requires_a_terminal_but_demo_snapshot_is_offline_opaque_text() {
    for args in [vec!["tui"], vec!["tui", "--demo"]] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(12));
        assert!(output.stdout.is_empty());
        assert_eq!(parse(&output.stderr)["error"]["kind"], "tty_required");
    }
    let output = run(&[
        "tui",
        "--demo",
        "--snapshot",
        "--width",
        "60",
        "--height",
        "24",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 24);
    assert!(text.contains("simplenote"));
    assert!(text.contains("q quit"));
    assert!(!text.contains('\u{1b}'));
    let schema = parse(&run(&["schema"]).stdout);
    let tui = schema["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "tui")
        .unwrap();
    assert_eq!(tui["requires_tty"], true);
    assert_eq!(tui["output_kind"], "opaque");
}

#[test]
fn tui_snapshot_requires_demo_and_valid_geometry() {
    for args in [
        vec!["tui", "--snapshot"],
        vec!["tui", "--demo", "--width", "90"],
        vec!["tui", "--demo", "--snapshot", "--width", "0"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}
