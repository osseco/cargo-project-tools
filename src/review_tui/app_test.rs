use super::*;

use std::path::PathBuf;

use crate::review::tui::Choice;
use crate::review::{Entry, Pin};

use std::path::Path;

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::review::tui::fixtures::{self, cargo_config, proc_macro, root_script};
use crate::review::tui::text::Warn;
use crate::review::tui::text::commented_copy;

fn app(dir: &Path, items: Vec<Item>) -> App {
    let pending: Vec<Pending> = items.into_iter().map(|i| (i, None)).collect();
    App::new(
        "tool",
        &pending,
        &vec![None; pending.len()],
        &dir.join("crate"),
    )
    .unwrap()
}

fn draw(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 30)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buffer = terminal.backend().buffer();
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>() + "\n")
        .collect()
}

fn press(app: &mut App, code: KeyCode) {
    app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn chord(app: &mut App, c: char) {
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('w'),
        KeyModifiers::CONTROL,
    )));
    press(app, KeyCode::Char(c));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn click(app: &mut App, rect: Rect) {
    app.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    }));
}

fn button(app: &App, b: Button) -> Rect {
    app.areas.buttons.iter().find(|(_, x)| *x == b).unwrap().0
}

fn node(app: &App, label: &str) -> usize {
    app.nodes
        .iter()
        .position(|n| n.label.starts_with(label))
        .unwrap_or_else(|| panic!("no node {label}"))
}

fn editor_text(app: &App) -> String {
    match &app.open.as_ref().unwrap().content {
        OpenContent::Text { state, .. } => state.lines.to_string(),
        _ => panic!("not text"),
    }
}

#[test]
fn decisions_by_keyboard_and_done() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(
        tmp.path(),
        vec![
            root_script(tmp.path(), "a", "fn main() {}\n"),
            proc_macro(tmp.path(), "m"),
        ],
    );
    // Starts on the first piece; approve it from the radios.
    chord(&mut app, 'k');
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.radio[0], Some(Action::Approve));
    // A proc macro can't be stripped.
    app.run(Then::Select(node(&app, "m@")));
    app.set_radio(1, Action::Strip);
    assert_eq!(app.radio[1], None);
    assert!(app.message.as_ref().unwrap().1);
    let screen = draw(&mut app);
    assert!(screen.contains("1 approved"), "{screen}");
    assert!(screen.contains("1 undecided"), "{screen}");
    // Done from the buttons, by keyboard.
    chord(&mut app, 'j');
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.result().unwrap(),
        [
            Some(Choice {
                action: Action::Approve,
                file: None
            }),
            None
        ]
    );
}

#[test]
fn cancel_decides_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "x\n")]);
    app.set_radio(0, Action::Approve);
    draw(&mut app);
    let cancel = button(&app, Button::Cancel);
    click(&mut app, cancel);
    assert_eq!(app.result(), None);
}

#[test]
fn mouse_selects_rows_and_sets_radios() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(
        tmp.path(),
        vec![
            root_script(tmp.path(), "a", "x\n"),
            root_script(tmp.path(), "b", "y\n"),
        ],
    );
    draw(&mut app);
    // Rows: build-script, a, b.
    let tree = app.areas.tree;
    click(&mut app, Rect::new(tree.x + 2, tree.y + 2, 1, 1));
    assert_eq!(app.selected, node(&app, "b@"));
    assert_eq!(editor_text(&app), "y\n");
    draw(&mut app);
    let strip = app.areas.radios[1].0;
    click(&mut app, strip);
    assert_eq!(app.radio, [None, Some(Action::Strip)]);
}

#[test]
fn shipped_files_are_view_only() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "abc\n")]);
    chord(&mut app, 'l');
    typed(&mut app, "xiZZ");
    press(&mut app, KeyCode::Esc);
    typed(&mut app, "dd");
    assert_eq!(editor_text(&app), "abc\n");
    assert!(!app.modified());
    // Keys edtui doesn't know are dropped rather than panicking.
    press(&mut app, KeyCode::F(5));
    press(&mut app, KeyCode::Insert);
}

