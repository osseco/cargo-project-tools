use super::*;

use ratatui::backend::TestBackend;

fn drawn(view: &mut TextView, lines: &[Line], width: u16, height: u16) -> Vec<String> {
    let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| view.render(f, Rect::new(0, 0, width, height), lines))
        .unwrap();
    let buffer = terminal.backend().buffer();
    buffer
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
        .collect()
}

fn text(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|r| r.cells.iter().map(|c| c.ch).collect())
        .collect()
}

#[test]
fn wraps_at_spaces_and_mid_word_when_it_must() {
    let rows = wrap(
        &[
            Line::raw("one two three"),
            Line::raw(""),
            Line::raw("abcdefgh"),
        ],
        7,
    );
    assert_eq!(text(&rows), ["one two", "three", "", "abcdefg", "h"]);
    let joins: Vec<&str> = rows.iter().map(|r| r.join).collect();
    assert_eq!(joins, ["", " ", "\n", "\n", ""]);
}

#[test]
fn a_drag_selects_and_reads_back_across_wrapped_lines() {
    let mut view = TextView::default();
    let lines = [Line::raw("one two three"), Line::raw("four")];
    drawn(&mut view, &lines, 7, 5);
    assert!(view.press(Position::new(4, 0)));
    view.drag(Position::new(2, 3));
    assert_eq!(view.release().as_deref(), Some("two three\nfo"));
    // Dragging backwards selects the same.
    view.press(Position::new(2, 3));
    view.drag(Position::new(4, 0));
    assert_eq!(view.release().as_deref(), Some("two three\nfo"));
    // A click selects nothing.
    view.press(Position::new(1, 1));
    assert_eq!(view.release(), None);
    assert_eq!(view.selected(), None);
}

#[test]
fn a_selection_stays_inside_the_view() {
    let mut view = TextView::default();
    let lines = [Line::raw("abc"), Line::raw("def")];
    drawn(&mut view, &lines, 10, 4);
    assert!(!view.press(Position::new(20, 0)));
    view.press(Position::new(0, 0));
    // Past the right edge and below the text: to the end of the last line.
    view.drag(Position::new(30, 9));
    assert_eq!(view.release().as_deref(), Some("abc\ndef"));
}

#[test]
fn scrolling_stops_at_the_ends() {
    let mut view = TextView::default();
    let lines: Vec<Line> = (0..10).map(|i| Line::raw(format!("line {i}"))).collect();
    let screen = drawn(&mut view, &lines, 10, 4);
    assert_eq!(screen[0].trim_end(), "line 0");
    assert!(!view.more_above() && view.more_below());
    view.scroll(100);
    let screen = drawn(&mut view, &lines, 10, 4);
    assert_eq!(screen[3].trim_end(), "line 9");
    assert!(view.more_above() && !view.more_below());
    view.scroll(-view.page());
    view.scroll(-100);
    assert!(!view.more_above());
}

#[test]
fn a_hit_names_the_line_and_span() {
    let mut view = TextView::default();
    let lines = [Line::from(vec![Span::raw("ab "), Span::raw("[view]")])];
    drawn(&mut view, &lines, 20, 2);
    assert_eq!(view.hit(Position::new(4, 0)), Some((0, 1)));
    assert_eq!(view.hit(Position::new(0, 0)), Some((0, 0)));
    assert_eq!(view.hit(Position::new(15, 0)), None);
}

#[test]
fn osc52_is_base64() {
    assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64("ä€".as_bytes()), "w6Tigqw=");
}
