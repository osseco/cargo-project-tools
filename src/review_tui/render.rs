//! Drawing the review screen. Every string from the reviewed code goes through `shown`
//! (tree labels are sanitized when the tree is built).

use edtui::{EditorTheme, EditorView, LineNumbers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph, Widget, Wrap};

use super::app::{App, BUTTONS, Button, Focus, OpenContent, RADIOS};
use super::text::{Warn, shown};
use super::tree::NodeKind;
use crate::review::Kind;

impl App {
    pub(super) fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [approved, overridden, stripped, undecided] = self.counts();
        let undecided_style = if undecided > 0 {
            Style::default().fg(Color::Yellow).bold()
        } else {
            Style::default()
        };
        let counts = Line::from(vec![
            Span::raw(format!(
                " {approved} approved · {overridden} override · {stripped} stripped · "
            )),
            Span::styled(format!("{undecided} undecided"), undecided_style),
            Span::raw(" "),
        ])
        .right_aligned();
        let outer = Block::bordered()
            .title(Line::from(format!(" Build review: {} ", shown(&self.tool))).bold())
            .title(counts);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        // A blank row under the title, and the message above the buttons.
        let [_, main, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(inner);
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(35), Constraint::Min(0)]).areas(main);
        self.render_tree(frame, left);
        self.render_right(frame, right);
        self.render_bottom(frame, bottom);
        if self.dialog.is_some() {
            self.render_dialog(frame, area);
        }
    }

    fn border(&self, focus: Focus) -> Style {
        if self.focus == focus && self.dialog.is_none() {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    }

    fn render_tree(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .title(" Build-time code ")
            .border_style(self.border(Focus::Tree));
        self.areas.tree = block.inner(area);
        let visible = self.visible();
        let rows: Vec<ListItem> = visible
            .iter()
            .map(|&i| {
                let n = &self.nodes[i];
                let fold = match (n.folds(), n.expanded) {
                    (true, true) => "▾ ",
                    (true, false) => "▸ ",
                    _ => "  ",
                };
                let mut spans = vec![Span::raw(format!("{}{fold}", "  ".repeat(n.depth)))];
                let is_item = matches!(n.kind, NodeKind::ItemFile(_) | NodeKind::ItemDir);
                if let (true, Some(item)) = (is_item, n.item) {
                    let style = match self.choice(item) {
                        None => Style::default().fg(Color::Yellow),
                        Some(_) => Style::default().fg(Color::Green),
                    };
                    spans.push(Span::styled(format!("{} ", self.glyph(item)), style));
                    spans.push(Span::raw(n.label.clone()));
                    if let Some(prev) = self.previous[item] {
                        spans.push(Span::styled(
                            format!("  changed (was {})", prev.as_str()),
                            Style::default().fg(Color::Magenta),
                        ));
                    }
                } else if n.kind == NodeKind::Group {
                    spans.push(Span::raw(n.label.clone()).bold());
                } else {
                    spans.push(Span::raw(n.label.clone()));
                }
                if n.warn != Warn::None {
                    spans.push(Span::styled(n.warn.mark(), Style::default().fg(Color::Red)));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let pos = visible.iter().position(|&n| n == self.selected);
        self.tree_state.select(pos);
        let list = List::new(rows)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, area, &mut self.tree_state);
    }

    fn render_right(&mut self, frame: &mut Frame, area: Rect) {
        let warning = self.open.as_ref().and_then(|o| match &o.content {
            OpenContent::Text { replaced, .. } if *replaced > 0 => Some(format!(
                " This file has {replaced} characters that can't be shown as they are; they're \
                 displayed as \\x.. or <U+..>. It can't be edited here: Override writes a \
                 commented-out copy for you to edit yourself."
            )),
            _ => None,
        });
        let banner_height = if warning.is_some() { 2 } else { 0 };
        let [radios, _, banner, editor] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(banner_height),
            Constraint::Min(0),
        ])
        .areas(area);
        self.render_radios(frame, radios);
        if let Some(text) = warning {
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(Color::White).bg(Color::Red)),
                banner,
            );
        }
        self.areas.editor = editor;
        let border = self.border(Focus::Editor);
        let focused = self.focus == Focus::Editor && self.dialog.is_none();
        let editable = self.open.as_ref().is_some_and(|o| self.editable(o));
        let modified = self.modified();
        let Some(open) = &mut self.open else {
            let text = match self.selected_item() {
                Some(i) if self.items[i].kind == Kind::ProcMacro => {
                    "A proc macro: its whole crate runs in the compiler. Expand it in the tree \
                     to read its files."
                }
                Some(_) => "Select a file in the tree to read it.",
                None => "Select a piece of build-time code in the tree.",
            };
            frame.render_widget(
                Paragraph::new(text)
                    .wrap(Wrap { trim: true })
                    .block(Block::bordered().border_style(border)),
                editor,
            );
            return;
        };
        let what = if open.showing_override {
            "override"
        } else {
            "shipped"
        };
        let mode = match (editable, modified) {
            (true, true) => "editing, unsaved",
            (true, false) => "editable",
            (false, _) => "view only",
        };
        let item = &self.items[open.item];
        let name = match (&self.files[open.item], open.showing_override) {
            (Some(f), true) => f.clone(),
            _ => {
                let rel = open.path.strip_prefix(&item.base).unwrap_or(&open.path);
                match item.kind {
                    Kind::CargoConfig => rel.display().to_string(),
                    _ => format!("{}@{}/{}", item.package, item.version, rel.display()),
                }
            }
        };
        let title = format!(" {} ({what}, {mode}) ", shown(&name));
        let block = Block::bordered().title(title).border_style(border);
        match &mut open.content {
            OpenContent::Text { state, rows, .. } => {
                // Only the rows around the view: edtui looks highlights up per character.
                let height = editor.height as usize;
                let top = state.viewport_offset().1.saturating_sub(height);
                let end = (top + 3 * height).min(rows.len());
                state.set_highlights(rows.get(top..end).unwrap_or(&[]).concat());
                // The terminal's own colours, so highlights (foreground only) match the rest.
                // Unfocused, the cursor style is empty, which leaves its cell as drawn.
                let cursor = if focused {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let theme = EditorTheme::default()
                    .base(Style::default())
                    .line_numbers_style(Style::default().fg(Color::DarkGray))
                    .cursor_style(cursor)
                    .block(block);
                EditorView::new(state)
                    .theme(theme)
                    .wrap(false)
                    .line_numbers(LineNumbers::Absolute)
                    .render(editor, frame.buffer_mut());
            }
            OpenContent::Binary(size) => frame.render_widget(
                Paragraph::new(format!("Binary file, {size} bytes; not shown.")).block(block),
                editor,
            ),
            OpenContent::Link(target) => frame.render_widget(
                Paragraph::new(format!(
                    "A link to {}; not followed.",
                    shown(&target.to_string_lossy())
                ))
                .block(block),
                editor,
            ),
        }
    }

    fn render_radios(&mut self, frame: &mut Frame, area: Rect) {
        self.areas.radios.clear();
        let Some(item) = self.selected_item() else {
            frame.render_widget(
                Paragraph::new(" Select a piece to decide on it.").dim(),
                area,
            );
            return;
        };
        let focused = self.focus == Focus::Radios && self.dialog.is_none();
        let mut spans = vec![Span::raw(" ")];
        let mut x = area.x + 1;
        for (i, action) in RADIOS.iter().enumerate() {
            let mark = if self.radio[item] == Some(*action) {
                "(•)"
            } else {
                "( )"
            };
            let label = format!("{mark} {}", capitalized(action.as_str()));
            let mut style = if self.allowed(item, *action) {
                Style::default()
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            if focused && i == self.radio_cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            let width = label.chars().count() as u16;
            self.areas
                .radios
                .push((Rect::new(x, area.y, width, 1).intersection(area), *action));
            x += width + 2;
            spans.push(Span::styled(label, style));
            spans.push(Span::raw("  "));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn render_bottom(&mut self, frame: &mut Frame, area: Rect) {
        self.areas.buttons.clear();
        let [message, row] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
        let save_enabled = self.modified();
        let labels: Vec<String> = BUTTONS
            .iter()
            .map(|b| format!("[ {} ]", b.label()))
            .collect();
        let total: u16 = labels.iter().map(|l| l.chars().count() as u16).sum::<u16>()
            + labels.len().saturating_sub(1) as u16;
        let focused = self.focus == Focus::Buttons && self.dialog.is_none();
        let mut x = row.x + row.width.saturating_sub(total) / 2;
        for (i, (button, label)) in BUTTONS.iter().zip(&labels).enumerate() {
            let width = label.chars().count() as u16;
            let rect = Rect::new(x, row.y, width, 1).intersection(row);
            let mut style = Style::default().bold();
            if *button == Button::Save && !save_enabled {
                style = Style::default().add_modifier(Modifier::DIM);
            }
            if focused && i == self.button_cursor {
                style = style.add_modifier(Modifier::REVERSED);
            }
            frame.render_widget(Paragraph::new(label.as_str()).style(style), rect);
            self.areas.buttons.push((rect, *button));
            x += width + 1;
        }
        if let Some((text, error)) = &self.message {
            let style = if *error {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::Green)
            };
            frame.render_widget(Paragraph::new(format!(" {text}")).style(style), message);
        }
    }

    fn render_dialog(&mut self, frame: &mut Frame, area: Rect) {
        let Some(dialog) = &self.dialog else {
            return;
        };
        let (ask, cursor) = (dialog.ask, dialog.cursor);
        let width = 60.min(area.width);
        let height = 5.min(area.height);
        let rect = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        let block = Block::bordered()
            .title(ask.title())
            .border_style(Style::default().fg(Color::Yellow));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        frame.render_widget(
            Paragraph::new(format!(" {}", ask.text())),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        self.areas.answers.clear();
        let mut x = inner.x + 1;
        let y = inner.y + inner.height.saturating_sub(1);
        for (i, answer) in ask.answers().iter().enumerate() {
            let label = format!("[ {} ]", answer.label());
            let width = label.chars().count() as u16;
            let r = Rect::new(x, y, width, 1).intersection(inner);
            let style = if i == cursor {
                Style::default().bold().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().bold()
            };
            frame.render_widget(Paragraph::new(label).style(style), r);
            self.areas.answers.push((r, *answer));
            x += width + 2;
        }
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}
