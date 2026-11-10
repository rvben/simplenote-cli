//! Terminal-native note workspace. Background reads keep navigation responsive;
//! writes share the CLI's version checks and recovery behavior.
mod data;
mod markdown;
mod state;
#[cfg(test)]
mod tests;
mod view;

use crate::{
    Error,
    api::{Api, Http},
};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use data::{Fetch, Reply, Request, Worker};
use ratatui::{Terminal, backend::CrosstermBackend};
use state::{Action, App, Focus, Overlay};
use std::{
    io::{self, IsTerminal},
    sync::mpsc::TrySendError,
    time::Duration,
};

pub fn require_terminal() -> Result<(), Error> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() && io::stderr().is_terminal() {
        Ok(())
    } else {
        Err(Error::new(
            "tty_required",
            "The note workspace needs terminal stdin, stdout, and stderr. Use notes list/search/get for scripts, or tui --demo --snapshot to inspect a sample screen.",
        ))
    }
}
pub fn snapshot(width: u16, height: u16) -> Result<String, Error> {
    if !(40..=240).contains(&width) || !(10..=80).contains(&height) {
        return Err(Error::usage(
            "Snapshot size must be 40–240 columns and 10–80 rows.",
        ));
    }
    view::frame_text(&mut App::new(true), width, height)
}

struct TerminalGuard {
    active: bool,
}
fn restore_terminal() {
    let _ = execute!(
        io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture,
        Show,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
}
impl TerminalGuard {
    fn enter() -> Result<Self, Error> {
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            let owner = std::thread::current().id();
            std::panic::set_hook(Box::new(move |info| {
                if std::thread::current().id() == owner {
                    restore_terminal();
                }
                previous(info);
            }));
        });
        let mut guard = Self { active: false };
        guard.resume()?;
        Ok(guard)
    }
    fn resume(&mut self) -> Result<(), Error> {
        self.active = true;
        if let Err(error) = enable_raw_mode()
            .and_then(|()| execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, Hide))
        {
            restore_terminal();
            self.active = false;
            return Err(terminal_error("resume", error));
        }
        let _ = execute!(io::stdout(), EnableBracketedPaste);
        Ok(())
    }
    fn suspend(&mut self) {
        if self.active {
            restore_terminal();
            self.active = false;
        }
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.suspend();
    }
}

