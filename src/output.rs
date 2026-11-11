//! Human-facing command output. JSON and exact `notes cat` output bypass this layer.
use crate::{
    Error,
    cli::{AuthCommand, Command, NotesCommand, safe_terminal_content, safe_text},
};
use crossterm::style::{Color, Stylize};
use serde_json::Value;
use std::io::{self, IsTerminal, Write};
use time::OffsetDateTime;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy)]
pub struct Appearance {
    width: usize,
    color: bool,
}

impl Appearance {
    pub fn plain(width: usize) -> Self {
        Self {
            width: width.max(1),
            color: false,
        }
    }
    pub fn stdout() -> Self {
        Self::terminal(io::stdout().is_terminal())
    }
    pub fn stderr() -> Self {
        Self::terminal(io::stderr().is_terminal())
    }
    fn terminal(tty: bool) -> Self {
        let width = if tty {
            crossterm::terminal::size().map_or(80, |(w, _)| usize::from(w).max(1))
        } else {
            80
        };
        Self {
            width: width.min(100),
            color: tty
                && std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").as_deref() != Ok("dumb"),
        }
    }
}

#[derive(Clone, Copy)]
enum Ink {
    Body,
    Heading,
    Accent,
    Success,
    Warning,
}

struct Printer<'a> {
    appearance: Appearance,
    writer: &'a mut dyn Write,
}
impl Printer<'_> {
    fn line(&mut self, text: &str, ink: Ink) -> io::Result<()> {
        if self.appearance.color {
            match ink {
                Ink::Heading => writeln!(self.writer, "{}", text.bold()),
                Ink::Accent => writeln!(self.writer, "{}", text.with(Color::Blue)),
                Ink::Success => writeln!(self.writer, "{}", text.with(Color::Green).bold()),
                Ink::Warning => writeln!(self.writer, "{}", text.with(Color::Yellow).bold()),
                Ink::Body => writeln!(self.writer, "{text}"),
            }
        } else {
            writeln!(self.writer, "{text}")
        }
    }
    fn blank(&mut self) -> io::Result<()> {
        self.line("", Ink::Body)
    }
    fn prose(&mut self, text: &str, indent: usize, ink: Ink) -> io::Result<()> {
        let indent = indent.min(self.appearance.width.saturating_sub(1));
        let prefix = " ".repeat(indent);
        for line in wrap(text, self.appearance.width.saturating_sub(indent).max(1)) {
            self.line(&format!("{prefix}{line}"), ink)?;
        }
        Ok(())
    }
    fn field(&mut self, label: &str, value: &str) -> io::Result<()> {
        let text = format!("  {label}  {value}");
        if text.width() <= self.appearance.width
            || matches!(label, "Note ID" | "Directory" | "Recovery draft")
        {
            // Let the terminal soft-wrap copyable identifiers and paths.
            return self.line(&text, Ink::Body);
        }
        self.prose(&format!("{label}  {value}"), 2, Ink::Body)
    }
    fn body(&mut self, content: &str) -> io::Result<()> {
        // Preserve paragraphs, Markdown, code indentation, and literal backslashes.
        // `notes cat` remains the byte-for-byte content interface.
        for line in safe_terminal_content(content).replace('\t', "    ").lines() {
            self.line(line, Ink::Body)?;
        }
        Ok(())
    }
}

