use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color};
use serde_json::{Value, json};
use state::Section;

fn key(app: &mut App, code: KeyCode) -> Action {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn render(app: &mut App, width: u16, height: u16, color: bool) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            view::draw(
                frame,
                app,
                view::Theme {
                    color,
                    truecolor: true,
                },
            )
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
#[test]
fn demo_browsing_search_and_collections_are_coherent() {
    let mut app = App::new(true);
    assert_eq!(app.visible().len(), 7);
    key(&mut app, KeyCode::Down);
    assert_eq!(app.current().unwrap()["id"], "demo-1");
    key(&mut app, KeyCode::Char('/'));
    for c in "by the sea".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    assert_eq!(app.visible().len(), 1);
    assert!(matches!(
        key(&mut app, KeyCode::Enter),
        Action::Fetch(false)
    ));
    assert_eq!(app.query, "by the sea");
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.visible().len(), 7);
    key(&mut app, KeyCode::Char('2'));
    assert_eq!(app.visible().len(), 2);
    key(&mut app, KeyCode::Char('3'));
    assert_eq!(app.visible().len(), 1);
    assert_eq!(app.current().unwrap()["deleted"], true);
}
#[test]
fn demo_can_never_emit_write_actions() {
    let mut app = App::new(true);
    for code in ['n', 'e', 'p', 'd', 'u'] {
        assert!(matches!(key(&mut app, KeyCode::Char(code)), Action::None));
    }
    app.choose_section(Section::Trash);
    assert!(matches!(key(&mut app, KeyCode::Char('u')), Action::None));
    assert!(app.status.contains("read-only"));
}
#[test]
fn trash_requires_explicit_confirmation_and_uses_displayed_version() {
    let mut app = App::new(true);
    app.demo = false;
    key(&mut app, KeyCode::Char('d'));
    assert!(matches!(app.overlay, Some(Overlay::ConfirmTrash { .. })));
    assert!(matches!(key(&mut app, KeyCode::Char('n')), Action::None));
    assert!(app.overlay.is_none());
    key(&mut app, KeyCode::Char('d'));
    match key(&mut app, KeyCode::Char('y')) {
        Action::Change { id, base, patch } => {
            assert_eq!(id, "demo-0");
            assert_eq!(base, 12);
            assert_eq!(patch, json!({"deleted":true}));
        }
        other => panic!("Unexpected action: {other:?}"),
    }
    app.busy = true;
    for code in ['n', 'e', 'p', 'd', 'u'] {
        assert!(matches!(key(&mut app, KeyCode::Char(code)), Action::None));
    }
}
#[test]
fn compact_navigation_returns_from_reader_and_handles_resize() {
    let mut app = App::new(true);
    render(&mut app, 60, 20, true);
    assert_eq!(app.geometry.reader.width, 0);
    key(&mut app, KeyCode::Enter);
    render(&mut app, 60, 20, true);
    assert!(app.geometry.reader.width > 0);
    assert!(app.max_scroll > 0);
    key(&mut app, KeyCode::End);
    assert_eq!(app.scroll, app.max_scroll);
    key(&mut app, KeyCode::Esc);
    render(&mut app, 132, 38, true);
    assert_eq!(app.focus, Focus::Index);
    assert!(app.geometry.sidebar.width > 0);
    render(&mut app, 25, 8, true);
    assert!(matches!(key(&mut app, KeyCode::Char('q')), Action::Quit));
}
#[test]
fn replacement_and_append_preserve_selection_and_deduplicate() {
    let mut app = App::new(true);
    app.move_selection(2);
    let id = app.current().unwrap()["id"].clone();
    let mut notes = data::demo_notes();
    notes[2]["version"] = json!(99);
    app.set_notes(notes.clone(), false);
    assert_eq!(app.current().unwrap()["id"], id);
    app.set_notes(notes, true);
    assert_eq!(app.notes.len(), 8);
    assert_eq!(app.current().unwrap()["version"], 99);
}

