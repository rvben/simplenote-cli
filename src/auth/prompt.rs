use crate::{Error, cli::safe_text};
use crossterm::{
    cursor::MoveToColumn,
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{self, Clear, ClearType},
};
use std::io::{self, IsTerminal, Write};
use unicode_width::UnicodeWidthChar;

const LIMIT: usize = 4096;

// Deliberately no Debug implementation: this state contains the login secret.
#[derive(Default)]
struct Input {
    value: String,
    revealed: bool,
    notice: Option<&'static str>,
}

enum Action {
    Continue,
    Submit,
    Cancel,
}

impl Input {
    fn append(&mut self, text: &str) {
        self.notice = None;
        if text.chars().any(char::is_control) {
            self.notice = Some("Paste a single email:CODE line");
        } else if self.value.len() + text.len() > LIMIT {
            self.notice = Some("Input too long (maximum 4096 bytes); Ctrl+U clears");
        } else {
            self.value.push_str(text);
        }
    }

    fn handle(&mut self, event: Event) -> Action {
        match event {
            Event::Paste(text) => self.append(text.trim()),
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                self.notice = None;
                let control = key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT);
                match key.code {
                    KeyCode::Esc => return Action::Cancel,
                    KeyCode::Char('c' | 'd') if control => {
                        return Action::Cancel;
                    }
                    KeyCode::Char('r') if control => {
                        if key.kind == KeyEventKind::Press {
                            self.revealed = !self.revealed;
                        }
                    }
                    KeyCode::Char('u') if control => self.value.clear(),
                    KeyCode::Backspace => {
                        self.value.pop();
                    }
                    KeyCode::Enter => {
                        if super::credential_parts(&self.value).is_ok() {
                            return Action::Submit;
                        }
                        self.notice = Some("Expected email:CODE; edit or Ctrl+U to clear");
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                            || key
                                .modifiers
                                .contains(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.append(c.encode_utf8(&mut [0; 4]));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Action::Continue
    }

    fn line(&self, columns: u16) -> String {
        let label = self.notice.unwrap_or(if self.revealed {
            "Paste email:CODE (visible)"
        } else {
            "Paste email:CODE"
        });
        let prefix = format!("{label}: ");
        let display = if self.revealed {
            safe_text(&self.value)
        } else {
            "*".repeat(self.value.chars().count())
        };
        // Keep the entire editable line inside the viewport, without wrapping
        // secrets onto previous terminal rows. Never query cursor position.
        let width = usize::from(columns.saturating_sub(1));
        let prefix: String = prefix.chars().take(width).collect();
        let available = width.saturating_sub(prefix.len());
        let mut used = 0;
        let tail: String = display
            .chars()
            .rev()
            .take_while(|c| {
                used += c.width().unwrap_or(0);
                used <= available
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{prefix}{tail}")
    }
}

struct PromptGuard;
impl PromptGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(io::stderr(), EnableBracketedPaste)?;
        Ok(guard)
    }
}
impl Drop for PromptGuard {
    fn drop(&mut self) {
        // Clear any revealed text on submit, cancel, I/O error, or unwind.
        let _ = execute!(
            io::stderr(),
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            DisableBracketedPaste
        );
        let _ = terminal::disable_raw_mode();
    }
}

pub(crate) fn read_credential() -> Result<Option<String>, Error> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(Error::new(
            "tty_required",
            "Interactive login needs terminal stdin and stderr. Use auth login --code-stdin for scripts.",
        ));
    }
    read_prompt().map_err(|_| {
        Error::new(
            "io",
            "Cannot read the login prompt. Check your terminal, or use auth login --code-stdin.",
        )
    })
}

fn read_prompt() -> io::Result<Option<String>> {
    writeln!(
        io::stderr(),
        "Enter to sign in · Ctrl+R show/hide · Ctrl+U clear · Esc cancel"
    )?;
    let guard = PromptGuard::enter()?;
    let mut input = Input::default();
    let result = loop {
        let columns = terminal::size().map_or(80, |(columns, _)| columns);
        let mut stderr = io::stderr().lock();
        execute!(stderr, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        write!(stderr, "{}", input.line(columns))?;
        stderr.flush()?;
        drop(stderr);
        match input.handle(event::read()?) {
            Action::Continue => {}
            Action::Submit => break Some(input.value),
            Action::Cancel => break None,
        }
    };
    drop(guard);
    writeln!(
        io::stderr(),
        "{}",
        if result.is_some() {
            "Credential received."
        } else {
            "Login cancelled."
        }
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn masking_reveal_edit_clear_and_submit() {
        let mut input = Input::default();
        input.handle(Event::Paste("probe@example.invalid:SECRET\r\n".into()));
        assert_eq!(
            input.line(80),
            "Paste email:CODE: ****************************"
        );
        input.handle(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(input.line(80).ends_with("probe@example.invalid:SECRET"));
        input.handle(key(KeyCode::Backspace, KeyModifiers::NONE));
        input.handle(key(KeyCode::Char('X'), KeyModifiers::SHIFT));
        assert!(input.value.ends_with("SECREX"));
        assert!(matches!(
            input.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Submit
        ));
        input.handle(key(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(!input.line(80).contains("SECREX"));
        input.handle(key(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(input.value.is_empty());
        assert!(matches!(
            input.handle(key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::Continue
        ));
    }

    #[test]
    fn invalid_paste_is_atomic_and_does_not_submit_or_echo_controls() {
        let mut input = Input::default();
        for paste in ["x\ny", "x\x1b[31my", &"x".repeat(LIMIT + 1)] {
            assert!(matches!(
                input.handle(Event::Paste(paste.into())),
                Action::Continue
            ));
            assert!(input.value.is_empty());
            assert!(input.notice.is_some());
        }
        input.handle(Event::Paste("x".repeat(LIMIT)));
        input.handle(key(KeyCode::Char('é'), KeyModifiers::NONE));
        assert_eq!(input.value.len(), LIMIT);
        for code in [KeyCode::Esc, KeyCode::Char('c'), KeyCode::Char('d')] {
            assert!(matches!(
                input.handle(key(code, KeyModifiers::CONTROL)),
                Action::Cancel
            ));
        }
    }

    #[test]
    fn narrow_and_unicode_input_stays_on_one_row() {
        use unicode_width::UnicodeWidthStr;
        let mut input = Input::default();
        input.handle(Event::Paste("界".repeat(100)));
        for revealed in [false, true] {
            input.revealed = revealed;
            for columns in [0, 1, 10, 20, 40, 80] {
                assert!(input.line(columns).width() < usize::from(columns.max(1)));
            }
        }
    }

    #[test]
    fn international_keyboard_characters_are_not_shortcuts() {
        let mut input = Input::default();
        for c in ['@', 'c', 'r', 'u'] {
            assert!(matches!(
                input.handle(key(
                    KeyCode::Char(c),
                    KeyModifiers::CONTROL | KeyModifiers::ALT
                )),
                Action::Continue
            ));
        }
        assert_eq!(input.value, "@cru");
        assert!(!input.revealed);
        let release = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        ));
        input.handle(release);
        assert!(!input.revealed);
    }
}