pub fn render(
    value: &Value,
    command: Option<&Command>,
    appearance: Appearance,
    writer: &mut impl Write,
) -> Result<(), Error> {
    let mut p = Printer { appearance, writer };
    if let Some(Command::Auth { command }) = command {
        return auth(value, command, &mut p).map_err(Into::into);
    }
    if let Some(items) = value["items"].as_array() {
        collection(value, items, command, &mut p)?;
    } else if value.get("exported").is_some() {
        let count = value["exported"].as_u64().unwrap_or_default();
        p.prose(
            &format!("Exported {}", counted(count, "note", "notes")),
            0,
            Ink::Success,
        )?;
        p.blank()?;
        p.field("Directory", &scalar(&value["directory"]))?;
        if value["includes_deleted"] == true {
            p.prose("Includes notes in Trash.", 2, Ink::Body)?;
        }
    } else if value["dry_run"] == true {
        p.prose("Preview · nothing written", 0, Ink::Warning)?;
        p.blank()?;
        if value["note"].is_object() {
            note(&value["note"], &mut p)?;
        } else {
            fields(value, &["id", "base_version"], &mut p)?;
            if let Some(patch) = value["patch"].as_object() {
                for (key, value) in patch {
                    if key == "content" {
                        p.blank()?;
                        p.prose("Proposed content", 0, Ink::Heading)?;
                        p.body(value.as_str().unwrap_or_default())?;
                    } else {
                        p.field(&label(key), &field_value(key, value))?;
                    }
                }
            }
        }
        recovery(value, &mut p)?;
    } else if value["no_op"] == true {
        p.prose("No changes needed.", 0, Ink::Heading)?;
        if value["note"].is_object() {
            p.blank()?;
            note(&value["note"], &mut p)?;
        }
        recovery(value, &mut p)?;
    } else if value.get("content").is_some() && value.get("id").is_some() {
        let result = match command {
            Some(Command::Notes {
                command: NotesCommand::Create { .. },
            }) => Some("Note created"),
            Some(Command::Notes {
                command: NotesCommand::Update { .. } | NotesCommand::Edit { .. },
            }) => Some("Note saved"),
            Some(Command::Notes {
                command: NotesCommand::Trash { .. },
            }) => Some("Note moved to Trash"),
            Some(Command::Notes {
                command: NotesCommand::Restore { .. },
            }) => Some("Note restored"),
            _ => None,
        };
        if let Some(result) = result {
            p.prose(result, 0, Ink::Success)?;
            p.blank()?;
        }
        note(value, &mut p)?;
        recovery(value, &mut p)?;
    } else {
        generic(value, &mut p)?;
    }
    Ok(())
}

fn auth(value: &Value, command: &AuthCommand, p: &mut Printer<'_>) -> io::Result<()> {
    match command {
        AuthCommand::Status if value["authenticated"] == true => {
            p.prose("Credentials available", 0, Ink::Heading)?;
            p.blank()?;
            p.field(
                "Source",
                if value["source"] == "environment" {
                    "SIMPLENOTE_TOKEN environment variable"
                } else {
                    "Saved login"
                },
            )?;
            p.prose(
                "Not verified with Simplenote. Run a notes command to check access.",
                2,
                Ink::Body,
            )
        }
        AuthCommand::Status => {
            p.prose("Not signed in", 0, Ink::Heading)?;
            p.blank()?;
            p.prose("Sign in with simplenote auth login.", 0, Ink::Body)
        }
        AuthCommand::Login { .. } if value.get("login_url").is_some() => {
            p.prose("Sign in to Simplenote", 0, Ink::Heading)?;
            p.blank()?;
            p.prose("Open this link in your browser:", 0, Ink::Body)?;
            p.line(&format!("  {}", scalar(&value["login_url"])), Ink::Accent)?;
            p.blank()?;
            p.prose(&scalar(&value["next_step"]), 0, Ink::Body)
        }
        AuthCommand::Login { .. } => {
            p.prose("Signed in to Simplenote", 0, Ink::Success)?;
            p.prose(
                "Browse your notes with simplenote notes list, or open simplenote tui.",
                0,
                Ink::Body,
            )
        }
        AuthCommand::Logout => {
            p.prose(
                if value["removed"] == true {
                    "Saved login removed"
                } else {
                    "No saved login to remove"
                },
                0,
                Ink::Heading,
            )?;
            if value["environment_token_present"] == true {
                p.blank()?;
                p.prose("SIMPLENOTE_TOKEN is still set. Unset it to stop using those credentials in this shell.", 0, Ink::Warning)?;
            }
            Ok(())
        }
    }
}