#[test]
fn numeric_string_timestamps_sort_as_times_and_plain_text_stays_literal() {
    let mut app = App::new(true);
    app.notes[3]["modified"] = json!("1800000001");
    app.notes[3]["markdown"] = json!(false);
    app.notes[3]["content"] =
        json!("Notes on making things\n\nKeep **literal** markers and `code` as written.");
    app.selected = 2;
    assert_eq!(app.current().unwrap()["id"], "demo-3");
    let text = view::frame_text(&mut app, 132, 38).unwrap();
    assert!(text.contains("**literal**"));
    assert!(text.contains("`code`"));
}
#[test]
fn outdated_search_replies_are_ignored_and_cursor_loops_stop() {
    let mut app = App::new(false);
    app.generation = 2;
    app.busy = true;
    let fetch = Fetch {
        generation: 1,
        section: Section::All,
        query: "old".into(),
        cursor: None,
    };
    receive(
        &mut app,
        Reply::Page {
            fetch: fetch.clone(),
            result: Ok(json!({"items":data::demo_notes(),"next_cursor":"next"})),
        },
    );
    assert!(app.notes.is_empty());
    assert!(app.busy);
    let fetch = Fetch {
        generation: 2,
        ..fetch
    };
    receive(
        &mut app,
        Reply::Page {
            fetch: fetch.clone(),
            result: Ok(json!({"items":data::demo_notes(),"next_cursor":"next"})),
        },
    );
    assert_eq!(app.notes.len(), 8);
    assert!(!app.busy);
    assert!(app.connected);
    receive(
        &mut app,
        Reply::Page {
            fetch: Fetch {
                cursor: Some("next".into()),
                ..fetch
            },
            result: Ok(json!({"items":[],"next_cursor":"next"})),
        },
    );
    assert!(app.cursor.is_none());
    assert!(matches!(app.overlay, Some(Overlay::Message(_))));
}
#[test]
fn errors_keep_recovery_paths_and_uncertain_write_guidance() {
    let mut app = App::new(false);
    let mut error = Error::new("network", "Timed out");
    error.details = Some(json!({"outcome_unknown":true,"recovery_file":"/tmp/draft.md"}));
    show_error(&mut app, error);
    assert_eq!(app.recovery, ["/tmp/draft.md"]);
    assert!(!app.connected);
    match app.overlay {
        Some(Overlay::Message(message)) => {
            assert!(message.contains("may have succeeded"));
            assert!(message.contains("/tmp/draft.md"));
        }
        _ => panic!("Missing recovery message"),
    }
}
#[test]
fn narrow_help_scrolls_to_every_action() {
    let mut app = App::new(true);
    key(&mut app, KeyCode::Char('?'));
    render(&mut app, 48, 16, false);
    assert!(app.overlay_max_scroll > 0);
    for _ in 0..30 {
        key(&mut app, KeyCode::PageDown);
    }
    let text = view::frame_text(&mut app, 48, 16).unwrap();
    assert!(text.contains("Ctrl-C"));
    assert!(text.contains("return"));
}
#[test]
fn all_layouts_fit_and_no_color_has_no_palette() {
    for (width, height) in [(132, 38), (90, 28), (60, 24), (40, 12), (40, 10)] {
        let mut app = App::new(true);
        let buffer = render(&mut app, width, height, false);
        for cell in buffer.content() {
            assert_eq!(cell.fg, Color::Reset);
            assert_eq!(cell.bg, Color::Reset);
        }
        let text = view::frame_text(&mut app, width, height).unwrap();
        assert_eq!(text.lines().count(), height as usize);
        use unicode_width::UnicodeWidthStr;
        assert!(text.lines().all(|line| line.width() <= width as usize));
        if height >= 12 {
            assert!(text.contains("simplenote"));
            assert!(text.contains("q quit"));
        }
    }
}
#[test]
fn note_content_cannot_inject_terminal_controls() {
    let mut app = App::new(true);
    app.notes[0]["content"] = json!("# Hostile\n\n\u{1b}]52;c;bad\u{7}\n\u{202e}reversed");
    app.notes[0]["title"] = json!("\u{1b}[2J hostile");
    app.notes[0]["tags"] = json!(["\u{1b}]0;title\u{7}"]);
    let buffer = render(&mut app, 132, 38, true);
    for cell in buffer.content() {
        assert!(
            !cell
                .symbol()
                .chars()
                .any(|c| c.is_control() || c == '\u{202e}')
        );
    }
}
#[test]
fn account_search_reads_past_irrelevant_pages_and_preserves_pagination() {
    let mut server = mockito::Server::new();
    let page1 = server
        .mock("GET", "/1/test/note/index")
        .match_query(mockito::Matcher::AllOf(vec![
            mockito::Matcher::UrlEncoded("data".into(), "true".into()),
            mockito::Matcher::UrlEncoded("limit".into(), "50".into()),
        ]))
        .with_body(
            json!({"index":[{"id":"other","v":1,"d":{"content":"Other note"}}],"mark":"second"})
                .to_string(),
        )
        .expect(1)
        .create();
    let page2=server.mock("GET","/1/test/note/index").match_query(mockito::Matcher::UrlEncoded("mark".into(),"second".into())).with_body(json!({"index":[{"id":"found","v":3,"d":{"content":"A seaside note","tags":["travel"]}}]}).to_string()).expect(1).create();
    let http = Http::with_urls(&format!("{}/1/test/", server.url()), &server.url()).unwrap();
    let reply = data::perform(
        &Api {
            http: &http,
            token: "fixture".into(),
        },
        Request::Fetch(Fetch {
            generation: 9,
            section: Section::Tag("travel".into()),
            query: "sea".into(),
            cursor: None,
        }),
    );
    match reply {
        Reply::Page { fetch, result } => {
            assert_eq!(fetch.generation, 9);
            let value = result.unwrap();
            assert_eq!(value["items"][0]["id"], "found");
            assert!(value["next_cursor"].is_null());
        }
        _ => panic!("Wrong reply"),
    }
    page1.assert();
    page2.assert();
}
#[test]
fn pin_preserves_unknown_system_tags_and_refuses_stale_versions() {
    let mut server = mockito::Server::new();
    let read = server
        .mock("GET", "/1/test/note/i/abc")
        .with_header("x-simperium-version", "4")
        .with_body(json!({"content":"Note","systemTags":["markdown","future-flag"]}).to_string())
        .expect(3)
        .create();
    let write = server
        .mock("POST", "/1/test/note/i/abc/v/4")
        .match_query(mockito::Matcher::Any)
        .match_body(mockito::Matcher::PartialJson(
            json!({"systemTags":["markdown","future-flag","pinned"]}),
        ))
        .with_header("x-simperium-version", "5")
        .with_body(
            json!({"content":"Note","systemTags":["markdown","future-flag","pinned"]}).to_string(),
        )
        .expect(1)
        .create();
    let http = Http::with_urls(&format!("{}/1/test/", server.url()), &server.url()).unwrap();
    let api = Api {
        http: &http,
        token: "fixture".into(),
    };
    assert_eq!(api.pin("abc", 4, true).unwrap()["pinned"], true);
    assert_eq!(api.pin("abc", 3, true).unwrap_err().kind, "conflict");
    read.assert();
    write.assert();
}

