use mockito::{Matcher, Server};
use serde_json::{Value, json};
use simplenote_cli::{
    Error,
    api::{Api, Http},
    edit::edit,
};
use std::{path::PathBuf, process::Command};

// Run this ignored test in a child process as a portable fake editor. The draft
// filename consumes --skip, so libtest still runs exactly this fixture.
#[test]
#[ignore]
fn editor_fixture() {
    let path = PathBuf::from(std::env::args_os().last().unwrap());
    assert!(std::env::var_os("SIMPLENOTE_TOKEN").is_none());
    std::fs::write(
        std::env::var_os("EDITOR_CAPTURE").unwrap(),
        path.to_str().unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
    match std::env::var("EDITOR_MODE").unwrap().as_str() {
        "unchanged" => {}
        "changed" => std::fs::write(&path, "edited\n").unwrap(),
        #[cfg(unix)]
        "interrupt_parent" => {
            assert_eq!(unsafe { libc::kill(libc::getppid(), libc::SIGINT) }, 0);
            std::fs::write(&path, "edited\n").unwrap();
        }
        #[cfg(unix)]
        "interrupt_child" => {
            unsafe {
                libc::raise(libc::SIGINT);
            }
            std::fs::write(&path, "inherited ignored interrupt").unwrap();
            panic!("Editor must receive SIGINT normally");
        }
        "replace" => {
            let replacement = path.with_extension("replacement");
            std::fs::write(&replacement, "edited\n").unwrap();
            std::fs::remove_file(&path).unwrap();
            std::fs::rename(replacement, &path).unwrap();
        }
        "failed" => {
            std::fs::write(&path, "unsaved\n").unwrap();
            std::process::exit(7);
        }
        "blank" => std::fs::write(&path, " \n\t").unwrap(),
        "invalid" => std::fs::write(&path, [0xff]).unwrap(),
        "large" => std::fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).unwrap(),
        "removed" => std::fs::remove_file(&path).unwrap(),
        #[cfg(unix)]
        "symlink" => {
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
        }
        #[cfg(unix)]
        "hardlink" => std::fs::hard_link(&path, path.with_extension("linked")).unwrap(),
        other => panic!("Unknown fixture mode: {other}"),
    }
}

fn note(content: &str) -> Value {
    json!({"content":content,"tags":[],"systemTags":[],"deleted":false})
}
fn exercise(
    mode: &str,
    dry_run: bool,
    stale: bool,
    status: usize,
    returned: &str,
) -> (Result<Value, Error>, PathBuf) {
    exercise_original(mode, dry_run, stale, status, returned, "original\n")
}
fn exercise_original(
    mode: &str,
    dry_run: bool,
    stale: bool,
    status: usize,
    returned: &str,
    original: &str,
) -> (Result<Value, Error>, PathBuf) {
    let mut server = Server::new();
    let changed = matches!(mode, "changed" | "replace" | "interrupt_parent");
    let first = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "4")
        .with_body(note(original).to_string())
        .expect(1)
        .create();
    let second = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", if stale { "5" } else { "4" })
        .with_body(note(original).to_string())
        .expect(usize::from(changed))
        .create();
    let write = server
        .mock("POST", "/1/test/note/i/abc/v/4")
        .match_query(Matcher::Any)
        .match_body(Matcher::PartialJson(json!({"content":"edited\n"})))
        .with_status(status)
        .with_header("x-simperium-version", "5")
        .with_body(note(returned).to_string())
        .expect(usize::from(changed && !dry_run && !stale))
        .create();
    let http = Http::with_urls(
        &format!("{}/1/test/", server.url()),
        &format!("{}/", server.url()),
    )
    .unwrap();
    let capture = tempfile::tempdir().unwrap();
    let record = capture.path().join("editor-path");
    let mut editor = Command::new(std::env::current_exe().unwrap());
    editor
        .args([
            "--exact",
            "editor_fixture",
            "--ignored",
            "--nocapture",
            "--skip",
        ])
        .env("EDITOR_CAPTURE", &record)
        .env("EDITOR_MODE", mode)
        .env("SIMPLENOTE_TOKEN", "fixture-token-must-not-reach-editor");
    let result = edit(
        &Api {
            http: &http,
            token: "fixture".into(),
        },
        "abc",
        dry_run,
        editor,
    );
    let path = PathBuf::from(std::fs::read_to_string(record).unwrap());
    first.assert();
    second.assert();
    write.assert();
    (result, path)
}
fn retained(value: &Value, path: &std::path::Path, expected: &str) {
    assert_eq!(value["recovery_file"], path.to_str().unwrap());
    assert_eq!(value["base_version"], 4);
    assert_eq!(std::fs::read_to_string(path).unwrap(), expected);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

// Isolate signal behavior from the outer test runner and parallel tests.
#[cfg(unix)]
#[test]
#[ignore]
fn interrupt_driver() {
    fn handler() -> usize {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            assert_eq!(
                libc::sigaction(libc::SIGINT, std::ptr::null(), &mut action),
                0
            );
            action.sa_sigaction
        }
    }
    let original_handler = handler();
    let (result, path) = exercise("interrupt_parent", false, false, 200, "edited\n");
    assert_eq!(result.unwrap()["content"], "edited\n");
    assert!(!path.exists());
    assert_eq!(handler(), original_handler);
    let (result, path) = exercise("interrupt_child", false, false, 200, "edited\n");
    let error = result.unwrap_err();
    assert_eq!(error.kind, "io");
    retained(&error.details.unwrap(), &path, "original\n");
    assert_eq!(handler(), original_handler);
}

