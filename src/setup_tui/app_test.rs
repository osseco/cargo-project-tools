use super::*;

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::config::AuditIgnore;
use crate::setup::tests::{Fixture, fixture};

/// The fixture's tool: `dep` 1.0.0 (with an advisory, fixed in 1.1.0) and `opt` behind the
/// default feature `extra`. Moving the tool isn't possible here. The guard keeps the
/// temporary directories while the test runs.
fn new_app() -> (App<'static>, impl Sized) {
    let Fixture {
        _dir,
        _registry,
        setup,
    } = fixture(AuditIgnore::default());
    let mover: Mover = Box::new(|_| anyhow::bail!("no moves in tests"));
    (App::new(setup, mover), (_dir, _registry))
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

fn click(app: &mut App, x: u16, y: u16) {
    app.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
}

fn labels(app: &App) -> Vec<String> {
    app.visible()
        .iter()
        .map(|&r| app.rows[r].key.clone())
        .collect()
}

fn row_of(app: &App, key: &str) -> usize {
    app.rows
        .iter()
        .position(|r| r.key == key)
        .unwrap_or_else(|| panic!("no row {key}"))
}

#[test]
fn rows_start_with_the_tool_features_and_direct_dependencies() {
    let (app, _guard) = new_app();
    assert_eq!(
        labels(&app),
        [
            "h:Tool",
            "tool",
            "h:Features",
            "default",
            "f:extra",
            "f:unused",
            "h:Dependencies",
            "root",
            "d:dep@1.0.0#tool/dep",
            "d:opt@0.1.0#tool/opt",
        ]
    );
    assert_eq!(app.selected, row_of(&app, "default"));
}

#[test]
fn turning_off_default_features_drops_their_dependencies() {
    let (mut app, _guard) = new_app();
    let screen = draw(&mut app);
    assert!(
        screen.contains("3 packages · 1 vulnerable · 0 warnings"),
        "{screen}"
    );
    assert!(screen.contains("[x] default"), "{screen}");
    assert!(screen.contains("[·] extra"), "{screen}");
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.pending, Some(Op::ToggleDefault));
    assert!(draw(&mut app).contains("working…"));
    // Nothing else happens until it has run.
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.selected, row_of(&app, "default"));
    app.run_pending();
    assert!(!labels(&app).iter().any(|l| l.starts_with("d:opt")));
    let screen = draw(&mut app);
    assert!(screen.contains("2 packages"), "{screen}");
    assert!(screen.contains("[ ] default"), "{screen}");
    assert!(screen.contains("[ ] extra"), "{screen}");
}

#[test]
fn clicking_a_feature_turns_it_on() {
    let (mut app, _guard) = new_app();
    draw(&mut app);
    let tree = app.areas.tree;
    let pos = app
        .visible()
        .iter()
        .position(|&r| r == row_of(&app, "f:unused"))
        .unwrap();
    click(&mut app, tree.x + 4, tree.y + pos as u16);
    assert_eq!(app.pending, Some(Op::ToggleFeature("unused".into())));
    app.run_pending();
    assert!(app.setup.selected.contains("unused"));
    assert!(draw(&mut app).contains("[x] unused"));
}

#[test]
fn next_finding_selects_the_vulnerable_package() {
    let (mut app, _guard) = new_app();
    app.press(Button::NextFinding);
    assert_eq!(app.selected, row_of(&app, "d:dep@1.0.0#tool/dep"));
    let screen = draw(&mut app);
    assert!(screen.contains("dep 1.0.0 ⚠"), "{screen}");
    assert!(
        screen.contains("vulnerability RUSTSEC-2024-0001: Dep is broken (patched: >=1.1.0)"),
        "{screen}"
    );
}