#[test]
fn edited_override_is_saved_on_leaving() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(
        tmp.path(),
        vec![
            root_script(tmp.path(), "a", "fn main() {}\n"),
            root_script(tmp.path(), "b", "y\n"),
        ],
    );
    app.set_radio(0, Action::Override);
    chord(&mut app, 'l');
    typed(&mut app, "i// ours ");
    press(&mut app, KeyCode::Esc);
    assert!(app.modified());
    // Leaving asks first; Save writes the override and moves on.
    chord(&mut app, 'h');
    press(&mut app, KeyCode::Char('j'));
    assert!(app.dialog.is_some());
    draw(&mut app);
    let save = app.areas.answers[0].0;
    click(&mut app, save);
    assert!(app.dialog.is_none());
    assert_eq!(app.selected, node(&app, "b@"));
    let written = tmp.path().join("crate/overrides/a/build.rs");
    assert_eq!(
        fs::read_to_string(&written).unwrap(),
        "// ours fn main() {}\n"
    );
    assert_eq!(
        app.choice(0),
        Some(Choice {
            action: Action::Override,
            file: Some("overrides/a/build.rs".into())
        })
    );
    // Back on it, the override is what's shown.
    app.run(Then::Select(node(&app, "a@")));
    assert!(app.open.as_ref().unwrap().showing_override);
    assert_eq!(editor_text(&app), "// ours fn main() {}\n");
}

#[test]
fn discarded_edit_leaves_override_undecided() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "x\n")]);
    app.set_radio(0, Action::Override);
    chord(&mut app, 'l');
    typed(&mut app, "iy");
    press(&mut app, KeyCode::Esc);
    // Done asks first.
    app.press(Button::Done);
    assert!(app.dialog.is_some());
    app.answer(Answer::Discard);
    assert_eq!(app.exit, Some(Exit::Done));
    assert!(!tmp.path().join("crate/overrides").exists());
    assert_eq!(app.result().unwrap(), [None]);
}

#[test]
fn unsafe_files_are_view_only_and_overridden_by_a_commented_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let text = "fn main() { let s = \"\u{202e}x\"; }\x1b[2K\n";
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", text)]);
    assert_eq!(app.nodes[1].warn, Warn::Unsafe);
    let screen = draw(&mut app);
    assert!(screen.contains("can't be shown as they are"), "{screen}");
    assert!(screen.contains("<U+202E>"), "{screen}");
    assert!(!screen.contains('\u{202e}'));

    app.set_radio(0, Action::Override);
    let written = tmp.path().join("crate/overrides/a/build.rs");
    let copy = fs::read(&written).unwrap();
    assert_eq!(copy, commented_copy(text.as_bytes(), true));
    assert_eq!(
        app.choice(0),
        Some(Choice {
            action: Action::Override,
            file: Some("overrides/a/build.rs".into())
        })
    );
    // The copy is shown, sanitized, and still can't be edited here.
    assert!(app.open.as_ref().unwrap().showing_override);
    chord(&mut app, 'l');
    typed(&mut app, "ix");
    assert!(!app.modified());
    // Choosing override again keeps the copy someone may be editing.
    fs::write(&written, "fn main() {}\n").unwrap();
    app.set_radio(0, Action::Approve);
    app.set_radio(0, Action::Override);
    assert_eq!(fs::read_to_string(&written).unwrap(), "fn main() {}\n");
}

/// Choose override for the first piece and type `text` at the start of its file.
fn edit(app: &mut App, text: &str) {
    app.set_radio(0, Action::Override);
    chord(app, 'l');
    typed(app, &format!("i{text}"));
    press(app, KeyCode::Esc);
    assert!(app.modified());
}

#[test]
fn stay_keeps_the_edits_and_the_place() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(
        tmp.path(),
        vec![
            root_script(tmp.path(), "a", "x\n"),
            root_script(tmp.path(), "b", "y\n"),
        ],
    );
    let a = app.selected;
    edit(&mut app, "z");
    app.guarded(Then::Select(node(&app, "b@")));
    // Esc answers Stay.
    press(&mut app, KeyCode::Esc);
    assert!(app.dialog.is_none());
    assert_eq!(app.selected, a);
    assert_eq!(editor_text(&app), "zx\n");
    assert!(app.modified());
}

#[test]
fn saving_asks_before_replacing_an_override_it_did_not_open() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "x\n")]);
    edit(&mut app, "mine ");
    // Someone else writes the override meanwhile.
    let dest = tmp.path().join("crate/overrides/a/build.rs");
    fixtures::write(&dest, "theirs\n");

    app.press(Button::Save);
    assert_eq!(app.dialog.as_ref().unwrap().ask, Ask::Overwrite(None));
    app.answer(Answer::Stay);
    assert_eq!(fs::read_to_string(&dest).unwrap(), "theirs\n");
    assert!(app.modified());

    app.press(Button::Save);
    draw(&mut app);
    let replace = app.areas.answers[0];
    assert_eq!(replace.1, Answer::Overwrite);
    click(&mut app, replace.0);
    assert_eq!(fs::read_to_string(&dest).unwrap(), "mine x\n");
    assert!(!app.modified());
}

