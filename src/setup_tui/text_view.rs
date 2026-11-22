//! Text that scrolls, and that a mouse drag selects and copies. The terminal's own selection
//! can't be used while the screen captures the mouse, and it would run across both panes.
//! The text is wrapped here, so each screen row is one row of `rows`.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// A character as drawn, with the line and span of the text it came from.
#[derive(Debug, Clone)]
struct Cell {
    ch: char,
    style: Style,
    line: usize,
    span: usize,
}

#[derive(Debug, Clone, Default)]
struct Row {
    cells: Vec<Cell>,
    /// What stands between this row and the one before it in the text: a new line, or for
    /// a wrapped line, the space the wrap took (or nothing).
    join: &'static str,
}

/// A place in the wrapped text: row, then character.
type At = (usize, usize);

#[derive(Default)]
pub(super) struct TextView {
    rows: Vec<Row>,
    /// Where it was drawn last; clicks are matched against it.
    pub(super) area: Rect,
    offset: usize,
    /// Anchor and cursor.
    selection: Option<(At, At)>,
    dragging: bool,
}

impl TextView {
    /// Wrap `lines` to `area` and draw what's in view, the selection reversed.
    pub(super) fn render(&mut self, frame: &mut Frame, area: Rect, lines: &[Line]) {
        self.area = area;
        self.rows = wrap(lines, area.width as usize);
        self.offset = self.offset.min(self.max_offset());
        if let Some((a, b)) = self.selection
            && (a.0 >= self.rows.len() || b.0 >= self.rows.len())
        {
            self.selection = None;
        }
        let range = self.range();
        let shown: Vec<Line> = self
            .rows
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(area.height as usize)
            .map(|(r, row)| {
                let mut spans: Vec<Span> = Vec::new();
                let mut text = String::new();
                let mut style = None;
                for (c, cell) in row.cells.iter().enumerate() {
                    let mut s = cell.style;
                    if range.is_some_and(|(a, b)| a <= (r, c) && (r, c) < b) {
                        s = s.add_modifier(Modifier::REVERSED);
                    }
                    if style.is_some_and(|st| st != s) {
                        spans.push(Span::styled(std::mem::take(&mut text), style.unwrap()));
                    }
                    style = Some(s);
                    text.push(cell.ch);
                }
                if let Some(s) = style {
                    spans.push(Span::styled(text, s));
                }
                Line::from(spans)
            })
            .collect();
        frame.render_widget(Paragraph::new(shown), area);
    }

    fn max_offset(&self) -> usize {
        self.rows.len().saturating_sub(self.area.height as usize)
    }

    pub(super) fn more_above(&self) -> bool {
        self.offset > 0
    }

    pub(super) fn more_below(&self) -> bool {
        self.offset < self.max_offset()
    }

    pub(super) fn scroll(&mut self, by: isize) {
        self.offset = self.offset.saturating_add_signed(by).min(self.max_offset());
    }

    pub(super) fn page(&self) -> isize {
        self.area.height.max(2) as isize - 1
    }

    /// Back to the top, nothing selected: for other text.
    pub(super) fn reset(&mut self) {
        self.offset = 0;
        self.selection = None;
        self.dragging = false;
    }

    pub(super) fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// The text place under `pos`, kept inside the drawn area and the text.
    fn at(&self, pos: Position) -> At {
        let area = self.area;
        let y = pos.y.clamp(area.y, area.bottom().saturating_sub(1)) - area.y;
        let x = pos.x.clamp(area.x, area.right()) - area.x;
        let row = (self.offset + y as usize).min(self.rows.len().saturating_sub(1));
        let len = self.rows.get(row).map_or(0, |r| r.cells.len());
        (row, (x as usize).min(len))
    }

    /// The line and span of the text under `pos`, if a character is there.
    pub(super) fn hit(&self, pos: Position) -> Option<(usize, usize)> {
        if !self.area.contains(pos) {
            return None;
        }
        let row = self.offset + (pos.y - self.area.y) as usize;
        let cell = self
            .rows
            .get(row)?
            .cells
            .get((pos.x - self.area.x) as usize)?;
        Some((cell.line, cell.span))
    }

    /// A press: starts a selection when it's inside the text.
    pub(super) fn press(&mut self, pos: Position) -> bool {
        if !self.area.contains(pos) {
            return false;
        }
        let at = self.at(pos);
        self.selection = Some((at, at));
        self.dragging = true;
        true
    }

    /// A drag: moves the selection's end, kept in the text; past an edge it scrolls.
    pub(super) fn drag(&mut self, pos: Position) {
        if !self.dragging {
            return;
        }
        if pos.y < self.area.y {
            self.scroll(-1);
        } else if pos.y >= self.area.bottom() {
            self.scroll(1);
        }
        let at = self.at(pos);
        if let Some((_, end)) = &mut self.selection {
            *end = at;
        }
    }

    /// A release: the selected text, if any was.
    pub(super) fn release(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.dragging) {
            return None;
        }
        let text = self.selected();
        if text.is_none() {
            self.selection = None;
        }
        text
    }

    /// Start and end, in order; `None` when nothing is selected.
    fn range(&self) -> Option<(At, At)> {
        let (a, b) = self.selection?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        (a != b).then_some((a, b))
    }

    /// The selected text as it reads, wrapped lines joined back.
    pub(super) fn selected(&self) -> Option<String> {
        let ((r0, c0), (r1, c1)) = self.range()?;
        let mut out = String::new();
        for r in r0..=r1 {
            let row = &self.rows[r];
            if r > r0 {
                out.push_str(row.join);
            }
            let from = if r == r0 { c0 } else { 0 };
            let to = if r == r1 { c1 } else { row.cells.len() };
            out.extend(row.cells[from.min(to)..to].iter().map(|c| c.ch));
        }
        Some(out)
    }
}

/// Wrap at spaces where it can, else mid-word, to `width` characters.
fn wrap(lines: &[Line], width: usize) -> Vec<Row> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for (l, line) in lines.iter().enumerate() {
        let mut cells: Vec<Cell> = Vec::new();
        for (s, span) in line.spans.iter().enumerate() {
            let style = line.style.patch(span.style);
            cells.extend(span.content.chars().map(|ch| Cell {
                ch,
                style,
                line: l,
                span: s,
            }));
        }
        let mut join = "\n";
        loop {
            if cells.len() <= width {
                rows.push(Row { cells, join });
                break;
            }
            match cells[1..=width].iter().rposition(|c| c.ch == ' ') {
                Some(i) => {
                    let rest = cells.split_off(i + 1);
                    rows.push(Row { cells, join });
                    // The space the wrap takes.
                    cells = rest[1..].to_vec();
                    join = " ";
                }
                None => {
                    let rest = cells.split_off(width);
                    rows.push(Row { cells, join });
                    cells = rest;
                    join = "";
                }
            }
        }
    }
    if let Some(first) = rows.first_mut() {
        first.join = "";
    }
    rows
}

/// The OSC 52 sequence that puts `text` on the terminal's clipboard; it passes through ssh,
/// and through tmux with `set-clipboard on`.
pub(super) fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "text_view_test.rs"]
mod tests;