fn collection(
    value: &Value,
    items: &[Value],
    command: Option<&Command>,
    p: &mut Printer<'_>,
) -> io::Result<()> {
    let (heading, projected) = match command {
        Some(Command::Notes {
            command: NotesCommand::Search { query, filter },
        }) => (
            format!(
                "{} for “{}”",
                counted(items.len() as u64, "match", "matches"),
                safe_text(query)
            ),
            filter.page.fields.as_deref(),
        ),
        Some(Command::Notes {
            command: NotesCommand::History { id, page },
        }) => (
            format!(
                "{} · {}",
                counted(items.len() as u64, "version", "versions"),
                safe_text(id)
            ),
            page.fields.as_deref(),
        ),
        Some(Command::Notes {
            command: NotesCommand::List { filter },
        }) => (
            counted(items.len() as u64, "note", "notes"),
            filter.page.fields.as_deref(),
        ),
        _ => (counted(items.len() as u64, "note", "notes"), None),
    };
    p.prose(&heading, 0, Ink::Heading)?;
    if items.is_empty() {
        p.blank()?;
        let message = if value["next_cursor"].is_string() {
            "No matching notes on this page. More pages are available below."
        } else {
            match command {
                Some(Command::Notes {
                    command: NotesCommand::Search { .. },
                }) => "No matching notes. Try a shorter search or remove a tag filter.",
                Some(Command::Notes {
                    command: NotesCommand::History { .. },
                }) => "No versions to show.",
                Some(Command::Notes {
                    command: NotesCommand::List { filter },
                }) if filter.tag.is_some() => "No notes with this tag. Try removing --tag.",
                _ => "No notes to show.",
            }
        };
        p.prose(message, 0, Ink::Body)?;
        if message == "No notes to show." {
            p.line(
                "Create one: simplenote notes create --file <path>",
                Ink::Accent,
            )?;
        }
    }
    for item in items {
        p.blank()?;
        if let Some(projected) = projected {
            let mut keys = projected.iter().map(String::as_str).collect::<Vec<_>>();
            for key in ["id", "version", "available"] {
                if item.get(key).is_some() && !keys.contains(&key) {
                    keys.push(key);
                }
            }
            fields(item, &keys, p)?;
        } else if item["available"] == false {
            p.prose(
                &format!("Version {} · unavailable", scalar(&item["version"])),
                2,
                Ink::Heading,
            )?;
            if item.get("id").is_some() {
                p.field("Note", &scalar(&item["id"]))?;
            }
        } else if item.get("title").is_some() {
            let title = title(item);
            p.line(
                &format!("  {}", fit(&title, p.appearance.width.saturating_sub(2))),
                Ink::Heading,
            )?;
            let metadata = metadata(item, true);
            if !metadata.is_empty() {
                metadata_line(item, &metadata, 2, p)?;
            }
            if let Some(tags) = tags(item) {
                p.field("Tags", &tags)?;
            }
        } else {
            generic(item, p)?;
        }
    }
    if let Some(count) = value["skipped_irregular"].as_u64().filter(|n| *n > 0) {
        p.blank()?;
        p.prose(
            &format!("Skipped malformed records: {count} (preserved by export)."),
            0,
            Ink::Warning,
        )?;
    }
    if value["scan_limited"] == true {
        p.blank()?;
        p.prose(
            "Stopped at the scan limit. Continue with the cursor below to keep looking.",
            0,
            Ink::Warning,
        )?;
    }
    if let Some(cursor) = value["next_cursor"].as_str() {
        p.blank()?;
        p.prose("More results available", 0, Ink::Heading)?;
        p.line(&format!("Next cursor: {}", safe_text(cursor)), Ink::Body)?;
        if safe_text(cursor) == cursor
            && let Some(next) = continuation(command, cursor)
        {
            p.line(&format!("  {next}"), Ink::Accent)?;
        }
    }
    Ok(())
}

fn note(value: &Value, p: &mut Printer<'_>) -> io::Result<()> {
    p.prose(&title(value), 0, Ink::Heading)?;
    metadata_line(value, &metadata(value, false), 0, p)?;
    for key in ["modified", "created"] {
        if !value[key].is_null() {
            p.field(&label(key), &date(&value[key]))?;
        }
    }
    if let Some(tags) = tags(value) {
        p.field("Tags", &tags)?;
    }
    if value["markdown"] == true {
        p.field("Format", "Markdown")?;
    }
    if let Some(content) = value["content"].as_str() {
        // The first nonblank line supplies the title. Show it once, retaining
        // all remaining body lines, including blank paragraphs and code blocks.
        let mut lines = content.lines();
        let first = lines.find(|line| !line.trim().is_empty());
        if first.is_some_and(|line| line.trim() == value["title"].as_str().unwrap_or_default()) {
            let body = lines.collect::<Vec<_>>().join("\n");
            if !body.starts_with('\n') {
                p.blank()?;
            }
            p.body(&body)?;
        } else if content.trim().is_empty() {
            p.blank()?;
            p.prose("This note is empty.", 0, Ink::Body)?;
        } else {
            p.blank()?;
            p.body(content)?;
        }
    }
    Ok(())
}

fn metadata(value: &Value, modified: bool) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(id) = value["id"].as_str() {
        parts.push(safe_text(id));
    }
    if let Some(version) = value["version"].as_u64().filter(|n| *n > 0) {
        parts.push(format!("v{version}"));
    }
    if modified && !value["modified"].is_null() {
        parts.push(date(&value["modified"]));
    }
    if value["pinned"] == true {
        parts.push("Pinned".into());
    }
    if value["deleted"] == true {
        parts.push("In Trash".into());
    }
    parts
}

