use super::{
    markdown,
    state::{App, Focus, Overlay, Section},
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy)]
pub struct Theme {
    pub color: bool,
    pub truecolor: bool,
}
impl Theme {
    pub fn for_terminal() -> Self {
        Self {
            color: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
            truecolor: std::env::var("COLORTERM")
                .is_ok_and(|v| matches!(v.to_lowercase().as_str(), "truecolor" | "24bit"))
                || std::env::var("TERM").is_ok_and(|v| v.contains("direct"))
                || std::env::var_os("WT_SESSION").is_some(),
        }
    }
    fn style(self, fg: (u8, u8, u8), bg: (u8, u8, u8)) -> Style {
        if self.color {
            Style::default().fg(self.ink(fg)).bg(self.ink(bg))
        } else {
            Style::default()
        }
    }
    fn ink(self, rgb: (u8, u8, u8)) -> Color {
        if self.truecolor {
            return Color::Rgb(rgb.0, rgb.1, rgb.2);
        }
        // A restrained 256-color counterpart for terminals without truecolor.
        Color::Indexed(match rgb {
            (19, 27, 25) => 234,
            (204, 213, 204) => 188,
            (237, 232, 214) => 230,
            (143, 162, 150) => 246,
            (166, 205, 170) => 151,
            (68, 87, 77) => 239,
            (221, 197, 153) => 180,
            (231, 239, 226) => 254,
            (43, 64, 52) => 237,
            _ => 235,
        })
    }
    pub fn text(self) -> Style {
        self.style((204, 213, 204), (19, 27, 25))
    }
    pub fn strong(self) -> Style {
        self.style((237, 232, 214), (19, 27, 25))
    }
    pub fn muted(self) -> Style {
        self.style((143, 162, 150), (19, 27, 25))
    }
    pub fn accent(self) -> Style {
        self.style((166, 205, 170), (19, 27, 25))
    }
    pub fn rule(self) -> Style {
        self.style((68, 87, 77), (19, 27, 25))
    }
    pub fn code(self) -> Style {
        self.style((221, 197, 153), (19, 27, 25))
    }
    pub fn selected(self, focused: bool) -> Style {
        if !self.color {
            return Style::default().add_modifier(if focused {
                Modifier::REVERSED | Modifier::BOLD
            } else {
                Modifier::BOLD
            });
        }
        self.style(
            (231, 239, 226),
            if focused { (43, 64, 52) } else { (31, 43, 36) },
        )
    }
}
#[derive(Clone, Copy, Default)]
pub struct Geometry {
    pub sidebar: Rect,
    pub index: Rect,
    pub reader: Rect,
    pub list: Rect,
    pub sidebar_list: Rect,
}
pub fn draw(frame: &mut Frame<'_>, app: &mut App, theme: Theme) {
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.text()), area);
    app.geometry = Geometry::default();
    if area.width < 40 || area.height < 12 {
        frame.render_widget(
            Paragraph::new(
                "simplenote\n\nMake a little more room.\nMinimum: 40 columns × 12 rows\n\nq quit",
            )
            .style(theme.strong())
            .alignment(Alignment::Center),
            area,
        );
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(area);
    header(frame, app, rows[0], theme);
    let wide = area.width >= 112;
    let split = area.width >= 78;
    if wide {
        let columns = Layout::horizontal([
            Constraint::Length(20),
            Constraint::Length(36),
            Constraint::Min(1),
        ])
        .split(rows[1]);
        app.geometry.sidebar = columns[0];
        app.geometry.index = columns[1];
        app.geometry.reader = columns[2];
        sidebar(frame, app, columns[0], theme, false);
        index(frame, app, columns[1], theme, true);
        reader(frame, app, columns[2], theme);
    } else if split {
        let columns =
            Layout::horizontal([Constraint::Length(31), Constraint::Min(1)]).split(rows[1]);
        app.geometry.index = columns[0];
        app.geometry.reader = columns[1];
        index(frame, app, columns[0], theme, true);
        reader(frame, app, columns[1], theme);
    } else if app.reader_visible {
        app.geometry.reader = rows[1];
        reader(frame, app, rows[1], theme);
    } else {
        app.geometry.index = rows[1];
        index(frame, app, rows[1], theme, false);
    }
    footer(frame, app, rows[2], theme);
    if let Some(overlay) = app.overlay.clone() {
        overlay_view(frame, app, overlay, theme);
    }
}
fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect::new(
        area.x.saturating_add(x),
        area.y.saturating_add(y),
        area.width.saturating_sub(x * 2),
        area.height.saturating_sub(y * 2),
    )
}
fn header(frame: &mut Frame<'_>, app: &App, area: Rect, theme: Theme) {
    let inner = inset(area, 2, 0);
    let brand = if area.width >= 78 {
        "simplenote  /  "
    } else {
        "simplenote · "
    };
    let line = Line::from(vec![
        Span::styled(brand, theme.strong().add_modifier(Modifier::BOLD)),
        Span::styled(clean(app.section.name()), theme.muted()),
    ]);
    frame.render_widget(Paragraph::new(line), Rect { height: 1, ..inner });
    if area.width >= 78 {
        let connection = if app.demo {
            "DEMO · offline"
        } else if app.busy {
            ["·  ", "·· ", "···", " ··"][app.tick % 4]
        } else if !app.connected {
            "○ not connected"
        } else {
            "● connected"
        };
        frame.render_widget(
            Paragraph::new(connection)
                .style(theme.accent())
                .alignment(Alignment::Right),
            Rect {
                x: area.right().saturating_sub(24),
                width: 22,
                height: 1,
                ..area
            },
        );
    }
    let search = if let Some(query) = &app.search {
        Line::from(vec![
            Span::styled("/ ", theme.accent()),
            Span::styled(clean(query), theme.strong()),
            Span::styled("▏", theme.accent()),
            Span::styled("  Enter search account · Esc cancel", theme.muted()),
        ])
    } else if !app.query.is_empty() {
        Line::from(vec![
            Span::styled(format!("/ {}", clean(&app.query)), theme.accent()),
            Span::styled("  Esc clear", theme.muted()),
        ])
    } else {
        Line::from(Span::styled("/ Search your notes", theme.muted()))
    };
    frame.render_widget(
        Paragraph::new(search),
        Rect {
            y: area.y + 1,
            height: 1,
            ..inner
        },
    );
    frame.render_widget(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.rule()),
        area,
    );
}
fn sidebar(frame: &mut Frame<'_>, app: &mut App, area: Rect, theme: Theme, overlay: bool) {
    let block = Block::default()
        .borders(if overlay {
            Borders::NONE
        } else {
            Borders::RIGHT
        })
        .border_style(theme.rule())
        .padding(Padding::new(1, 1, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new("Library").style(theme.muted()),
        Rect { height: 1, ..inner },
    );
    let list_area = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(2),
        ..inner
    };
    if !overlay {
        app.geometry.sidebar_list = list_area;
    }
    let sections = app.sections();
    let items: Vec<_> = sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let active = s == &app.section;
            let name = match s {
                Section::Tag(tag) => format!("# {}", clean(tag)),
                _ => s.name().to_owned(),
            };
            let prefix = if active { "› " } else { "  " };
            let suffix = if i < 3 {
                format!("  {}", i + 1)
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{prefix}{name}"),
                    if active {
                        theme.accent().add_modifier(Modifier::BOLD)
                    } else {
                        theme.text()
                    },
                ),
                Span::styled(suffix, theme.muted()),
            ]))
        })
        .collect();
    app.sidebar_state.select(Some(
        app.sidebar_selected.min(sections.len().saturating_sub(1)),
    ));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme.selected(app.focus == Focus::Sidebar || overlay)),
        list_area,
        &mut app.sidebar_state,
    );
}
fn index(frame: &mut Frame<'_>, app: &mut App, area: Rect, theme: Theme, divider: bool) {
    let focused = app.focus == Focus::Index;
    let block = Block::default()
        .borders(if divider {
            Borders::RIGHT
        } else {
            Borders::NONE
        })
        .border_style(theme.rule())
        .padding(Padding::new(1, 1, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let visible = app.visible();
    let count = if app.cursor.is_some() {
        format!("{}+", visible.len())
    } else {
        visible.len().to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(if focused { "› " } else { "  " }, theme.accent()),
            Span::styled(
                clean(app.section.name()),
                theme.strong().add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {count}"), theme.muted()),
        ])),
        Rect { height: 1, ..inner },
    );
    let list_area = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(3),
        ..inner
    };
    app.geometry.list = list_area;
    if visible.is_empty() {
        let message = if app.busy {
            "Finding your notes…"
        } else if app.cursor.is_some() && app.search.is_none() {
            "No matches on this page.\n\nPress m to keep looking\nor / for a new search."
        } else if !app.query.is_empty() || app.search.is_some() {
            "No matching notes.\n\nTry another search or press\nEsc to clear it."
        } else if app.section == Section::Trash {
            "Nothing in Trash.\n\nA little less clutter."
        } else if app.section == Section::Pinned {
            "No pinned notes here.\n\nPress p on a note to keep\nit close."
        } else {
            "A fresh page.\n\nPress n to write a note."
        };
        frame.render_widget(
            Paragraph::new(message)
                .style(theme.muted())
                .wrap(Wrap { trim: false }),
            inset(list_area, 1, 1),
        );
    } else {
        let items: Vec<_> = visible
            .iter()
            .enumerate()
            .map(|(row, i)| {
                let note = &app.notes[*i];
                let selected = row == app.selected;
                let title = title(note);
                let preview = note["content"]
                    .as_str()
                    .unwrap_or_default()
                    .lines()
                    .skip_while(|l| l.trim().is_empty())
                    .skip(1)
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("Empty note");
                let first_tag = note["tags"]
                    .as_array()
                    .and_then(|t| t.first())
                    .and_then(|t| t.as_str())
                    .map(|t| format!("#{}", clean(t)))
                    .unwrap_or_else(|| "untagged".into());
                let pin = if note["pinned"] == true {
                    " · pinned"
                } else {
                    ""
                };
                let prefix = if selected { "› " } else { "  " };
                let style = if selected {
                    theme.selected(focused)
                } else {
                    theme.text()
                };
                ListItem::new(vec![
                    Line::from(Span::styled(
                        fit(&format!("{prefix}{title}"), list_area.width as usize),
                        style.add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        fit(
                            &format!("  {}", clean(preview.trim().trim_start_matches('#').trim())),
                            list_area.width as usize,
                        ),
                        if selected { style } else { theme.muted() },
                    )),
                    Line::from(Span::styled(
                        fit(&format!("  {first_tag}{pin}"), list_area.width as usize),
                        if selected { style } else { theme.muted() },
                    )),
                    Line::from(""),
                ])
                .style(style)
            })
            .collect();
        app.list_state
            .select(Some(app.selected.min(items.len() - 1)));
        frame.render_stateful_widget(List::new(items), list_area, &mut app.list_state);
    }
    let bottom = if app.cursor.is_some() {
        "m  Load more notes"
    } else if app.demo {
        "Sample collection"
    } else {
        if app.collection_loaded {
            "End of this collection"
        } else {
            "r  Refresh collection"
        }
    };
    frame.render_widget(
        Paragraph::new(bottom).style(theme.muted()),
        Rect {
            y: inner.bottom().saturating_sub(1),
            height: 1,
            ..inner
        },
    );
}
fn reader(frame: &mut Frame<'_>, app: &mut App, area: Rect, theme: Theme) {
    let margin = if area.width >= 70 { 4 } else { 2 };
    let mut inner = inset(area, margin, 1);
    if inner.width > 78 {
        inner.width = 78;
        inner.x = area.x + (area.width - inner.width) / 2;
    }
    let Some(note) = app.current() else {
        frame.render_widget(
            Paragraph::new("A little room to think.\n\nChoose a note from the index.")
                .style(theme.muted())
                .alignment(Alignment::Center),
            inset(inner, 1, inner.height / 3),
        );
        app.max_scroll = 0;
        return;
    };
    let deleted = note["deleted"] == true;
    let title = title(note);
    let content = note["content"].as_str().unwrap_or_default();
    let tags = note["tags"]
        .as_array()
        .map(|tags| {
            tags.iter()
                .filter_map(|t| t.as_str())
                .map(|t| format!("#{}", clean(t)))
                .collect::<Vec<_>>()
                .join("  ")
        })
        .unwrap_or_default();
    let words = content.split_whitespace().count();
    let metadata = format!(
        "{}{}{} words · v{}",
        tags,
        if tags.is_empty() { "" } else { "  ·  " },
        words,
        note["version"]
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if app.focus == Focus::Reader {
                    "› "
                } else {
                    ""
                },
                theme.accent(),
            ),
            Span::styled(title, theme.strong().add_modifier(Modifier::BOLD)),
        ])),
        Rect { height: 1, ..inner },
    );
    frame.render_widget(
        Paragraph::new(metadata).style(theme.muted()),
        Rect {
            y: inner.y + 2,
            height: 1,
            ..inner
        },
    );
    frame.render_widget(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.rule()),
        Rect {
            y: inner.y + 3,
            height: 1,
            ..inner
        },
    );
    let body_area = Rect {
        y: inner.y + 5,
        height: inner.height.saturating_sub(6),
        ..inner
    };
    // The title already has its own place in the page header.
    let mut body = content.lines();
    let first = body
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    let body = if first.trim_start().trim_start_matches('#').trim()
        == title_text(note).trim_start_matches('#').trim()
    {
        body.collect::<Vec<_>>().join("\n")
    } else {
        content.to_owned()
    };
    let text = if note["markdown"] == true {
        markdown::render(body.trim_start_matches('\n'), theme)
    } else {
        ratatui::text::Text::styled(
            crate::cli::safe_terminal_content(body.trim_start_matches('\n')).replace('\t', "    "),
            theme.text(),
        )
    };
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    let count = paragraph.line_count(body_area.width.max(1));
    app.max_scroll = count
        .saturating_sub(body_area.height as usize)
        .min(u16::MAX as usize) as u16;
    app.scroll = app.scroll.min(app.max_scroll);
    frame.render_widget(paragraph.scroll((app.scroll, 0)), body_area);
    if app.max_scroll > 0 {
        let mut state = ScrollbarState::new(count)
            .position(app.scroll as usize)
            .viewport_content_length(body_area.height as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(theme.rule())
                .thumb_style(theme.accent()),
            Rect {
                x: area.right().saturating_sub(1),
                width: 1,
                ..body_area
            },
            &mut state,
        );
    }
    let progress = if app.max_scroll == 0 {
        "All of this note".into()
    } else {
        format!("{}%", (app.scroll as usize * 100 / app.max_scroll as usize))
    };
    let footer = if area.width >= 65 {
        format!(
            "{}  ·  {}",
            if deleted {
                "In Trash · u restore"
            } else {
                "e edit in your editor"
            },
            progress
        )
    } else {
        progress
    };
    frame.render_widget(
        Paragraph::new(footer).style(theme.muted()),
        Rect {
            y: inner.bottom().saturating_sub(1),
            height: 1,
            ..inner
        },
    );
}
fn footer(frame: &mut Frame<'_>, app: &App, area: Rect, theme: Theme) {
    frame.render_widget(
        Block::default()
            .borders(Borders::TOP)
            .border_style(theme.rule()),
        area,
    );
    let inner = inset(area, 2, 0);
    let status = if app.busy {
        format!(
            "{}  {}",
            ["·  ", "·· ", "···", " ··"][app.tick % 4],
            app.status
        )
    } else {
        app.status.clone()
    };
    frame.render_widget(
        Paragraph::new(clean(&status)).style(theme.muted()),
        Rect {
            y: inner.y + 1,
            height: 1,
            ..inner
        },
    );
    let hints = if app.search.is_some() {
        "Enter search account   Esc cancel"
    } else if area.width < 55 && app.focus == Focus::Reader {
        "Esc index  ↑↓ scroll  ? help  q quit"
    } else if area.width < 55 {
        "↑↓ move  Enter read  ? help  q quit"
    } else if app.focus == Focus::Reader && area.width < 78 {
        "Esc index   j/k scroll   e edit   ? help   q quit"
    } else if area.width < 78 {
        "↑↓ select  Enter read  / search  ? help  q quit"
    } else {
        "↑↓ navigate   Tab focus   / search   n new   e edit   t tags   ? help   q quit"
    };
    frame.render_widget(
        Paragraph::new(hints).style(theme.accent()),
        Rect {
            y: inner.y + 2,
            height: 1,
            ..inner
        },
    );
}
fn overlay_view(frame: &mut Frame<'_>, app: &mut App, overlay: Overlay, theme: Theme) {
    let area = frame.area();
    let (title,body,height) = match &overlay {
        Overlay::Help => ("Make yourself at home", "Navigation\n  ↑ / ↓ or j / k   Move through notes / scroll page\n  Enter / l         Read selected note\n  Tab               Move focus between panes\n  Esc / h           Back to index / clear search\n  PgUp / PgDn       Move a page\n  g / G             Beginning / end\n\nYour collection\n  /                 Search, Enter to search account\n  t                 Browse tags and collections\n  1 / 2 / 3         All notes / pinned / trash\n  m / r             Load more / refresh\n\nYour notes\n  n / e             New note / edit in your editor\n  p                 Pin or unpin\n  d                 Move to Trash (confirmation)\n  u                 Restore a note from Trash\n\n  q / Ctrl-C        Quit\n\nEsc or Enter to return".to_owned(),29),
        Overlay::Tags => ("Collections & tags",String::new(),(app.sections().len()+7).min(28) as u16),
        Overlay::ConfirmTrash {title,..} => ("Move this note to Trash?",format!("{title}\n\nYou can restore it from Trash at any time.\n\ny  Move to Trash\nAny other key  Keep note"),10),
        Overlay::Message(message) => ("A moment before you continue",format!("{message}\n\nEsc or Enter to return"),area.height.saturating_sub(4)),
        Overlay::Saving(id) => ("A change is still saving",format!("Note {id}\n\nPlease wait for the result before quitting. Each request has a 30-second timeout.\n\nNo automatic retry will be made.\n\nEsc to return while you wait."),14),
    };
    let width = if area.width > 80 {
        72
    } else {
        area.width.saturating_sub(4)
    };
    let height = height.min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(theme.accent())
        .style(theme.text())
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if overlay == Overlay::Tags {
        sidebar(frame, app, inner, theme, true);
    } else {
        let paragraph = Paragraph::new(clean_multiline(&body))
            .style(theme.text())
            .wrap(Wrap { trim: false });
        app.overlay_max_scroll = paragraph
            .line_count(inner.width.max(1))
            .saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16;
        app.overlay_scroll = app.overlay_scroll.min(app.overlay_max_scroll);
        frame.render_widget(paragraph.scroll((app.overlay_scroll, 0)), inner);
        if app.overlay_max_scroll > 0 {
            frame.render_widget(
                Paragraph::new(" ↑↓ scroll · Esc close ")
                    .style(theme.accent())
                    .alignment(Alignment::Right),
                Rect {
                    y: popup.bottom() - 1,
                    height: 1,
                    ..popup
                },
            );
        }
    }
}
fn title_text(note: &serde_json::Value) -> &str {
    note["title"].as_str().unwrap_or_default()
}
pub fn title(note: &serde_json::Value) -> String {
    let title = clean(title_text(note).trim().trim_start_matches('#').trim());
    if title.is_empty() {
        "Untitled note".into()
    } else {
        title
    }
}
pub fn clean(text: &str) -> String {
    crate::cli::safe_text(text)
}
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 0;
    let mut clipped = String::new();
    for c in text.chars() {
        let next = c.width().unwrap_or_default();
        if used + next > width - 1 {
            break;
        }
        clipped.push(c);
        used += next;
    }
    clipped.push('…');
    clipped
}
fn clean_multiline(text: &str) -> String {
    crate::cli::safe_terminal_content(text).replace('\t', "    ")
}

/// Inspectable text from the same renderer used by interactive mode.
pub fn frame_text(app: &mut App, width: u16, height: u16) -> Result<String, crate::Error> {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap_or_else(|never| match never {});
    terminal
        .draw(|frame| {
            draw(
                frame,
                app,
                Theme {
                    color: false,
                    truecolor: false,
                },
            )
        })
        .unwrap_or_else(|never| match never {});
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..height {
        let mut line = String::new();
        let mut x = 0;
        while x < width {
            let symbol = buffer[(x, y)].symbol();
            line.push_str(symbol);
            x += symbol.width().max(1) as u16;
        }
        // Wide glyph continuation cells are empty in Ratatui's test backend.
        text.push_str(line.trim_end());
        text.push('\n');
    }
    Ok(text)
}