#[test]
fn an_override_changed_on_disk_since_it_was_opened_is_not_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "x\n")]);
    edit(&mut app, "1");
    app.press(Button::Save);
    let dest = tmp.path().join("crate/overrides/a/build.rs");
    assert_eq!(fs::read_to_string(&dest).unwrap(), "1x\n");

    typed(&mut app, "i2");
    press(&mut app, KeyCode::Esc);
    fs::write(&dest, "changed elsewhere\n").unwrap();
    assert_eq!(app.save_override(true), Saved::No);
    let (message, error) = app.message.clone().unwrap();
    assert!(error && message.contains("changed on disk"), "{message}");
    assert_eq!(fs::read_to_string(&dest).unwrap(), "changed elsewhere\n");

    // Discarding loads what's on disk now.
    app.guarded(Then::Select(app.selected));
    app.answer(Answer::Discard);
    assert_eq!(editor_text(&app), "changed elsewhere\n");
}

#[test]
fn save_with_nothing_to_save_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "x\n")]);
    let screen = draw(&mut app);
    assert!(screen.contains("[ Save override ]"), "{screen}");
    let save = button(&app, Button::Save);
    let buffer_style = {
        let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 30)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        terminal.backend().buffer()[(save.x, save.y)].modifier
    };
    assert!(buffer_style.contains(ratatui::style::Modifier::DIM));
    click(&mut app, save);
    assert!(app.message.as_ref().unwrap().0.contains("nothing to save"));
}

#[test]
fn cargo_config_is_overridden_under_its_own_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![cargo_config(tmp.path(), "[build]\n")]);
    let screen = draw(&mut app);
    assert!(
        screen.contains(".cargo/config.toml (shipped, view only)"),
        "{screen}"
    );
    edit(&mut app, "# ours\n");
    app.press(Button::Save);
    assert_eq!(
        fs::read_to_string(
            tmp.path()
                .join("crate/overrides/cargo-config/.cargo/config.toml")
        )
        .unwrap(),
        "# ours\n[build]\n"
    );
}

#[test]
fn tree_folds_by_keyboard_and_mouse() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![proc_macro(tmp.path(), "m")]);
    // On the proc macro, folded: its files are hidden and nothing is open.
    assert_eq!(app.visible(), [0, 1]);
    assert!(app.open.is_none());
    press(&mut app, KeyCode::Char('l'));
    assert_eq!(app.visible().len(), 4);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.nodes[app.selected].label, "Cargo.toml");
    assert!(editor_text(&app).starts_with("[package]"));
    // h on a file goes to its folder, h again folds it.
    press(&mut app, KeyCode::Char('h'));
    assert_eq!(app.selected, 1);
    press(&mut app, KeyCode::Char('h'));
    assert_eq!(app.visible(), [0, 1]);
    // Clicking the selected folder unfolds it.
    draw(&mut app);
    let tree = app.areas.tree;
    click(&mut app, Rect::new(tree.x, tree.y + 1, 1, 1));
    assert_eq!(app.visible().len(), 4);
}

#[test]
fn title_counts_follow_the_decisions() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(
        tmp.path(),
        vec![
            root_script(tmp.path(), "a", "x\n"),
            root_script(tmp.path(), "b", "y\n"),
            root_script(tmp.path(), "c", "z\n"),
        ],
    );
    app.set_radio(0, Action::Approve);
    app.set_radio(1, Action::Strip);
    let screen = draw(&mut app);
    assert!(
        screen.contains("1 approved · 0 override · 1 stripped · 1 undecided"),
        "{screen}"
    );
}