pub fn run(token: Option<String>) -> Result<(), Error> {
    require_terminal()?;
    let demo = token.is_none();
    let worker = token.as_ref().map(|token| Worker::start(token.clone()));
    let mut pending = None;
    let mut app = App::new(demo);
    let mut guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let theme = view::Theme::for_terminal();
    if !demo {
        fetch(&mut app, false, worker.as_ref(), &mut pending);
    }
    let result = (|| -> Result<(), Error> {
        loop {
            if let Some(worker) = &worker {
                loop {
                    match worker.replies.try_recv() {
                        Ok(reply) => {
                            if receive(&mut app, reply) {
                                fetch(&mut app, false, Some(worker), &mut pending);
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            return Err(Error::new(
                                "internal",
                                "The connection worker stopped. Quit and restart the workspace; no retry was attempted.",
                            ));
                        }
                    }
                }
                if let Some(request) = pending.take() {
                    dispatch(request, Some(worker), &mut pending);
                }
            }
            terminal
                .draw(|frame| view::draw(frame, &mut app, theme))
                .map_err(|e| terminal_error("draw", e))?;
            if !event::poll(Duration::from_millis(100)).map_err(|e| terminal_error("poll", e))? {
                app.tick = app.tick.wrapping_add(1);
                continue;
            }
            let action = match event::read().map_err(|e| terminal_error("read", e))? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                Event::Paste(text) => {
                    if let Some(search) = app.search.as_mut() {
                        for c in text.chars().filter(|c| !c.is_control()) {
                            if search.len() + c.len_utf8() > 512 {
                                break;
                            }
                            search.push(c);
                        }
                        app.selected = 0;
                    }
                    Action::None
                }
                Event::Mouse(mouse) if app.overlay.is_none() && app.search.is_none() => {
                    let point = ratatui::layout::Position::new(mouse.column, mouse.row);
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            if app.geometry.reader.contains(point) {
                                app.scroll = app.scroll.saturating_add(3).min(app.max_scroll);
                            } else {
                                app.move_selection(1);
                            }
                            Action::None
                        }
                        MouseEventKind::ScrollUp => {
                            if app.geometry.reader.contains(point) {
                                app.scroll = app.scroll.saturating_sub(3);
                            } else {
                                app.move_selection(-1);
                            }
                            Action::None
                        }
                        MouseEventKind::Down(MouseButton::Left)
                            if app.geometry.sidebar_list.contains(point) =>
                        {
                            let row = usize::from(mouse.row - app.geometry.sidebar_list.y)
                                + app.sidebar_state.offset();
                            app.sections()
                                .get(row)
                                .cloned()
                                .map_or(Action::None, |section| app.choose_section(section))
                        }
                        MouseEventKind::Down(MouseButton::Left)
                            if app.geometry.list.contains(point) =>
                        {
                            let row = usize::from(mouse.row - app.geometry.list.y) / 4
                                + app.list_state.offset();
                            if row < app.visible().len() {
                                app.selected = row;
                                app.scroll = 0;
                                app.focus = Focus::Index;
                            }
                            Action::None
                        }
                        MouseEventKind::Down(MouseButton::Left)
                            if app.geometry.reader.contains(point) =>
                        {
                            app.focus = Focus::Reader;
                            Action::None
                        }
                        _ => Action::None,
                    }
                }
                _ => Action::None,
            };
            match action {
                Action::Quit => return Ok(()),
                Action::None => {}
                Action::Fetch(append) => fetch(&mut app, append, worker.as_ref(), &mut pending),
                Action::Change { id, base, patch } => {
                    app.busy = true;
                    app.writing = Some(id.clone());
                    app.status = "Saving change…".into();
                    dispatch(
                        Request::Change { id, base, patch },
                        worker.as_ref(),
                        &mut pending,
                    );
                }
                Action::Pin { id, base, pinned } => {
                    app.busy = true;
                    app.writing = Some(id.clone());
                    app.status = "Saving pin…".into();
                    dispatch(
                        Request::Pin { id, base, pinned },
                        worker.as_ref(),
                        &mut pending,
                    );
                }
                action @ (Action::Edit(_) | Action::Create) => {
                    let creating = matches!(&action, Action::Create);
                    guard.suspend();
                    let result = (|| {
                        let http = Http::new()?;
                        let api = Api {
                            http: &http,
                            token: token.as_ref().expect("live action").clone(),
                        };
                        let editor = crate::edit::editor_from_environment()?;
                        eprintln!("Opening your editor. Close it to return to your notes.");
                        match action {
                            Action::Edit(id) => crate::edit::edit(&api, &id, false, editor),
                            _ => crate::edit::create(&api, editor),
                        }
                    })();
                    match &result {
                        Ok(value) => remember_recovery(&mut app, value),
                        Err(error) => {
                            if let Some(details) = &error.details {
                                remember_recovery(&mut app, details);
                            }
                        }
                    }
                    guard.resume()?;
                    redraw_after_editor(&mut terminal)
                        .map_err(|e| terminal_error("redraw after editor", e))?;
                    match result {
                        Ok(value) => {
                            remember_recovery(&mut app, &value);
                            if value.get("recovery_file").is_some() {
                                app.overlay = Some(Overlay::Message(format!(
                                    "The service returned different content. Your edited draft was retained:\n\n{}\n\nRead the saved note before retrying.",
                                    value["recovery_file"].as_str().unwrap_or_default()
                                )));
                            }
                            apply_saved(&mut app, value, creating);
                        }
                        Err(error) => show_error(&mut app, error),
                    }
                }
            }
        }
    })();
    drop(terminal);
    drop(guard);
    for path in &app.recovery {
        eprintln!("Draft retained: {}", crate::cli::safe_text(path));
    }
    result
}
fn dispatch(request: Request, worker: Option<&Worker>, pending: &mut Option<Request>) {
    if let Some(worker) = worker {
        match worker.requests.try_send(request) {
            Ok(()) => {}
            Err(TrySendError::Full(request)) => *pending = Some(request),
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}
fn fetch(app: &mut App, append: bool, worker: Option<&Worker>, pending: &mut Option<Request>) {
    if app.demo {
        app.set_notes(data::demo_notes(), false);
        return;
    }
    app.generation = app.generation.wrapping_add(1);
    app.fetching = Some(app.generation);
    app.busy = true;
    if !append {
        app.seen_cursors.clear();
        app.cursor = None;
    }
    app.status = if append {
        "Loading more notes…".into()
    } else if app.query.is_empty() {
        "Refreshing your notes…".into()
    } else {
        "Searching your account…".into()
    };
    dispatch(
        Request::Fetch(Fetch {
            generation: app.generation,
            section: app.section.clone(),
            query: app.query.clone(),
            cursor: if append { app.cursor.clone() } else { None },
        }),
        worker,
        pending,
    );
}
fn receive(app: &mut App, reply: Reply) -> bool {
    match reply {
        Reply::Page { fetch, result } => {
            if fetch.generation != app.generation {
                return false;
            }
            app.fetching = None;
            app.busy = false;
            match result {
                Ok(value) => {
                    app.connected = true;
                    app.collection_loaded = true;
                    let next = value["next_cursor"].as_str().map(str::to_owned);
                    if let Some(cursor) = &next
                        && (fetch.cursor.as_ref() == Some(cursor)
                            || !app.seen_cursors.insert(cursor.clone()))
                    {
                        app.cursor = None;
                        show_error(
                            app,
                            Error::new(
                                "invalid_response",
                                "The service repeated a page cursor. Refresh to start over.",
                            ),
                        );
                        return false;
                    }
                    app.cursor = next;
                    app.set_notes(
                        value["items"].as_array().cloned().unwrap_or_default(),
                        fetch.cursor.is_some(),
                    );
                    let skipped = value["skipped_irregular"].as_u64().unwrap_or_default();
                    app.status = format!(
                        "{} notes loaded{}{}",
                        app.visible().len(),
                        if app.cursor.is_some() {
                            " · m to load more"
                        } else {
                            " · up to date"
                        },
                        if skipped > 0 {
                            format!(" · {skipped} irregular records skipped")
                        } else {
                            String::new()
                        }
                    );
                }
                Err(error) => show_error(app, error),
            }
        }
        Reply::Written(result) => {
            app.busy = app.fetching.is_some();
            app.writing = None;
            if matches!(app.overlay, Some(Overlay::Saving(_))) {
                app.overlay = None;
            }
            match result {
                Ok(value) => {
                    apply_saved(app, value, false);
                }
                Err(error) => show_error(app, error),
            }
        }
    }
    false
}
fn remember_recovery(app: &mut App, value: &serde_json::Value) {
    if let Some(path) = value["recovery_file"].as_str()
        && !app.recovery.iter().any(|p| p == path)
    {
        app.recovery.push(path.into());
    }
}
fn show_error(app: &mut App, error: Error) {
    app.busy = app.fetching.is_some();
    if matches!(error.kind, "auth" | "network" | "remote") {
        app.connected = false;
    }
    app.overlay_scroll = 0;
    let mut message = error.message.clone();
    if let Some(details) = &error.details {
        remember_recovery(app, details);
        if let Some(path) = details["recovery_file"].as_str() {
            message.push_str(&format!("\n\nDraft: {path}"));
        }
        if details["outcome_unknown"] == true {
            if let Some(id) = details["note_id"].as_str() {
                message.push_str(&format!("\n\nNote ID: {id}"));
            }
            message.push_str("\n\nThe write may have succeeded. Refresh and review the note before trying again.");
        }
    }
    if error.kind == "auth" {
        message.push_str("\n\nQuit, then run simplenote-cli auth login.");
    } else if error.kind == "conflict" {
        message
            .push_str("\n\nPress r to refresh, then review the latest note before editing again.");
    } else {
        message.push_str("\n\nPress r to refresh. Nothing is retried automatically.");
    }
    app.status = format!("{} · press ? for keys", error.kind);
    app.overlay = Some(Overlay::Message(message));
}

fn apply_saved(app: &mut App, value: serde_json::Value, creating: bool) {
    let note = value.get("note").unwrap_or(&value);
    if note["id"].is_string() {
        if creating {
            if app.section != state::Section::All || !app.query.is_empty() {
                app.notes.clear();
                app.cursor = None;
                app.collection_loaded = false;
            }
            app.section = state::Section::All;
            app.query.clear();
            app.sidebar_selected = 0;
        }
        app.set_notes(vec![note.clone()], true);
        if creating {
            app.selected = app
                .visible()
                .iter()
                .position(|i| app.notes[*i]["id"] == note["id"])
                .unwrap_or(0);
            app.focus = Focus::Reader;
            app.reader_visible = true;
        }
    }
    app.status = if value["no_op"] == true {
        "Unchanged · nothing written".into()
    } else if creating {
        "Created · r to refresh the collection".into()
    } else {
        "Saved to Simplenote · r to refresh other notes".into()
    };
}

fn terminal_error(operation: &str, error: io::Error) -> Error {
    Error::new(
        "io",
        format!("Terminal {operation} failed ({:?}).", error.kind()),
    )
}

fn redraw_after_editor<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
) -> Result<(), B::Error> {
    // Fullscreen redraws do not need to preserve the editor's cursor. Terminal::clear
    // queries its position, which can time out on terminals that do not answer DSR.
    let area = terminal.size()?.into();
    terminal.resize(area)
}