#[test]
#[ignore = "Writes local visual review fixtures only when requested"]
fn capture_visual_review() {
    let path = std::path::PathBuf::from(
        std::env::var_os("SIMPLENOTE_TUI_CAPTURE").expect("capture directory"),
    );
    std::fs::create_dir_all(&path).unwrap();
    for (name, width, height, color) in [
        ("wide", 132, 38, true),
        ("medium", 90, 28, true),
        ("narrow", 60, 24, true),
        ("mono", 100, 30, false),
        ("tiny", 40, 10, true),
    ] {
        let mut app = App::new(true);
        if name == "narrow" {
            key(&mut app, KeyCode::Enter);
        }
        let buffer = render(&mut app, width, height, color);
        let cells:Vec<Value>=buffer.content().iter().map(|cell|json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"bold":cell.modifier.contains(ratatui::style::Modifier::BOLD),"reverse":cell.modifier.contains(ratatui::style::Modifier::REVERSED)})).collect();
        std::fs::write(
            path.join(format!("{name}.json")),
            serde_json::to_vec(&json!({"width":width,"height":height,"cells":cells})).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn quit_waits_for_write_result_and_loaded_pages_survive_writes() {
    let mut app = App::new(true);
    app.demo = false;
    app.selected = 3;
    let id = app.current().unwrap()["id"].clone();
    app.writing = Some(id.as_str().unwrap().into());
    app.busy = true;
    assert!(matches!(key(&mut app, KeyCode::Char('q')), Action::None));
    assert!(matches!(app.overlay, Some(Overlay::Saving(_))));
    let mut updated = app.current().unwrap().clone();
    updated["version"] = json!(90);
    receive(&mut app, Reply::Written(Ok(updated)));
    assert!(app.writing.is_none());
    assert!(app.overlay.is_none());
    assert_eq!(app.notes.len(), 8);
    assert_eq!(app.current().unwrap()["id"], id);
    assert_eq!(app.current().unwrap()["version"], 90);
    assert!(matches!(key(&mut app, KeyCode::Char('q')), Action::Quit));
}
#[test]
fn trash_confirmation_captures_the_original_note_and_enter_cancels() {
    let mut app = App::new(true);
    app.demo = false;
    let original = app.current().unwrap()["id"].clone();
    key(&mut app, KeyCode::Char('d'));
    assert!(matches!(key(&mut app, KeyCode::Enter), Action::None));
    assert!(app.overlay.is_none());
    key(&mut app, KeyCode::Char('d'));
    app.selected = 3;
    match key(&mut app, KeyCode::Char('y')) {
        Action::Change { id, .. } => assert_eq!(id, original.as_str().unwrap()),
        other => panic!("{other:?}"),
    }
}
#[test]
fn failed_collection_switch_discards_the_previous_cursor() {
    let mut app = App::new(false);
    app.cursor = Some("old-query-cursor".into());
    let mut pending = None;
    fetch(&mut app, false, None, &mut pending);
    assert!(app.cursor.is_none());
    let generation = app.generation;
    receive(
        &mut app,
        Reply::Page {
            fetch: Fetch {
                generation,
                section: Section::All,
                query: String::new(),
                cursor: None,
            },
            result: Err(Error::new("network", "Failed")),
        },
    );
    app.overlay = None;
    assert!(matches!(key(&mut app, KeyCode::Char('m')), Action::None));
}
#[test]
fn created_note_is_selected_even_outside_the_first_index_page() {
    let mut app = App::new(true);
    app.demo = false;
    app.section = Section::Trash;
    let mut note = data::demo_notes().remove(0);
    note["id"] = json!("new-note");
    apply_saved(&mut app, note, true);
    assert_eq!(app.section, Section::All);
    assert_eq!(app.current().unwrap()["id"], "new-note");
    assert!(app.cursor.is_none());
    assert!(!app.collection_loaded);
}
#[test]
fn unicode_tags_and_no_op_writes_match_the_cli_contract() {
    let mut app = App::new(true);
    app.notes[0]["tags"] = json!(["Über"]);
    app.section = Section::Tag("über".into());
    assert_eq!(app.visible().len(), 1);
    let note = app.current().unwrap().clone();
    receive(
        &mut app,
        Reply::Written(Ok(json!({"no_op":true,"note":note}))),
    );
    assert_eq!(app.status, "Unchanged · nothing written");
}
#[test]
fn pinned_and_trash_search_scan_past_irrelevant_index_pages() {
    for (collection, raw) in [
        (
            crate::api::Collection::Pinned,
            json!({"content":"Match","systemTags":["pinned"]}),
        ),
        (
            crate::api::Collection::Trash,
            json!({"content":"Match","deleted":true}),
        ),
    ] {
        let mut server = mockito::Server::new();
        let first = server
            .mock("GET", "/1/test/note/index")
            .match_query(mockito::Matcher::Missing)
            .with_body("{}")
            .expect(0)
            .create();
        let page1=server.mock("GET","/1/test/note/index").match_query(mockito::Matcher::UrlEncoded("limit".into(),"50".into())).with_body(json!({"index":[{"id":"ordinary","v":1,"d":{"content":"An ordinary active note"}}],"mark":"second"}).to_string()).expect(1).create();
        let page2 = server
            .mock("GET", "/1/test/note/index")
            .match_query(mockito::Matcher::UrlEncoded("mark".into(), "second".into()))
            .with_body(json!({"index":[{"id":"match","v":1,"d":raw}]}).to_string())
            .expect(1)
            .create();
        let http = Http::with_urls(&format!("{}/1/test/", server.url()), &server.url()).unwrap();
        let result = Api {
            http: &http,
            token: "fixture".into(),
        }
        .list_collection(50, None, None, None, collection)
        .unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(result["items"][0]["id"], "match");
        first.assert();
        page1.assert();
        page2.assert();
    }
}

#[test]
fn limited_color_terminals_use_indexed_colors() {
    let mut app = App::new(true);
    let mut terminal = Terminal::new(TestBackend::new(90, 28)).unwrap();
    terminal
        .draw(|frame| {
            view::draw(
                frame,
                &mut app,
                view::Theme {
                    color: true,
                    truecolor: false,
                },
            )
        })
        .unwrap();
    for cell in terminal.backend().buffer().content() {
        assert!(!matches!(cell.fg, Color::Rgb(..)));
        assert!(!matches!(cell.bg, Color::Rgb(..)));
    }
}
#[test]
#[ignore = "Requires a real terminal; exercised by the PTY smoke test"]
fn terminal_suspend_resume_fixture() {
    let mut guard = TerminalGuard::enter().unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout())).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget("Before editor", frame.area());
        })
        .unwrap();
    assert!(crossterm::terminal::is_raw_mode_enabled().unwrap());
    guard.suspend();
    assert!(!crossterm::terminal::is_raw_mode_enabled().unwrap());
    #[cfg(unix)]
    assert!(
        std::process::Command::new("sh")
            .args(["-c", "printf 'Editor handoff\n'"])
            .status()
            .unwrap()
            .success()
    );
    guard.resume().unwrap();
    redraw_after_editor(&mut terminal).unwrap();
    terminal
        .draw(|frame| {
            frame.render_widget("After editor", frame.area());
        })
        .unwrap();
    assert!(crossterm::terminal::is_raw_mode_enabled().unwrap());
    drop(terminal);
    drop(guard);
    assert!(!crossterm::terminal::is_raw_mode_enabled().unwrap());
}

#[test]
fn write_completion_does_not_unlock_mutations_while_a_search_is_pending() {
    let mut app = App::new(true);
    app.demo = false;
    app.fetching = Some(3);
    app.generation = 3;
    app.writing = Some("demo-0".into());
    let note = app.current().unwrap().clone();
    receive(&mut app, Reply::Written(Ok(note)));
    assert!(app.busy);
    for code in ['p', 'd', 'e', 'n'] {
        assert!(matches!(key(&mut app, KeyCode::Char(code)), Action::None));
    }
    receive(
        &mut app,
        Reply::Page {
            fetch: Fetch {
                generation: 3,
                section: Section::All,
                query: String::new(),
                cursor: None,
            },
            result: Ok(json!({"items":data::demo_notes(),"next_cursor":null})),
        },
    );
    assert!(!app.busy);
    assert!(matches!(
        key(&mut app, KeyCode::Char('p')),
        Action::Pin { .. }
    ));
}