fn buffer(app: &mut App) -> ratatui::buffer::Buffer {
    let mut terminal = ratatui::Terminal::new(TestBackend::new(140, 30)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    terminal.backend().buffer().clone()
}

/// The first cell of `text` drawn in the editor.
fn editor_cell(app: &App, buf: &ratatui::buffer::Buffer, text: &str) -> (u16, u16) {
    let e = app.areas.editor;
    let chars: Vec<char> = text.chars().collect();
    for y in e.top()..e.bottom() {
        for x in e.left()..e.right() {
            let hit = chars.iter().enumerate().all(|(k, c)| {
                buf.cell((x + k as u16, y))
                    .is_some_and(|cell| cell.symbol() == c.to_string())
            });
            if hit {
                return (x, y);
            }
        }
    }
    panic!("{text} not in the editor");
}

#[test]
fn the_unfocused_cursor_leaves_its_cell_alone() {
    use ratatui::style::{Color, Modifier};
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(
        tmp.path(),
        vec![root_script(tmp.path(), "a", "fn main() {}\n")],
    );
    // The cursor is on `f`, the editor isn't focused: `fn` is drawn like any keyword.
    let buf = buffer(&mut app);
    let (x, y) = editor_cell(&app, &buf, "fn main");
    for cell in [&buf[(x, y)], &buf[(x + 1, y)]] {
        assert_eq!(cell.fg, Color::Magenta);
        assert_eq!(cell.bg, Color::Reset);
        assert!(!cell.modifier.contains(Modifier::REVERSED));
    }
    chord(&mut app, 'l');
    let buf = buffer(&mut app);
    assert!(buf[(x, y)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(x + 1, y)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn buttons_are_centred_with_the_message_above() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "x\n")]);
    app.say("hello");
    let buf = buffer(&mut app);
    let first = app.areas.buttons.first().unwrap().0;
    let last = app.areas.buttons.last().unwrap().0;
    // Inside the outer border: columns 1..139.
    let (left, right) = (first.x - 1, 139 - last.right());
    assert!(left.abs_diff(right) <= 1, "{left} vs {right}");
    let row: String = (1..139)
        .map(|x| buf[(x, first.y - 1)].symbol().to_string())
        .collect();
    assert!(row.trim_start().starts_with("hello"), "{row}");
}

#[test]
fn a_blank_row_under_the_title_and_above_the_editor() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app(tmp.path(), vec![root_script(tmp.path(), "a", "x\n")]);
    let buf = buffer(&mut app);
    let row = |y: u16, from: u16| -> String {
        (from..139)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    };
    assert_eq!(row(1, 1).trim(), "");
    // The tree's border starts below that row.
    assert_eq!(buf[(1, 2)].symbol(), "┌");
    let radios = app.areas.radios[0].0;
    assert_eq!(radios.y, 2);
    assert_eq!(row(radios.y + 1, radios.x).trim(), "");
    assert_eq!(app.areas.editor.y, radios.y + 2);
}

/// An app whose pieces have these decisions recorded for other tools.
fn app_knowing(dir: &Path, items: Vec<Item>, known: Vec<Option<Known>>) -> App {
    let pending: Vec<Pending> = items.into_iter().map(|i| (i, None)).collect();
    App::new("tool", &pending, &known, &dir.join("crate")).unwrap()
}

fn known(tool: &str, action: Action, file: Option<PathBuf>) -> Option<Known> {
    Some(Known {
        tool: tool.into(),
        action,
        file,
    })
}

#[test]
fn decisions_recorded_for_other_tools_are_pre_selected() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_knowing(
        tmp.path(),
        vec![
            root_script(tmp.path(), "a", "x\n"),
            root_script(tmp.path(), "b", "y\n"),
            root_script(tmp.path(), "c", "z\n"),
        ],
        vec![
            known("nextest", Action::Approve, None),
            known("deny", Action::Strip, None),
            None,
        ],
    );
    assert_eq!(
        app.radio,
        [Some(Action::Approve), Some(Action::Strip), None]
    );
    let screen = draw(&mut app);
    assert!(screen.contains("approved in nextest"), "{screen}");
    assert!(screen.contains("stripped in deny"), "{screen}");
    // Pre-selected, not decided for you: it's recorded on Done.
    assert_eq!(app.result(), None);
    app.press(Button::Done);
    assert_eq!(
        app.result().unwrap()[1].as_ref().unwrap().action,
        Action::Strip
    );
}

#[test]
fn another_tools_override_is_copied_on_done_only() {
    let tmp = tempfile::tempdir().unwrap();
    let theirs = tmp.path().join("crates/nextest/overrides/a/build.rs");
    fixtures::write(&theirs, "fn main() { /* theirs */ }\n");
    let items = || vec![root_script(tmp.path(), "a", "fn main() {}\n")];
    let hint = || vec![known("nextest", Action::Override, Some(theirs.clone()))];
    let ours = tmp.path().join("crate/overrides/a/build.rs");

    let mut app = app_knowing(tmp.path(), items(), hint());
    assert_eq!(app.files[0].as_deref(), Some("overrides/a/build.rs"));
    // Shown from their file, named as such.
    assert_eq!(editor_text(&app), "fn main() { /* theirs */ }\n");
    let screen = draw(&mut app);
    assert!(
        screen.contains("overrides/a/build.rs from nextest"),
        "{screen}"
    );
    app.press(Button::Cancel);
    assert!(!ours.exists());

    let mut app = app_knowing(tmp.path(), items(), hint());
    app.press(Button::Done);
    app.copy_sources().unwrap();
    assert_eq!(
        fs::read_to_string(&ours).unwrap(),
        "fn main() { /* theirs */ }\n"
    );
    assert_eq!(
        app.result().unwrap(),
        [Some(Choice {
            action: Action::Override,
            file: Some("overrides/a/build.rs".into())
        })]
    );
}

