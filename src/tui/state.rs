use super::data::demo_notes;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Section {
    #[default]
    All,
    Pinned,
    Trash,
    Tag(String),
}
impl Section {
    pub fn name(&self) -> &str {
        match self {
            Self::All => "All notes",
            Self::Pinned => "Pinned",
            Self::Trash => "Trash",
            Self::Tag(tag) => tag,
        }
    }
    pub fn accepts(&self, note: &Value) -> bool {
        if matches!(self, Self::Trash) {
            return note["deleted"] == true;
        }
        note["deleted"] != true
            && match self {
                Self::Pinned => note["pinned"] == true,
                Self::Tag(tag) => note["tags"].as_array().is_some_and(|tags| {
                    tags.iter().any(|t| {
                        t.as_str()
                            .is_some_and(|s| s.to_lowercase() == tag.to_lowercase())
                    })
                }),
                _ => true,
            }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Focus {
    Sidebar,
    #[default]
    Index,
    Reader,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Overlay {
    Help,
    Tags,
    ConfirmTrash {
        id: String,
        base: u64,
        title: String,
    },
    Message(String),
    Saving(String),
}
#[derive(Clone, Debug)]
pub enum Action {
    None,
    Quit,
    Fetch(bool),
    Edit(String),
    Create,
    Pin { id: String, base: u64, pinned: bool },
    Change { id: String, base: u64, patch: Value },
}

pub struct App {
    pub demo: bool,
    pub notes: Vec<Value>,
    pub selected: usize,
    pub section: Section,
    pub tags: BTreeSet<String>,
    pub sidebar_selected: usize,
    pub focus: Focus,
    pub query: String,
    pub search: Option<String>,
    pub overlay: Option<Overlay>,
    pub overlay_scroll: u16,
    pub overlay_max_scroll: u16,
    pub scroll: u16,
    pub max_scroll: u16,
    pub reader_visible: bool,
    pub busy: bool,
    pub writing: Option<String>,
    pub connected: bool,
    pub collection_loaded: bool,
    pub generation: u64,
    pub fetching: Option<u64>,
    pub cursor: Option<String>,
    pub seen_cursors: HashSet<String>,
    pub status: String,
    pub recovery: Vec<String>,
    pub tick: usize,
    pub list_state: ratatui::widgets::ListState,
    pub sidebar_state: ratatui::widgets::ListState,
    pub geometry: super::view::Geometry,
}
impl App {
    pub fn new(demo: bool) -> Self {
        let mut app = Self {
            demo,
            notes: vec![],
            selected: 0,
            section: Section::All,
            tags: BTreeSet::new(),
            sidebar_selected: 0,
            focus: Focus::Index,
            query: String::new(),
            search: None,
            overlay: None,
            overlay_scroll: 0,
            overlay_max_scroll: 0,
            scroll: 0,
            max_scroll: 0,
            reader_visible: false,
            busy: false,
            writing: None,
            connected: false,
            collection_loaded: demo,
            generation: 0,
            fetching: None,
            cursor: None,
            seen_cursors: HashSet::new(),
            status: if demo {
                "Sample notes · changes disabled".into()
            } else {
                "Connecting to Simplenote…".into()
            },
            recovery: vec![],
            tick: 0,
            list_state: Default::default(),
            sidebar_state: Default::default(),
            geometry: Default::default(),
        };
        if demo {
            app.set_notes(demo_notes(), false);
        }
        app
    }
    pub fn sections(&self) -> Vec<Section> {
        [Section::All, Section::Pinned, Section::Trash]
            .into_iter()
            .chain(self.tags.iter().cloned().map(Section::Tag))
            .collect()
    }
    pub fn visible(&self) -> Vec<usize> {
        let query = self.search.as_ref().unwrap_or(&self.query).to_lowercase();
        let mut indices: Vec<_> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                self.section.accepts(n)
                    && (query.is_empty()
                        || n["content"]
                            .as_str()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&query))
            })
            .map(|(i, _)| i)
            .collect();
        indices.sort_by(|a, b| {
            self.notes[*b]["pinned"]
                .as_bool()
                .cmp(&self.notes[*a]["pinned"].as_bool())
                .then_with(|| modified(&self.notes[*b]).total_cmp(&modified(&self.notes[*a])))
                .then_with(|| {
                    self.notes[*a]["id"]
                        .as_str()
                        .cmp(&self.notes[*b]["id"].as_str())
                })
        });
        indices
    }
    pub fn current(&self) -> Option<&Value> {
        self.visible().get(self.selected).map(|i| &self.notes[*i])
    }
    pub fn set_notes(&mut self, notes: Vec<Value>, append: bool) {
        let id = self
            .current()
            .and_then(|n| n["id"].as_str())
            .map(str::to_owned);
        if !append {
            self.notes.clear();
        }
        for note in notes {
            if let Some(tags) = note["tags"].as_array() {
                self.tags
                    .extend(tags.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            if let Some(existing) = self.notes.iter_mut().find(|n| n["id"] == note["id"]) {
                *existing = note;
            } else {
                self.notes.push(note);
            }
        }
        self.selected = id
            .and_then(|id| {
                self.visible()
                    .iter()
                    .position(|i| self.notes[*i]["id"] == id)
            })
            .unwrap_or(0);
        self.scroll = 0;
    }
    pub fn move_selection(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.visible().len().saturating_sub(1));
        self.scroll = 0;
    }
    pub fn choose_section(&mut self, section: Section) -> Action {
        self.section = section;
        self.selected = 0;
        self.scroll = 0;
        self.reader_visible = false;
        self.sidebar_selected = self
            .sections()
            .iter()
            .position(|s| s == &self.section)
            .unwrap_or(0);
        self.focus = Focus::Index;
        Action::Fetch(false)
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return self.quit();
        }
        if let Some(input) = self.search.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    self.search = None;
                    self.selected = 0;
                }
                KeyCode::Enter => {
                    self.query = self.search.take().unwrap().trim().to_owned();
                    self.selected = 0;
                    return Action::Fetch(false);
                }
                KeyCode::Backspace => {
                    input.pop();
                    self.selected = 0;
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear();
                    self.selected = 0;
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && input.len() < 512 =>
                {
                    input.push(c);
                    self.selected = 0;
                }
                _ => {}
            }
            self.scroll = 0;
            return Action::None;
        }
        if let Some(overlay) = self.overlay.clone() {
            match overlay {
                Overlay::ConfirmTrash { id, base, .. } => {
                    self.overlay = None;
                    if key.code == KeyCode::Char('y') {
                        return Action::Change {
                            id,
                            base,
                            patch: json!({"deleted":true}),
                        };
                    }
                }
                Overlay::Tags => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.overlay = None,
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.sidebar_selected =
                            (self.sidebar_selected + 1).min(self.sections().len().saturating_sub(1))
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.sidebar_selected = self.sidebar_selected.saturating_sub(1)
                    }
                    KeyCode::Enter => {
                        self.overlay = None;
                        return self.choose_section(self.sections()[self.sidebar_selected].clone());
                    }
                    _ => {}
                },
                _ => {
                    if matches!(
                        key.code,
                        KeyCode::Down | KeyCode::Char('j') | KeyCode::PageDown
                    ) {
                        self.overlay_scroll = self
                            .overlay_scroll
                            .saturating_add(if key.code == KeyCode::PageDown { 8 } else { 1 })
                            .min(self.overlay_max_scroll);
                    } else if matches!(key.code, KeyCode::Up | KeyCode::Char('k') | KeyCode::PageUp)
                    {
                        self.overlay_scroll = self
                            .overlay_scroll
                            .saturating_sub(if key.code == KeyCode::PageUp { 8 } else { 1 });
                    }
                    if matches!(
                        key.code,
                        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?')
                    ) {
                        self.overlay = None;
                    }
                }
            }
            return Action::None;
        }
        self.overlay_scroll = 0;
        match key.code {
            KeyCode::Char('q') => return self.quit(),
            KeyCode::Char('?') | KeyCode::F(1) => self.overlay = Some(Overlay::Help),
            KeyCode::Char('/') => {
                self.search = Some(self.query.clone());
                self.selected = 0;
            }
            KeyCode::Char('t') => self.overlay = Some(Overlay::Tags),
            KeyCode::Char('1') => return self.choose_section(Section::All),
            KeyCode::Char('2') => return self.choose_section(Section::Pinned),
            KeyCode::Char('3') => return self.choose_section(Section::Trash),
            KeyCode::Char('r') if self.idle() => return Action::Fetch(false),
            KeyCode::Char('m') if self.idle() && self.cursor.is_some() => {
                return Action::Fetch(true);
            }
            KeyCode::Char('e') if self.idle() => {
                if self.demo {
                    self.read_only();
                } else if let Some(note) = self.current() {
                    if note["deleted"] == true {
                        self.status = "Restore this note before editing it.".into();
                    } else {
                        return Action::Edit(note["id"].as_str().unwrap().into());
                    }
                }
            }
            KeyCode::Char('n') if self.idle() => {
                if self.demo {
                    self.read_only();
                } else {
                    return Action::Create;
                }
            }
            KeyCode::Char('p') if self.idle() => {
                if self.demo {
                    self.read_only();
                } else if let Some(note) = self.current() {
                    if note["deleted"] == true {
                        self.status = "Restore this note before pinning it.".into();
                    } else {
                        return Action::Pin {
                            id: note["id"].as_str().unwrap().into(),
                            base: note["version"].as_u64().unwrap(),
                            pinned: note["pinned"] != true,
                        };
                    }
                }
            }
            KeyCode::Char('d') if self.idle() && self.current().is_some() => {
                if self.demo {
                    self.read_only();
                } else if self.current().is_some_and(|n| n["deleted"] == true) {
                    self.status = "This note is already in Trash. Press u to restore.".into();
                } else {
                    let note = self.current().unwrap();
                    self.overlay = Some(Overlay::ConfirmTrash {
                        id: note["id"].as_str().unwrap().into(),
                        base: note["version"].as_u64().unwrap(),
                        title: super::view::title(note),
                    });
                }
            }
            KeyCode::Char('u')
                if self.idle() && self.current().is_some_and(|n| n["deleted"] == true) =>
            {
                if self.demo {
                    self.read_only();
                } else {
                    return self.change(json!({"deleted":false}));
                }
            }
            KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
                if self.focus == Focus::Reader {
                    self.focus = Focus::Index;
                    self.reader_visible = false;
                } else if !self.query.is_empty() {
                    self.query.clear();
                    return Action::Fetch(false);
                } else if self.geometry.sidebar.width > 0 {
                    self.focus = Focus::Sidebar;
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Sidebar => Focus::Index,
                    Focus::Index => Focus::Reader,
                    Focus::Reader => {
                        if self.geometry.sidebar.width > 0 {
                            Focus::Sidebar
                        } else {
                            Focus::Index
                        }
                    }
                };
                self.reader_visible = self.focus == Focus::Reader;
            }
            KeyCode::Right | KeyCode::Enter | KeyCode::Char('l') => {
                if self.focus == Focus::Sidebar {
                    return self.choose_section(self.sections()[self.sidebar_selected].clone());
                }
                self.focus = Focus::Reader;
                self.reader_visible = true;
            }
            KeyCode::Down | KeyCode::Char('j') => match self.focus {
                Focus::Sidebar => {
                    self.sidebar_selected =
                        (self.sidebar_selected + 1).min(self.sections().len().saturating_sub(1))
                }
                Focus::Index => self.move_selection(1),
                Focus::Reader => self.scroll = self.scroll.saturating_add(1).min(self.max_scroll),
            },
            KeyCode::Up | KeyCode::Char('k') => match self.focus {
                Focus::Sidebar => self.sidebar_selected = self.sidebar_selected.saturating_sub(1),
                Focus::Index => self.move_selection(-1),
                Focus::Reader => self.scroll = self.scroll.saturating_sub(1),
            },
            KeyCode::PageDown | KeyCode::Char(' ') => {
                if self.focus == Focus::Reader {
                    self.scroll = self
                        .scroll
                        .saturating_add(self.geometry.reader.height.saturating_sub(8))
                        .min(self.max_scroll);
                } else {
                    self.move_selection(8);
                }
            }
            KeyCode::PageUp => {
                if self.focus == Focus::Reader {
                    self.scroll = self
                        .scroll
                        .saturating_sub(self.geometry.reader.height.saturating_sub(8));
                } else {
                    self.move_selection(-8);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.scroll = 0;
                self.selected = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.focus == Focus::Reader {
                    self.scroll = self.max_scroll;
                } else {
                    self.move_selection(isize::MAX);
                }
            }
            _ => {}
        }
        Action::None
    }
    fn idle(&self) -> bool {
        !self.busy && self.writing.is_none() && self.fetching.is_none()
    }
    fn read_only(&mut self) {
        self.status = "Demo is read-only · run simplenote-cli tui to use your notes".into();
    }
    fn quit(&mut self) -> Action {
        if let Some(id) = &self.writing {
            self.overlay_scroll = 0;
            self.overlay = Some(Overlay::Saving(id.clone()));
            Action::None
        } else {
            Action::Quit
        }
    }
    fn change(&self, patch: Value) -> Action {
        self.current().map_or(Action::None, |note| Action::Change {
            id: note["id"].as_str().unwrap().into(),
            base: note["version"].as_u64().unwrap(),
            patch,
        })
    }
}

fn modified(note: &Value) -> f64 {
    note["modified"]
        .as_f64()
        .or_else(|| note["modified"].as_str().and_then(|s| s.parse().ok()))
        .filter(|n| n.is_finite())
        .unwrap_or_default()
}
