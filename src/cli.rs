use crate::{
    Error,
    api::{Api, Http},
    auth,
};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    name = "simplenote-cli",
    version,
    about = "Simplenote for humans, scripts, and agents",
    subcommand_required = true
)]
pub struct Cli {
    /// Output format. Auto uses readable text in terminals and JSON when piped.
    #[arg(long, short = 'o', value_enum, default_value = "auto", global = true)]
    pub output: Output,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Clone, Copy, ValueEnum)]
pub enum Output {
    Auto,
    Json,
    Text,
}
impl Output {
    pub fn json(self) -> bool {
        matches!(self, Self::Json) || matches!(self, Self::Auto) && !io::stdout().is_terminal()
    }
}
#[derive(Subcommand)]
pub enum Command {
    /// Print the machine-readable clispec contract.
    Schema,
    /// Generate a shell completion script.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Open the interactive note workspace (requires a terminal).
    Tui {
        /// Explore sample notes offline without credentials or network access.
        #[arg(long)]
        demo: bool,
        /// Print a sample frame without entering the terminal. Requires --demo.
        #[arg(long, requires = "demo")]
        snapshot: bool,
        /// Width of the sample frame (40–240 columns).
        #[arg(long, requires = "snapshot", default_value_t = 132, value_parser = clap::value_parser!(u16).range(40..=240))]
        width: u16,
        /// Height of the sample frame (10–80 rows).
        #[arg(long, requires = "snapshot", default_value_t = 38, value_parser = clap::value_parser!(u16).range(10..=80))]
        height: u16,
    },
    /// Manage local authentication.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Read and manage notes.
    Notes {
        #[command(subcommand)]
        command: NotesCommand,
    },
    /// Export every note, including trash, to private JSON files. Never overwrite existing files.
    Export {
        #[arg(long)]
        directory: PathBuf,
    },
}
#[derive(Subcommand)]
pub enum AuthCommand {
    /// Sign in through your browser, then paste the one-time credential.
    Login {
        /// Read email:CODE from stdin. Otherwise prompt only on a terminal.
        #[arg(long)]
        code_stdin: bool,
    },
    /// Report local credential availability without contacting the service or revealing secrets.
    Status,
    /// Remove this CLI's stored credentials (does not revoke tokens or clear environment variables).
    Logout,
}
#[derive(Args)]
pub struct Page {
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u16).range(1..=100))]
    pub limit: u16,
    #[arg(long)]
    pub cursor: Option<String>,
    /// Comma-separated output fields. Content is omitted from lists by default.
    #[arg(long, value_delimiter = ',')]
    pub fields: Option<Vec<String>>,
}
#[derive(Args)]
pub struct Filter {
    #[command(flatten)]
    pub page: Page,
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub include_deleted: bool,
}
#[derive(Args)]
pub struct Content {
    /// UTF-8 file, or '-' for stdin (at most 1 MiB).
    #[arg(long)]
    pub file: PathBuf,
}
#[derive(Args)]
pub struct Change {
    pub id: String,
    /// Version last read. Rejects an already stale edit; concurrent writes may be merged by the server.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub base_version: u64,
    /// Read and validate, but do not write to the service.
    #[arg(long)]
    pub dry_run: bool,
}
#[derive(Subcommand)]
pub enum NotesCommand {
    /// List notes in server index order with cursor pagination.
    List {
        #[command(flatten)]
        filter: Filter,
    },
    /// Case-insensitive substring search across note content.
    Search {
        query: String,
        #[command(flatten)]
        filter: Filter,
    },
    /// Get the full note and its version.
    Get {
        id: String,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        version: Option<u64>,
    },
    /// Write note content; exact when piped, escaped on a terminal unless --raw.
    Cat {
        id: String,
        #[arg(long)]
        raw: bool,
    },
    /// Create a Markdown note. Repeating creates another note; never retried automatically.
    Create {
        #[command(flatten)]
        content: Content,
        #[arg(long)]
        tag: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Replace content against a known version. Returns the server's potentially merged result.
    Update {
        #[command(flatten)]
        change: Change,
        #[command(flatten)]
        content: Content,
    },
    /// Edit in $VISUAL or $EDITOR (requires a terminal). Keep drafts if saving fails.
    Edit {
        id: String,
        /// Open the editor and preview the update without writing; keep the edited draft.
        #[arg(long)]
        dry_run: bool,
    },
    /// Move a note to trash (recoverable).
    Trash {
        #[command(flatten)]
        change: Change,
        #[arg(long)]
        yes: bool,
    },
    /// Restore a note from trash.
    Restore {
        #[command(flatten)]
        change: Change,
    },
    /// Read previous versions; unavailable versions are marked explicitly.
    History {
        id: String,
        #[command(flatten)]
        page: Page,
    },
}

pub fn execute(cli: &Cli, http: &Http, dir: &Path) -> Result<Option<Value>, Error> {
    match &cli.command {
        Command::Schema => return Ok(Some(crate::schema::contract())),
        Command::Completions { shell } => {
            clap_complete::generate(
                *shell,
                &mut Cli::command(),
                "simplenote-cli",
                &mut io::stdout(),
            );
            return Ok(None);
        }
        Command::Tui {
            demo,
            snapshot,
            width,
            height,
        } => {
            if *snapshot {
                io::stdout().write_all(crate::tui::snapshot(*width, *height)?.as_bytes())?;
            } else {
                crate::tui::require_terminal()?;
                let token = if *demo { None } else { Some(auth::load(dir)?) };
                crate::tui::run(token)?;
            }
            return Ok(None);
        }
        Command::Auth { command } => {
            return Ok(Some(match command {
                AuthCommand::Status => {
                    let env = std::env::var("SIMPLENOTE_TOKEN").is_ok_and(|s| !s.trim().is_empty());
                    let stored = auth::read(dir).is_ok();
                    json!({"authenticated": env || stored, "source": if env {"environment"} else if stored {"file"} else {"none"}, "verified": false})
                }
                AuthCommand::Logout => {
                    json!({"removed": auth::logout(dir)?, "environment_token_present": std::env::var_os("SIMPLENOTE_TOKEN").is_some()})
                }
                AuthCommand::Login { code_stdin } => {
                    if !code_stdin && !io::stdin().is_terminal() {
                        return Ok(Some(
                            json!({"login_url": auth::LOGIN_URL, "next_step": "Open the URL, then pipe email:CODE to simplenote-cli auth login --code-stdin. Codes expire after five minutes."}),
                        ));
                    }
                    let credential = if *code_stdin {
                        read_limited(io::stdin().lock(), 4096)?
                    } else {
                        crate::output::login_intro(&mut io::stderr().lock())?;
                        let Some(credential) = auth::read_credential()? else {
                            return Ok(None);
                        };
                        credential
                    };
                    let token = auth::redeem(http, &credential)?;
                    auth::save(dir, token)?;
                    json!({"authenticated": true, "source": "file"})
                }
            }));
        }
        _ => {}
    }
    let mut editor = if matches!(
        cli.command,
        Command::Notes {
            command: NotesCommand::Edit { .. }
        }
    ) {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(Error::new(
                "tty_required",
                "notes edit requires terminal input and stderr; use notes update --file for scripts.",
            ));
        }
        Some(crate::edit::editor_from_environment()?)
    } else {
        None
    };
    let api = Api {
        http,
        token: auth::load(dir)?,
    };
    let result = match &cli.command {
        Command::Notes { command } => match command {
            NotesCommand::List { filter } => list(&api, filter, None)?,
            NotesCommand::Search { query, filter } => {
                if query.trim().is_empty() {
                    return Err(Error::usage("Search query cannot be empty."));
                }
                list(&api, filter, Some(query))?
            }
            NotesCommand::Get { id, version } => api.get(id, *version)?,
            NotesCommand::Cat { id, raw } => {
                let note = api.get(id, None)?;
                let content = note["content"].as_str().unwrap_or_default();
                let escaped;
                let content = if io::stdout().is_terminal() && !raw {
                    escaped = safe_terminal_content(content);
                    &escaped
                } else {
                    content
                };
                io::stdout().lock().write_all(content.as_bytes())?;
                return Ok(None);
            }
            NotesCommand::Create {
                content,
                tag,
                dry_run,
            } => api.create(&read_content(content)?, tag, *dry_run)?,
            NotesCommand::Update { change, content } => api.change(
                &change.id,
                change.base_version,
                json!({"content": read_content(content)?}),
                change.dry_run,
            )?,
            NotesCommand::Edit { id, dry_run } => {
                crate::edit::edit(&api, id, *dry_run, editor.take().expect("validated editor"))?
            }
            NotesCommand::Trash { change, yes } => {
                if !yes && !change.dry_run {
                    return Err(Error::new(
                        "confirmation_required",
                        "Trashing a note requires --yes.",
                    ));
                }
                api.change(
                    &change.id,
                    change.base_version,
                    json!({"deleted": true}),
                    change.dry_run,
                )?
            }
            NotesCommand::Restore { change } => api.change(
                &change.id,
                change.base_version,
                json!({"deleted": false}),
                change.dry_run,
            )?,
            NotesCommand::History { id, page } => {
                validate_fields(page.fields.as_deref())?;
                let cursor = page
                    .cursor
                    .as_ref()
                    .map(|v| {
                        v.parse::<u64>().map_err(|_| {
                            Error::usage("History cursor must be a positive version number.")
                        })
                    })
                    .transpose()?;
                let mut result = api.history(id, usize::from(page.limit), cursor)?;
                project(&mut result, page.fields.as_deref(), true);
                result
            }
        },
        Command::Export { directory } => export(&api, directory)?,
        _ => unreachable!(),
    };
    Ok(Some(result))
}
fn list(api: &Api<'_>, filter: &Filter, query: Option<&str>) -> Result<Value, Error> {
    validate_fields(filter.page.fields.as_deref())?;
    let mut result = api.list(
        usize::from(filter.page.limit),
        filter.page.cursor.as_deref(),
        filter.tag.as_deref(),
        query,
        filter.include_deleted,
    )?;
    project(&mut result, filter.page.fields.as_deref(), false);
    Ok(result)
}
const FIELDS: &[&str] = &[
    "id",
    "version",
    "title",
    "content",
    "tags",
    "pinned",
    "markdown",
    "deleted",
    "created",
    "modified",
    "available",
];
fn validate_fields(fields: Option<&[String]>) -> Result<(), Error> {
    if fields.is_some_and(|fs| fs.iter().any(|f| !FIELDS.contains(&f.as_str()))) {
        return Err(Error::usage(format!(
            "Unknown field; choose from {}.",
            FIELDS.join(", ")
        )));
    }
    Ok(())
}
fn project(value: &mut Value, fields: Option<&[String]>, history: bool) {
    if let Some(items) = value["items"].as_array_mut() {
        for item in items {
            if let Some(object) = item.as_object_mut() {
                object.retain(|k, _| {
                    fields.map_or(k != "content", |fs| fs.contains(k))
                        || history && ["id", "version", "available"].contains(&k.as_str())
                });
            }
        }
    }
}
pub(crate) fn read_limited(reader: impl Read, limit: u64) -> Result<String, Error> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::usage(format!("Input exceeds {limit} bytes.")));
    }
    String::from_utf8(bytes).map_err(|_| Error::usage("Input must be valid UTF-8."))
}
fn read_content(content: &Content) -> Result<String, Error> {
    if content.file == Path::new("-") {
        if io::stdin().is_terminal() {
            return Err(Error::usage(
                "Pipe content into --file -, or specify a file.",
            ));
        }
        read_note_content(io::stdin().lock())
    } else {
        read_note_content(std::fs::File::open(&content.file)?)
    }
}
pub(crate) fn read_note_content(reader: impl Read) -> Result<String, Error> {
    let text = read_limited(reader, 1024 * 1024)?;
    if text.trim().is_empty() {
        return Err(Error::usage("Note content cannot be blank."));
    }
    Ok(text)
}
fn export(api: &Api<'_>, directory: &Path) -> Result<Value, Error> {
    // A new directory gives each export an independent snapshot and prevents accidental replacement.
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(directory)
            .map_err(export_directory_error)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(directory).map_err(export_directory_error)?;
    let mut cursor = None;
    let mut seen = std::collections::HashSet::new();
    let mut count = 0;
    loop {
        let page = api.index_page(100, cursor.as_deref())?;
        for entry in page["index"].as_array().expect("validated index") {
            let id = entry["id"].as_str().ok_or_else(|| {
                Error::new(
                    "invalid_response",
                    "Missing export ID; partial export retained.",
                )
            })?;
            let version = entry["v"].as_u64().filter(|v| *v > 0).ok_or_else(|| {
                Error::new(
                    "invalid_response",
                    "Missing export version; partial export retained.",
                )
            })?;
            if !entry["d"].is_object() {
                return Err(Error::new(
                    "invalid_response",
                    "Missing export data; partial export retained.",
                ));
            }
            // Preserve the source even when a legacy record cannot be normalized.
            let mut note = match crate::api::normalize(id, &entry["d"], version) {
                Ok(note) => note,
                Err(_) => json!({"id": id, "version": version, "normalized": false}),
            };
            note["raw"] = entry["d"].clone();
            let mut file = tempfile::NamedTempFile::new_in(directory)?;
            serde_json::to_writer_pretty(&mut file, &note)
                .map_err(|_| Error::new("io", "Cannot serialize export."))?;
            file.flush()?;
            file.as_file().sync_all()?;
            // Hash the exact ID to avoid case-folding collisions and reserved filenames.
            let name = format!("note-{:x}.json", Sha256::digest(id.as_bytes()));
            file.persist_noclobber(directory.join(name)).map_err(|_| {
                Error::new("io", "Cannot save export file; partial export retained.")
            })?;
            count += 1;
        }
        cursor = page["mark"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        match &cursor {
            None => break,
            Some(c) if !seen.insert(c.clone()) => {
                return Err(Error::new(
                    "invalid_response",
                    "Export cursor repeated; partial export retained.",
                ));
            }
            _ => {}
        }
    }
    Ok(
        json!({"exported": count, "directory": directory.to_string_lossy(), "includes_deleted": true}),
    )
}

pub fn render(value: &Value, json: bool, writer: &mut impl Write) -> Result<(), Error> {
    if json {
        writeln!(writer, "{value}")?;
    } else {
        crate::output::render(value, None, crate::output::Appearance::plain(80), writer)?;
    }
    Ok(())
}
pub(crate) fn safe_text(text: &str) -> String {
    text.chars()
        .map(|c| {
            if unsafe_terminal_character(c) {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn export_directory_error(error: io::Error) -> Error {
    if error.kind() == io::ErrorKind::AlreadyExists {
        Error::new(
            "io",
            "Export directory already exists; choose a new directory.",
        )
    } else {
        error.into()
    }
}
fn unsafe_terminal_character(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
}
pub fn safe_terminal_content(content: &str) -> String {
    content
        .chars()
        .map(|c| {
            if unsafe_terminal_character(c) && c != '\n' && c != '\t' {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}
