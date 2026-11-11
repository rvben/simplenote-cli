use clap::Parser;
use serde_json::{Value, json};
use simplenote_cli::{
    Error,
    cli::Cli,
    output::{self, Appearance},
};

fn render(args: &[&str], value: &Value, width: usize) -> String {
    let cli = Cli::parse_from(std::iter::once("simplenote").chain(args.iter().copied()));
    let mut bytes = Vec::new();
    output::render(
        value,
        Some(&cli.command),
        Appearance::plain(width),
        &mut bytes,
    )
    .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains('\x1b'));
    text
}
fn note() -> Value {
    json!({"id":"abc-123", "version":12, "title":"Travel plans", "content":"Travel plans\n\nA quiet weekend.\n\n    keep_code_indented()\nLiteral \\n stays literal.\n", "tags":["travel","personal"], "pinned":true, "deleted":false, "markdown":true, "created": 1735689600, "modified":"1735779600"})
}

#[test]
fn note_details_preserve_body_structure_and_show_useful_metadata() {
    let text = render(&["notes", "get", "abc-123"], &note(), 80);
    assert_eq!(text.matches("Travel plans").count(), 1);
    assert!(text.contains("abc-123 · v12 · Pinned"));
    assert!(text.contains("Updated  2025-01-02 01:00 UTC"));
    assert!(text.contains("Created  2025-01-01 00:00 UTC"));
    assert!(text.contains("travel, personal"));
    assert!(
        text.contains("A quiet weekend.\n\n    keep_code_indented()\nLiteral \\n stays literal.")
    );
    assert!(!text.contains("false"));
    assert!(!text.contains("true"));
}

#[test]
fn collection_pagination_preserves_query_filters_and_projection() {
    let text = render(
        &[
            "notes",
            "search",
            "a 'quoted' query",
            "--tag",
            "work",
            "--include-deleted",
            "--limit",
            "3",
            "--fields",
            "title",
            "--cursor",
            "old",
        ],
        &json!({"items":[{"title":"Café\u{1b}[31m"}], "next_cursor":"opaque + 'next'", "has_more":true}),
        80,
    );
    assert!(text.contains("1 match"));
    assert!(text.contains("Café\\u{1b}[31m"));
    assert!(!text.contains("Note ID"));
    let command = text
        .lines()
        .find(|line| line.trim_start().starts_with("simplenote notes search"))
        .unwrap();
    let words = shell_words::split(command.trim()).unwrap();
    let next = Cli::try_parse_from(words).unwrap();
    if let simplenote_cli::cli::Command::Notes {
        command: simplenote_cli::cli::NotesCommand::Search { query, filter },
    } = next.command
    {
        assert_eq!(query, "a 'quoted' query");
        assert_eq!(filter.tag.as_deref(), Some("work"));
        assert!(filter.include_deleted);
        assert_eq!(filter.page.limit, 3);
        assert_eq!(filter.page.cursor.as_deref(), Some("opaque + 'next'"));
        assert_eq!(filter.page.fields.unwrap(), ["title"]);
    } else {
        panic!("Continuation changed the command");
    }
}

#[test]
fn empty_pages_do_not_claim_the_search_is_exhausted() {
    let text = render(
        &["notes", "search", "needle"],
        &json!({"items":[], "next_cursor":"next", "has_more":true, "scan_limited":true}),
        60,
    );
    assert!(text.contains("No matching notes on this page."));
    assert!(text.contains("More results available"));
    assert!(text.contains("scan limit"));
    let text = render(
        &["notes", "search", "needle"],
        &json!({"items":[], "next_cursor":null, "has_more":false}),
        60,
    );
    assert!(text.contains("Try a shorter search"));
    assert!(!text.contains("More results available"));
    let text = render(
        &["notes", "list"],
        &json!({"items":[], "next_cursor":null}),
        60,
    );
    assert!(text.contains("notes create --file <path>"));
}