fn metadata_line(
    value: &Value,
    parts: &[String],
    indent: usize,
    p: &mut Printer<'_>,
) -> io::Result<()> {
    let width = p.appearance.width.saturating_sub(indent).max(1);
    let prefix = " ".repeat(indent);
    let mut line = String::new();
    for part in parts {
        if !line.is_empty() && line.width() + 3 + part.width() > width {
            p.line(&format!("{prefix}{line}"), Ink::Body)?;
            line.clear();
        }
        if !line.is_empty() {
            line.push_str(" · ");
        }
        if part.width() > width && value["id"].as_str().is_none_or(|id| safe_text(id) != *part) {
            p.prose(part, indent, Ink::Body)?;
        } else {
            // Keep IDs and dates intact; long IDs soft-wrap in the terminal.
            line.push_str(part);
        }
    }
    if !line.is_empty() {
        p.line(&format!("{prefix}{line}"), Ink::Body)?;
    }
    Ok(())
}

fn tags(value: &Value) -> Option<String> {
    let tags = value["tags"].as_array()?;
    if tags.is_empty() {
        None
    } else {
        Some(tags.iter().map(scalar).collect::<Vec<_>>().join(", "))
    }
}
fn title(value: &Value) -> String {
    let title = value["title"].as_str().unwrap_or_default().trim();
    if title.is_empty() {
        "Untitled note".into()
    } else {
        safe_text(title)
    }
}
fn counted(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}
fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => safe_text(text),
        Value::Null => "Not recorded".into(),
        Value::Bool(value) => if *value { "Yes" } else { "No" }.into(),
        Value::Array(values) => {
            if values.is_empty() {
                "None".into()
            } else {
                values.iter().map(scalar).collect::<Vec<_>>().join(", ")
            }
        }
        _ => safe_text(&value.to_string()),
    }
}
fn date(value: &Value) -> String {
    let seconds = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok());
    if let Some(seconds) = seconds.filter(|n| n.is_finite())
        && let Ok(date) = OffsetDateTime::from_unix_timestamp(seconds as i64)
    {
        return format!(
            "{:04}-{:02}-{:02} {:02}:{:02} UTC",
            date.year(),
            date.month() as u8,
            date.day(),
            date.hour(),
            date.minute()
        );
    }
    scalar(value)
}
fn label(key: &str) -> String {
    match key {
        "id" => "Note ID".into(),
        "base_version" => "Base version".into(),
        "modified" => "Updated".into(),
        "created" => "Created".into(),
        _ => {
            let mut label = key.replace('_', " ");
            if let Some(first) = label.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            label
        }
    }
}
fn field_value(key: &str, value: &Value) -> String {
    if matches!(key, "created" | "modified") {
        date(value)
    } else {
        scalar(value)
    }
}
fn fields(value: &Value, keys: &[&str], p: &mut Printer<'_>) -> io::Result<()> {
    for key in keys {
        if let Some(value) = value.get(key) {
            if *key == "content" {
                p.prose("Content", 2, Ink::Heading)?;
                p.body(value.as_str().unwrap_or_default())?;
            } else {
                p.field(&label(key), &field_value(key, value))?;
            }
        }
    }
    Ok(())
}
fn generic(value: &Value, p: &mut Printer<'_>) -> io::Result<()> {
    if let Some(object) = value.as_object() {
        fields(
            value,
            &object.keys().map(String::as_str).collect::<Vec<_>>(),
            p,
        )
    } else {
        p.prose(&scalar(value), 0, Ink::Body)
    }
}
fn recovery(value: &Value, p: &mut Printer<'_>) -> io::Result<()> {
    if let Some(path) = value["recovery_file"].as_str() {
        p.blank()?;
        p.prose("Draft retained", 0, Ink::Warning)?;
        p.field("Recovery draft", &safe_text(path))?;
        p.prose("Review this temporary draft before deleting it; copy it somewhere permanent if you need to keep it.", 0, Ink::Body)?;
    }
    Ok(())
}