#[test]
fn a_package_lists_its_versions_and_why_it_is_built() {
    let (mut app, _guard) = new_app();
    app.next_finding();
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.pending, Some(Op::Load));
    assert_eq!(app.focus, Focus::Details);
    app.run_pending();
    let detail = app.detail.as_ref().unwrap();
    assert!(
        detail.why.iter().any(|l| l.contains("tool v1.0.0")),
        "{:?}",
        detail.why
    );
    // The cursor starts on the fix.
    let cursor = app.versions_state.selected().unwrap();
    assert_eq!(detail.versions[cursor].version.to_string(), "1.1.0");
    let screen = draw(&mut app);
    assert!(screen.contains("1.1.0  ★ fixes its advisories"), "{screen}");
    assert!(screen.contains("2.0.0  not semver-compatible"), "{screen}");
    assert!(screen.contains("Why it's built:"), "{screen}");

    // Offline, cargo can't really move it: the message says so and the lock stays.
    let before = std::fs::read_to_string(app.setup.lock_path()).unwrap();
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.pending, Some(Op::SetVersion(..))));
    app.run_pending();
    let (text, error) = app.message.clone().unwrap();
    assert!(error && text.contains("didn't move dep to 1.1.0"), "{text}");
    assert_eq!(
        std::fs::read_to_string(app.setup.lock_path()).unwrap(),
        before
    );
}

#[test]
fn the_current_version_is_not_a_move() {
    let (mut app, _guard) = new_app();
    app.next_finding();
    app.activate();
    app.run_pending();
    let current = app
        .detail
        .as_ref()
        .unwrap()
        .versions
        .iter()
        .position(|c| c.current)
        .unwrap();
    app.versions_state.select(Some(current));
    app.use_version();
    assert_eq!(app.pending, None);
    assert!(app.message.as_ref().unwrap().0.contains("already at 1.0.0"));
}

#[test]
fn a_failed_tool_move_says_why_and_keeps_the_version() {
    let (mut app, _guard) = new_app();
    app.queue(Op::MoveTool(None));
    app.run_pending();
    assert_eq!(
        app.message.clone().unwrap(),
        ("no moves in tests".to_string(), true)
    );
    assert_eq!(app.setup.checkout.version, "1.0.0");
}

#[test]
fn done_hands_back_the_setup_and_cancel_does_not() {
    let (mut app, _guard) = new_app();
    app.queue(Op::ToggleFeature("unused".into()));
    app.run_pending();
    chord(&mut app, 'j');
    // Done is the button the cursor starts on.
    press(&mut app, KeyCode::Enter);
    let setup = app.finish().unwrap();
    assert!(setup.selected.contains("unused"));

    let (mut app, _guard) = new_app();
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.finish().is_none());
}

#[test]
fn folding_survives_a_refresh() {
    let (mut app, _guard) = new_app();
    let root = row_of(&app, "root");
    app.rows[root].expanded = false;
    app.queue(Op::ToggleFeature("unused".into()));
    app.run_pending();
    let root = row_of(&app, "root");
    assert!(!app.rows[root].expanded);
    assert!(!labels(&app).iter().any(|l| l.starts_with("d:dep")));
}

#[test]
fn messages_are_sanitized() {
    let (mut app, _guard) = new_app();
    app.warn("cargo said \x1b[2Jgone");
    let screen = draw(&mut app);
    assert!(screen.contains("cargo said \\x1b[2Jgone"), "{screen}");
}

#[test]
fn a_selected_package_that_goes_away_is_handled() {
    // The last dependency is selected, and turning off its feature removes it: the rebuild
    // must not look it up in the shorter tree.
    let (mut app, _guard) = new_app();
    app.select(row_of(&app, "d:opt@0.1.0#tool/opt"));
    app.queue(Op::ToggleDefault);
    app.run_pending();
    assert_eq!(app.message.as_ref().map(|m| m.1), Some(false));
    assert!(!labels(&app).iter().any(|l| l.starts_with("d:opt")));
    assert!(app.selected < app.rows.len());
    draw(&mut app);
}