#[cfg(unix)]
#[test]
fn interrupts_reach_editor_and_parent_restores_its_handler() {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "interrupt_driver", "--ignored", "--nocapture"])
        .status()
        .unwrap();
    assert!(status.success());
}
#[test]
fn unchanged_content_skips_network_write_and_removes_draft() {
    let (result, path) = exercise("unchanged", false, false, 200, "edited\n");
    assert_eq!(result.unwrap()["no_op"], true);
    assert!(!path.parent().unwrap().exists());
}
#[test]
fn unchanged_legacy_blank_note_is_a_no_op() {
    let (result, path) = exercise_original("unchanged", false, false, 200, "edited\n", "\n");
    assert_eq!(result.unwrap()["no_op"], true);
    assert!(!path.parent().unwrap().exists());
}
#[test]
fn editor_update_uses_captured_version_and_cleans_up_on_success() {
    for mode in ["changed", "replace"] {
        let (result, path) = exercise(mode, false, false, 200, "edited\n");
        assert_eq!(result.unwrap()["content"], "edited\n");
        assert!(!path.parent().unwrap().exists());
    }
}
#[test]
fn dry_run_retains_the_edited_draft_without_posting() {
    let (result, path) = exercise("changed", true, false, 200, "edited\n");
    let result = result.unwrap();
    assert_eq!(result["dry_run"], true);
    retained(&result, &path, "edited\n");
}
#[test]
fn stale_version_retains_draft_without_posting() {
    let (result, path) = exercise("changed", false, true, 200, "edited\n");
    let error = result.unwrap_err();
    assert_eq!(error.kind, "conflict");
    retained(error.details.as_ref().unwrap(), &path, "edited\n");
}
#[test]
fn uncertain_write_preserves_both_draft_and_write_context() {
    let (result, path) = exercise("changed", false, false, 503, "edited\n");
    let error = result.unwrap_err();
    assert_eq!(error.kind, "remote");
    assert_eq!(error.details.as_ref().unwrap()["outcome_unknown"], true);
    retained(error.details.as_ref().unwrap(), &path, "edited\n");
}
#[test]
fn merged_server_result_keeps_the_submitted_draft() {
    let (result, path) = exercise("changed", false, false, 200, "merged\n");
    let result = result.unwrap();
    assert_eq!(result["content"], "merged\n");
    retained(&result, &path, "edited\n");
}
#[test]
fn failed_editor_retains_its_work_without_posting() {
    let (result, path) = exercise("failed", false, false, 200, "edited\n");
    let error = result.unwrap_err();
    assert_eq!(error.kind, "io");
    retained(error.details.as_ref().unwrap(), &path, "unsaved\n");
}
#[test]
fn invalid_drafts_are_kept_without_contacting_the_write_endpoint() {
    for mode in ["blank", "invalid", "large", "removed"] {
        let (result, path) = exercise(mode, false, false, 200, "edited\n");
        let error = result.unwrap_err();
        assert_eq!(
            error.details.as_ref().unwrap()["recovery_file"],
            path.to_str().unwrap()
        );
        assert!(path.parent().unwrap().exists());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
#[test]
#[cfg(unix)]
fn editor_symlink_is_not_followed() {
    for mode in ["symlink", "hardlink"] {
        let (result, path) = exercise(mode, false, false, 200, "edited\n");
        assert_eq!(result.unwrap_err().kind, "io");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}

#[test]
fn missing_editor_keeps_the_original_and_a_recovery_path() {
    let mut server = Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "4")
        .with_body(note("original\n").to_string())
        .expect(1)
        .create();
    let post = server.mock("POST", Matcher::Any).expect(0).create();
    let http = Http::with_urls(
        &format!("{}/1/test/", server.url()),
        &format!("{}/", server.url()),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let error = edit(
        &Api {
            http: &http,
            token: "fixture".into(),
        },
        "abc",
        false,
        Command::new(directory.path().join("does-not-exist")),
    )
    .unwrap_err();
    assert_eq!(error.kind, "io");
    let path = PathBuf::from(
        error.details.as_ref().unwrap()["recovery_file"]
            .as_str()
            .unwrap(),
    );
    retained(error.details.as_ref().unwrap(), &path, "original\n");
    read.assert();
    post.assert();
}

#[test]
fn trashed_note_is_refused_before_launching_an_editor() {
    let mut server = Server::new();
    let mut raw = note("trashed\n");
    raw["deleted"] = json!(true);
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "4")
        .with_body(raw.to_string())
        .expect(1)
        .create();
    let post = server.mock("POST", Matcher::Any).expect(0).create();
    let http = Http::with_urls(
        &format!("{}/1/test/", server.url()),
        &format!("{}/", server.url()),
    )
    .unwrap();
    let error = edit(
        &Api {
            http: &http,
            token: "fixture".into(),
        },
        "abc",
        false,
        Command::new("must-not-run"),
    )
    .unwrap_err();
    assert_eq!(error.kind, "usage");
    assert!(error.details.is_none());
    read.assert();
    post.assert();
}