fn continuation(command: Option<&Command>, cursor: &str) -> Option<String> {
    use crate::cli::Filter;
    fn filter_args(filter: &Filter, args: &mut Vec<String>) {
        if let Some(tag) = &filter.tag {
            args.extend(["--tag".into(), tag.clone()]);
        }
        if filter.include_deleted {
            args.push("--include-deleted".into());
        }
        page_args(&filter.page, args);
    }
    fn page_args(page: &crate::cli::Page, args: &mut Vec<String>) {
        if page.limit != 20 {
            args.extend(["--limit".into(), page.limit.to_string()]);
        }
        if let Some(fields) = &page.fields {
            args.extend(["--fields".into(), fields.join(",")]);
        }
    }
    let mut args = vec!["simplenote".into(), "notes".into()];
    match command? {
        Command::Notes {
            command: NotesCommand::List { filter },
        } => {
            args.push("list".into());
            filter_args(filter, &mut args);
        }
        Command::Notes {
            command: NotesCommand::Search { query, filter },
        } => {
            args.extend(["search".into(), query.clone()]);
            filter_args(filter, &mut args);
        }
        Command::Notes {
            command: NotesCommand::History { id, page },
        } => {
            args.extend(["history".into(), id.clone()]);
            page_args(page, &mut args);
        }
        _ => return None,
    }
    args.extend(["--cursor".into(), cursor.into()]);
    if args.iter().any(|arg| safe_text(arg) != *arg) {
        return None;
    }
    Some(
        args.iter()
            .map(|arg| shell_words::quote(&safe_text(arg)).into_owned())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

pub fn render_error(
    error: &Error,
    appearance: Appearance,
    writer: &mut impl Write,
) -> Result<(), Error> {
    let mut p = Printer { appearance, writer };
    if error.kind == "usage" {
        p.prose("Usage error", 0, Ink::Warning)?;
        let message = error.message.trim_end();
        p.body(message.strip_prefix("error: ").unwrap_or(message))?;
        return Ok(());
    }
    let heading = match error.kind {
        "auth" => "Sign-in required",
        "not_found" => "Note not found",
        "network" => "Connection failed",
        "rate_limited" => "Too many requests",
        "conflict" => "The note changed",
        "io" => "Local operation failed",
        "invalid_response" => "Unexpected service response",
        "remote" => "Service request failed",
        "confirmation_required" => "Confirmation required",
        "tty_required" => "A terminal is required",
        _ => "Something went wrong",
    };
    p.prose(heading, 0, Ink::Warning)?;
    p.prose(&safe_text(&error.message), 0, Ink::Body)?;
    p.blank()?;
    let envelope = error.envelope();
    let hint = if error
        .details
        .as_ref()
        .is_some_and(|details| details["outcome_unknown"] == true)
    {
        "The write may have succeeded. Read the note and review it before retrying."
    } else if error
        .details
        .as_ref()
        .is_some_and(|details| details["recovery_file"].is_string())
    {
        "Your draft has been retained. Read the latest note and review your draft before trying again."
    } else {
        envelope["error"]["hint"].as_str().unwrap_or_default()
    };
    p.prose(hint, 0, Ink::Body)?;
    if let Some(details) = &error.details {
        for (key, name) in [("note_id", "Note ID"), ("recovery_file", "Recovery draft")] {
            if let Some(value) = details[key].as_str() {
                p.field(name, &safe_text(value))?;
            }
        }
        if details["recovery_file"].is_string() {
            p.prose("Review the retained draft and the latest note before saving with notes update --file.", 0, Ink::Body)?;
        }
    }
    Ok(())
}

pub(crate) fn login_intro(writer: &mut impl Write) -> Result<(), Error> {
    let mut p = Printer {
        appearance: Appearance::stderr(),
        writer,
    };
    p.prose("Sign in to Simplenote", 0, Ink::Heading)?;
    p.blank()?;
    p.prose("Open this link in your browser:", 0, Ink::Body)?;
    p.line(&format!("  {}", crate::auth::LOGIN_URL), Ink::Accent)?;
    p.blank()?;
    p.prose(
        "After signing in, copy the email:CODE credential and paste it below.",
        0,
        Ink::Body,
    )?;
    p.blank()?;
    Ok(())
}

// Wrap prose without counting Unicode code points as terminal columns. Long
// words are split safely; content and copyable command lines retain their text.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.width() + 1 + word.width() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            for c in word.chars() {
                if !line.is_empty() && line.width() + c.width().unwrap_or(0) > width {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(c);
            }
        }
        lines.push(line);
    }
    lines
}
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    for c in text.chars() {
        if result.width() + c.width().unwrap_or(0) >= width {
            break;
        }
        result.push(c);
    }
    result.push('…');
    result
}
