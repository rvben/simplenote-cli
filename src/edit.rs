//! Interactive editing with a private, recoverable draft and versioned updates.
use crate::{
    Error,
    api::Api,
    cli::{read_limited, read_note_content},
};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

pub fn editor_from_environment() -> Result<Command, Error> {
    editor_from_settings(|name| std::env::var(name))
}
fn editor_from_settings(
    get: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> Result<Command, Error> {
    for name in ["VISUAL", "EDITOR"] {
        match get(name) {
            Ok(value) if !value.trim().is_empty() => return editor_command(&value),
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(Error::usage(format!("{name} must be valid UTF-8.")));
            }
            _ => {}
        }
    }
    if cfg!(windows) {
        Err(Error::usage(
            "Set VISUAL or EDITOR to an editor that waits, for example code.cmd --wait.",
        ))
    } else {
        editor_command("vi")
    }
}

fn editor_command(value: &str) -> Result<Command, Error> {
    let words = shell_words::split(value)
        .map_err(|_| Error::usage("Editor command has unmatched quotes."))?;
    let Some(program) = words.first().filter(|s| !s.is_empty()) else {
        return Err(Error::usage("Editor command must name an executable."));
    };
    let mut command = Command::new(program);
    command.args(&words[1..]);
    Ok(command)
}