#[test]
fn history_and_write_outcomes_are_explicit() {
    let text = render(
        &["notes", "history", "abc-123"],
        &json!({"items":[{"id":"abc-123","version":4,"available":false}],"next_cursor":null}),
        80,
    );
    assert!(text.contains("Version 4 · unavailable"));
    let text = render(
        &["notes", "create", "--file", "draft.txt", "--dry-run"],
        &json!({"dry_run":true,"note":note()}),
        80,
    );
    assert!(text.starts_with("Preview · nothing written\n"));
    assert!(!text.contains("Note created"));
    let text = render(
        &[
            "notes",
            "update",
            "abc-123",
            "--base-version",
            "12",
            "--file",
            "draft.txt",
        ],
        &note(),
        80,
    );
    assert!(text.starts_with("Note saved\n"));
    let text = render(
        &["notes", "get", "abc-123"],
        &json!({"no_op":true,"note":note(),"recovery_file":"/tmp/draft.md"}),
        80,
    );
    assert!(text.contains("No changes needed."));
    assert!(text.contains("Draft retained"));
    assert!(text.contains("/tmp/draft.md"));
}

#[test]
fn auth_status_does_not_claim_the_credentials_were_verified() {
    let text = render(
        &["auth", "status"],
        &json!({"authenticated":true,"source":"environment","verified":false}),
        80,
    );
    assert!(text.contains("Credentials available"));
    assert!(text.contains("SIMPLENOTE_TOKEN"));
    assert!(text.contains("Not verified with Simplenote."));
    assert!(!text.contains("Signed in"));
    let text = render(
        &["auth", "logout"],
        &json!({"removed":true,"environment_token_present":true}),
        80,
    );
    assert!(text.contains("Saved login removed"));
    assert!(text.contains("SIMPLENOTE_TOKEN is still set."));
    assert!(text.contains("Unset it"));
}

#[test]
fn recovery_errors_keep_the_note_id_and_draft_visible() {
    let error =
        Error::new("network", "Request failed.\u{1b}[31m").with_write_context("abc-123", "update");
    let mut error = error;
    error.details.as_mut().unwrap()["recovery_file"] = json!("/tmp/recovery/draft.md");
    let mut bytes = Vec::new();
    output::render_error(&error, Appearance::plain(60), &mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("Connection failed\n"));
    assert!(text.contains("write may have succeeded"));
    assert!(text.contains("Note ID  abc-123"));
    assert!(text.contains("Recovery draft  /tmp/recovery/draft.md"));
    assert!(!text.contains('\x1b'));
}

#[test]
fn long_unicode_titles_and_metadata_fit_narrow_terminals() {
    use unicode_width::UnicodeWidthStr;
    let mut note = note();
    note["title"] = json!("旅の計画とアイデア".repeat(10));
    note["tags"] = json!(["é".repeat(100)]);
    for width in [24, 40, 80, 100] {
        let text = render(
            &["notes", "list"],
            &json!({"items":[note],"next_cursor":null}),
            width,
        );
        for line in text.lines() {
            assert!(line.width() <= width, "Overflow at {width}: {line}");
        }
    }
}

#[test]
fn json_output_is_unchanged_and_note_controls_are_escaped_in_text() {
    let mut note = note();
    note["content"] = json!("Travel plans\n\nCafé\u{1b}]52;clipboard\u{7}\u{202e}\n");
    let text = render(&["notes", "get", "abc-123"], &note, 80);
    assert!(!text.contains('\x1b'));
    assert!(!text.contains('\u{202e}'));
    assert!(text.contains("Café"));
    let mut bytes = Vec::new();
    simplenote_cli::cli::render(&note, true, &mut bytes).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), note);
}

#[test]
fn long_ids_and_recovery_paths_remain_copyable() {
    let id = "note-".repeat(20);
    let path = format!("/tmp/{}/draft.md", "recovery-".repeat(15));
    let mut value = note();
    value["id"] = json!(id);
    value["recovery_file"] = json!(path);
    let text = render(&["notes", "get", &id], &value, 24);
    assert!(text.contains(&id));
    assert!(text.contains(&path));

    let mut error = Error::new("conflict", "The note changed while editing.");
    error.details = Some(json!({"note_id":id,"recovery_file":path}));
    let mut bytes = Vec::new();
    output::render_error(&error, Appearance::plain(80), &mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.contains("Your draft has been retained."));
    assert!(!text.contains("error.details"));
}