#[test]
fn an_override_of_ours_that_differs_is_kept_and_left_undecided() {
    let tmp = tempfile::tempdir().unwrap();
    let theirs = tmp.path().join("crates/nextest/overrides/a/build.rs");
    fixtures::write(&theirs, "theirs\n");
    let ours = tmp.path().join("crate/overrides/a/build.rs");
    fixtures::write(&ours, "ours\n");
    let app = app_knowing(
        tmp.path(),
        vec![root_script(tmp.path(), "a", "x\n")],
        vec![known("nextest", Action::Override, Some(theirs))],
    );
    assert_eq!(app.radio, [None]);
    assert!(
        app.message
            .as_ref()
            .unwrap()
            .0
            .contains("differs from nextest")
    );
    assert_eq!(fs::read_to_string(&ours).unwrap(), "ours\n");
    // The same content is simply used.
    fs::write(&ours, "theirs\n").unwrap();
    let theirs = tmp.path().join("crates/nextest/overrides/a/build.rs");
    let app = app_knowing(
        tmp.path(),
        vec![root_script(tmp.path(), "a", "x\n")],
        vec![known("nextest", Action::Override, Some(theirs))],
    );
    assert_eq!(app.sources, [None]);
    assert_eq!(app.radio, [Some(Action::Override)]);
}

#[test]
fn editing_another_tools_override_saves_ours() {
    let tmp = tempfile::tempdir().unwrap();
    let theirs = tmp.path().join("crates/nextest/overrides/a/build.rs");
    fixtures::write(&theirs, "theirs\n");
    let mut app = app_knowing(
        tmp.path(),
        vec![root_script(tmp.path(), "a", "x\n")],
        vec![known("nextest", Action::Override, Some(theirs.clone()))],
    );
    chord(&mut app, 'l');
    typed(&mut app, "imine ");
    press(&mut app, KeyCode::Esc);
    app.press(Button::Save);
    let ours = tmp.path().join("crate/overrides/a/build.rs");
    assert_eq!(fs::read_to_string(&ours).unwrap(), "mine theirs\n");
    assert_eq!(fs::read_to_string(&theirs).unwrap(), "theirs\n");
    assert_eq!(app.sources, [None]);
}

/// A changed piece's earlier entry with `action`.
fn earlier(item: &Item, action: Action) -> Entry {
    Entry {
        kind: item.kind,
        package: item.package.clone(),
        path: item.path.clone(),
        hash: "old".into(),
        pin: Pin::None,
        action,
        file: None,
    }
}

#[test]
fn a_changed_piece_starts_on_strip_but_not_on_approve() {
    let tmp = tempfile::tempdir().unwrap();
    let items = [
        root_script(tmp.path(), "a", "x\n"),
        root_script(tmp.path(), "b", "y\n"),
    ];
    let entries = [
        earlier(&items[0], Action::Approve),
        earlier(&items[1], Action::Strip),
    ];
    let pending: Vec<Pending> = items
        .iter()
        .cloned()
        .zip(entries.iter().map(Some))
        .collect();
    let app = App::new("tool", &pending, &[None, None], &tmp.path().join("crate")).unwrap();
    assert_eq!(app.radio, [None, Some(Action::Strip)]);
}

#[test]
fn another_tools_approve_does_not_replace_our_strip() {
    let tmp = tempfile::tempdir().unwrap();
    let item = root_script(tmp.path(), "a", "x\n");
    let entry = earlier(&item, Action::Strip);
    let pending: Vec<Pending> = vec![(item, Some(&entry))];
    let app = App::new(
        "tool",
        &pending,
        &[known("nextest", Action::Approve, None)],
        &tmp.path().join("crate"),
    )
    .unwrap();
    assert_eq!(app.radio, [Some(Action::Strip)]);
}