/// The CLI checks terminal availability before calling this entry point.
/// An explicit command also lets protocol tests exercise editors without a TTY.
pub fn edit(api: &Api<'_>, id: &str, dry_run: bool, mut editor: Command) -> Result<Value, Error> {
    let note = api.get(id, None)?;
    if note["deleted"] == true {
        return Err(Error::usage(
            "Restore the trashed note before editing its content.",
        ));
    }
    let base = note["version"].as_u64().expect("validated version");
    let original = note["content"].as_str().expect("validated content");
    let mut builder = tempfile::Builder::new();
    builder.prefix("simplenote-edit-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let directory = builder.tempdir()?;
    let path = directory.path().join("note.md");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(original.as_bytes())?;
    file.sync_all()?;
    drop(file); // Editors, especially on Windows, must be able to replace the file.
    let result = (|| {
        editor
            .arg(&path)
            .env_remove("SIMPLENOTE_TOKEN")
            .stdin(Stdio::inherit())
            .stdout(Stdio::from(std::io::stderr()))
            .stderr(Stdio::inherit());
        let status = wait_for_editor(&mut editor)?;
        if !status.success() {
            return Err(Error::new(
                "io",
                "Editor exited unsuccessfully; nothing was written to the service.",
            ));
        }
        let content = read_limited(open_draft(&path)?, 1024 * 1024)?;
        if content == original {
            return Ok((json!({"no_op": true, "note": note}), false));
        }
        // Unchanged legacy blank notes are still a no-op; changed content must
        // satisfy the same input rules as notes update.
        let content = read_note_content(content.as_bytes())?;
        let response = api.change(id, base, json!({"content": content}), dry_run)?;
        let returned_content = response.get("note").unwrap_or(&response)["content"].as_str();
        let keep = dry_run || returned_content != Some(content.as_str());
        Ok((response, keep))
    })();
    match result {
        Ok((mut response, true)) => {
            let _ = directory.keep();
            response["recovery_file"] = json!(path.to_string_lossy());
            response["base_version"] = json!(base);
            Ok(response)
        }
        Ok((response, false)) => Ok(response),
        Err(mut error) => {
            let _ = directory.keep();
            let details = error.details.get_or_insert_with(|| json!({}));
            details["note_id"] = json!(id);
            details["base_version"] = json!(base);
            details["recovery_file"] = json!(path.to_string_lossy());
            details["recovery_directory"] =
                json!(path.parent().expect("draft directory").to_string_lossy());
            let display =
                crate::cli::safe_text(&path.parent().expect("draft directory").to_string_lossy());
            error
                .message
                .push_str(&format!(" Editing files retained in {display}."));
            Err(error)
        }
    }
}

fn wait_for_editor(editor: &mut Command) -> Result<std::process::ExitStatus, Error> {
    // Signal handlers are process-wide, including for library callers.
    static WAIT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _lock = WAIT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Ignored dispositions survive exec, so restore them in the child.
        unsafe {
            editor.pre_exec(|| {
                for signal in [libc::SIGINT, libc::SIGQUIT] {
                    if libc::signal(signal, libc::SIG_DFL) == libc::SIG_ERR {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
    }
    let _interrupts = interrupts::Guard::install()?;
    let mut child = editor
        .spawn()
        .map_err(|_| Error::new("io", "Could not start the editor; check VISUAL or EDITOR."))?;
    loop {
        match child.wait() {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => return result.map_err(Error::from),
        }
    }
}

/// Compose a new note for the TUI, preserving the draft on every failed save.
pub(crate) fn create(api: &Api<'_>, mut editor: Command) -> Result<Value, Error> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("simplenote-edit-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let directory = builder.tempdir()?;
    let path = directory.path().join("note.md");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    drop(options.open(&path)?);
    let result = (|| {
        editor
            .arg(&path)
            .env_remove("SIMPLENOTE_TOKEN")
            .stdin(Stdio::inherit())
            .stdout(Stdio::from(std::io::stderr()))
            .stderr(Stdio::inherit());
        if !wait_for_editor(&mut editor)?.success() {
            return Err(Error::new(
                "io",
                "Editor exited unsuccessfully; nothing was written to the service.",
            ));
        }
        let content = read_limited(open_draft(&path)?, 1024 * 1024)?;
        if content.is_empty() {
            return Ok((json!({"no_op":true}), false));
        }
        let content = read_note_content(content.as_bytes())?;
        let response = api.create(&content, &[], false)?;
        let keep =
            response.get("note").unwrap_or(&response)["content"].as_str() != Some(content.as_str());
        Ok((response, keep))
    })();
    match result {
        Ok((mut response, true)) => {
            let _ = directory.keep();
            response["recovery_file"] = json!(path.to_string_lossy());
            Ok(response)
        }
        Ok((response, false)) => Ok(response),
        Err(mut error) => {
            let _ = directory.keep();
            let details = error.details.get_or_insert_with(|| json!({}));
            details["recovery_file"] = json!(path.to_string_lossy());
            details["recovery_directory"] =
                json!(path.parent().expect("draft directory").to_string_lossy());
            error.message.push_str(&format!(
                " Editing files retained in {}.",
                crate::cli::safe_text(&path.parent().expect("draft directory").to_string_lossy())
            ));
            Err(error)
        }
    }
}

#[cfg(unix)]
mod interrupts {
    pub struct Guard {
        interrupt: libc::sigaction,
        quit: libc::sigaction,
    }
    impl Guard {
        pub fn install() -> std::io::Result<Self> {
            // Each previous action is initialized by a successful sigaction.
            unsafe {
                let mut ignore: libc::sigaction = std::mem::zeroed();
                ignore.sa_sigaction = libc::SIG_IGN;
                libc::sigemptyset(&mut ignore.sa_mask);
                let mut guard = Self {
                    interrupt: std::mem::zeroed(),
                    quit: std::mem::zeroed(),
                };
                if libc::sigaction(libc::SIGINT, &ignore, &mut guard.interrupt) != 0 {
                    let error = std::io::Error::last_os_error();
                    std::mem::forget(guard);
                    return Err(error);
                }
                if libc::sigaction(libc::SIGQUIT, &ignore, &mut guard.quit) != 0 {
                    let error = std::io::Error::last_os_error();
                    libc::sigaction(libc::SIGINT, &guard.interrupt, std::ptr::null_mut());
                    // Do not run Drop with an uninitialized previous quit action.
                    std::mem::forget(guard);
                    return Err(error);
                }
                Ok(guard)
            }
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                libc::sigaction(libc::SIGINT, &self.interrupt, std::ptr::null_mut());
                libc::sigaction(libc::SIGQUIT, &self.quit, std::ptr::null_mut());
            }
        }
    }
}

#[cfg(windows)]
mod interrupts {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    pub struct Guard;
    unsafe extern "system" fn handler(event: u32) -> windows_sys::core::BOOL {
        // Registered handlers are not inherited by the editor process.
        i32::from(event == 0 || event == 1)
    }
    impl Guard {
        pub fn install() -> std::io::Result<Self> {
            if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self)
        }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                SetConsoleCtrlHandler(Some(handler), 0);
            }
        }
    }
}

fn open_draft(path: &Path) -> Result<File, Error> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err(Error::new("io", "The edited draft must be a regular file."));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::new("io", "The edited draft must be a regular file."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if file.metadata()?.nlink() != 1 {
            return Err(Error::new(
                "io",
                "The edited draft must not have hard links.",
            ));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore]
    fn new_note_editor_fixture() {
        let path = std::path::PathBuf::from(std::env::args_os().last().unwrap());
        assert!(std::env::var_os("SIMPLENOTE_TOKEN").is_none());
        std::fs::write(
            std::env::var_os("DRAFT_CAPTURE").unwrap(),
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
        match std::env::var("DRAFT_MODE").unwrap().as_str() {
            "cancel" => {}
            "failed" => {
                std::fs::write(path, "unfinished").unwrap();
                std::process::exit(7);
            }
            _ => std::fs::write(path, "New note\n").unwrap(),
        }
    }
    #[test]
    fn new_notes_cancel_save_and_retain_drafts_on_failed_or_merged_writes() {
        for (mode, status, returned) in [
            ("cancel", 200, "New note\n"),
            ("save", 200, "New note\n"),
            ("failed", 200, "New note\n"),
            ("save", 503, ""),
            ("save", 200, "Merged note\n"),
        ] {
            let mut server = mockito::Server::new();
            let write = server
                .mock(
                    "POST",
                    mockito::Matcher::Regex("^/1/test/note/i/[a-f0-9-]+$".into()),
                )
                .match_query(mockito::Matcher::Any)
                .match_body(mockito::Matcher::PartialJson(
                    json!({"content":"New note\n"}),
                ))
                .with_status(status)
                .with_header("x-simperium-version", "1")
                .with_body(json!({"content":returned}).to_string())
                .expect(usize::from(mode == "save"))
                .create();
            let http =
                crate::api::Http::with_urls(&format!("{}/1/test/", server.url()), &server.url())
                    .unwrap();
            let capture = tempfile::tempdir().unwrap();
            let record = capture.path().join("path");
            let mut editor = Command::new(std::env::current_exe().unwrap());
            editor
                .args([
                    "--exact",
                    "edit::tests::new_note_editor_fixture",
                    "--ignored",
                    "--nocapture",
                    "--skip",
                ])
                .env("DRAFT_MODE", mode)
                .env("DRAFT_CAPTURE", &record)
                .env("SIMPLENOTE_TOKEN", "fixture-secret");
            let result = create(
                &Api {
                    http: &http,
                    token: "fixture".into(),
                },
                editor,
            );
            let path = std::path::PathBuf::from(std::fs::read_to_string(record).unwrap());
            write.assert();
            if mode == "failed" || status == 503 || returned == "Merged note\n" {
                let details = match result {
                    Ok(response) => response,
                    Err(error) => {
                        if status == 503 {
                            assert_eq!(error.details.as_ref().unwrap()["outcome_unknown"], true);
                        }
                        error.details.unwrap()
                    }
                };
                assert_eq!(details["recovery_file"], path.to_str().unwrap());
                assert_eq!(
                    std::fs::read_to_string(&path).unwrap(),
                    if mode == "failed" {
                        "unfinished"
                    } else {
                        "New note\n"
                    }
                );
                std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
            } else {
                let response = result.unwrap();
                if mode == "cancel" {
                    assert_eq!(response["no_op"], true);
                } else {
                    assert_eq!(response["content"], "New note\n");
                }
                assert!(!path.parent().unwrap().exists());
            }
        }
    }
    #[test]
    fn editor_arguments_are_quoted_without_running_a_shell() {
        let command = editor_command("'editor with spaces' --wait 'literal;$(echo nope)'").unwrap();
        assert_eq!(command.get_program(), "editor with spaces");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--wait", "literal;$(echo nope)"]
        );
        assert!(editor_command("'unclosed").is_err());
        assert!(editor_command("''").is_err());
    }
    #[test]
    fn visual_precedes_editor_and_blank_settings_fall_back() {
        for (visual, editor, expected) in [
            (Some("visual --wait"), Some("editor"), "visual"),
            (Some("  "), Some("editor"), "editor"),
            (None, Some("editor"), "editor"),
        ] {
            let command = editor_from_settings(|name| {
                (if name == "VISUAL" { visual } else { editor })
                    .map(str::to_owned)
                    .ok_or(std::env::VarError::NotPresent)
            })
            .unwrap();
            assert_eq!(command.get_program(), expected);
        }
        let default = editor_from_settings(|_| Err(std::env::VarError::NotPresent));
        if cfg!(windows) {
            assert!(default.is_err());
        } else {
            assert_eq!(default.unwrap().get_program(), "vi");
        }
    }
}
