//! Drawing the setup screen. Every string from crates, the index or cargo goes through
//! `shown`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph};

use super::advisory;
use super::app::{App, BUTTONS, Focus, RowKind};
use super::text_view::TextView;
use crate::sanitize::shown;
use crate::setup::Finding;

impl App<'_> {
    pub fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let (built, vulns, warnings) = self.setup.counts();
        let red = |n: usize, c: Color| {
            if n > 0 {
                Style::default().fg(c).bold()
            } else {
                Style::default()
            }
        };
        let counts = Line::from(vec![
            Span::raw(format!(" {built} packages · ")),
            Span::styled(format!("{vulns} vulnerable"), red(vulns, Color::Red)),
            Span::raw(" · "),
            Span::styled(format!("{warnings} warnings"), red(warnings, Color::Yellow)),
            Span::raw(" "),
        ])
        .right_aligned();
        let title = format!(
            " Set up: {} {} ",
            shown(&self.setup.name),
            shown(&self.setup.checkout.version)
        );
        let outer = Block::bordered()
            .title(Line::from(title).bold())
            .title(counts);
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let [_, main, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(inner);
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(40), Constraint::Min(0)]).areas(main);
        self.render_tree(frame, left);
        self.render_details(frame, right);
        self.render_bottom(frame, bottom);
        self.render_dialog(frame, main);
    }

    /// The advisory dialog, over the middle of `area`.
    fn render_dialog(&mut self, frame: &mut Frame, area: Rect) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let id = &dialog.ids[dialog.index];
        let lines = self
            .setup
            .advisory(id)
            .map(advisory::lines)
            .unwrap_or_default();
        let [_, middle, _] = Layout::vertical([
            Constraint::Percentage(5),
            Constraint::Percentage(90),
            Constraint::Percentage(5),
        ])
        .areas(area);
        let [_, middle, _] = Layout::horizontal([
            Constraint::Percentage(8),
            Constraint::Percentage(84),
            Constraint::Percentage(8),
        ])
        .areas(middle);
        dialog.area = middle;
        let mut title = format!(" {}", shown(id));
        if dialog.ids.len() > 1 {
            title.push_str(&format!(
                " ({}/{}) · ←/→ others",
                dialog.index + 1,
                dialog.ids.len()
            ));
        }
        title.push_str(" · Esc closes ");
        let block = Block::bordered()
            .title(Line::from(title).bold())
            .title_bottom(scroll_hint(&dialog.view))
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(middle);
        frame.render_widget(Clear, middle);
        frame.render_widget(block, middle);
        dialog.view.render(frame, inner, &lines);
    }

    fn border(&self, focus: Focus) -> Style {
        if self.focus == focus {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    }

    fn findings_of(&self, i: usize) -> Option<&Vec<Finding>> {
        let d = &self.setup.tree[i];
        self.setup
            .findings
            .get(&(d.name.clone(), d.version.clone()))
    }

    fn render_tree(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .title(" Tool, features and dependencies ")
            .border_style(self.border(Focus::Tree));
        self.areas.tree = block.inner(area);
        let visible = self.visible();
        let items: Vec<ListItem> = visible
            .iter()
            .map(|&r| {
                let row = &self.rows[r];
                let fold = match (row.folds, row.expanded) {
                    (true, true) => "▾ ",
                    (true, false) => "▸ ",
                    _ => "  ",
                };
                let mut spans = vec![Span::raw(format!("{}{fold}", "  ".repeat(row.depth)))];
                match &row.kind {
                    RowKind::Header(s) => spans.push(Span::raw(s.label()).bold()),
                    RowKind::Tool => {
                        let mut label = format!(
                            "{} {}",
                            shown(&self.setup.name),
                            shown(&self.setup.checkout.version)
                        );
                        if let Some(c) = &self.setup.checkout.commit {
                            label.push_str(&format!(" @ {}", shown(&c[..c.len().min(10)])));
                        }
                        spans.push(Span::raw(label));
                        if let Ok(v) = semver::Version::parse(&self.setup.checkout.version) {
                            spans.extend(was(&self.setup.was(&self.setup.name, &v)));
                        }
                    }
                    RowKind::Default => {
                        let mark = if self.setup.default_features {
                            "[x]"
                        } else {
                            "[ ]"
                        };
                        spans.push(Span::raw(format!("{mark} default")));
                    }
                    RowKind::Feature(f) => {
                        let (chosen, by_default) = self.feature_on(f);
                        let mark = match (chosen, by_default) {
                            (true, _) => "[x]",
                            (false, true) => "[·]",
                            _ => "[ ]",
                        };
                        spans.push(Span::raw(format!("{mark} {}", shown(f))));
                        if by_default && !chosen {
                            spans.push(Span::styled("  by default", Style::default().dim()));
                        }
                    }
                    RowKind::Dep(i) => {
                        let d = &self.setup.tree[*i];
                        let grouped = self.setup.targets.as_ref().is_some_and(|t| t.len() > 1);
                        if d.depth == 0 && grouped {
                            // The groups' roots: the tool is in the Tool section.
                            let label = match &d.target {
                                None => "All targets".to_string(),
                                Some(t) => format!("Only {}", shown(t)),
                            };
                            spans.push(Span::raw(label).bold());
                        } else if d.shared {
                            // On the way to what the target adds; listed under All targets.
                            spans.push(Span::styled(
                                format!("{} {}", shown(&d.name), d.version),
                                Style::default().fg(Color::DarkGray),
                            ));
                        } else {
                            spans.push(Span::raw(format!("{} {}", shown(&d.name), d.version)));
                        }
                        if d.build {
                            spans.push(Span::styled(" build", Style::default().dim()));
                        }
                        if d.depth > 0 {
                            spans.extend(was(&self.setup.was(&d.name, &d.version)));
                        }
                        let open: Vec<&Finding> = self
                            .findings_of(*i)
                            .into_iter()
                            .flatten()
                            .filter(|f| !f.ignored)
                            .collect();
                        if open.iter().any(|f| f.is_vulnerability()) {
                            spans.push(Span::styled(" ⚠", Style::default().fg(Color::Red).bold()));
                        } else if !open.is_empty() {
                            spans.push(Span::styled(
                                " !",
                                Style::default().fg(Color::Yellow).bold(),
                            ));
                        }
                        if d.repeat {
                            spans.push(Span::styled(" (*)", Style::default().dim()));
                        }
                    }
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        self.tree_state
            .select(visible.iter().position(|&r| r == self.selected));
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, area, &mut self.tree_state);
    }

    /// The text part of the details for the selected row. Notes where its `[view]` links
    /// are.
    fn detail_lines(&mut self) -> Vec<Line<'static>> {
        self.areas.views.clear();
        let mut lines = Vec::new();
        let Some(row) = self.rows.get(self.selected) else {
            return lines;
        };
        let dim = Style::default().dim();
        match &row.kind {
            RowKind::Header(_) => {
                lines.push(Line::raw("Space toggles a feature; Enter on a package lists its versions and why it's built."));
                lines.push(Line::raw(
                    "n jumps to the next package with an advisory; v or [view] shows it.",
                ));
                lines.push(Line::raw("Drag over text here or in an advisory to copy it; y copies it again. PgUp/PgDn scroll."));
                lines.push(Line::styled(
                    "Shift- or Option-drag gives the terminal's own selection, across the whole screen.",
                    dim,
                ));
            }
            RowKind::Default => {
                let list = self
                    .setup
                    .features
                    .get("default")
                    .map(|d| d.join(", "))
                    .unwrap_or_default();
                lines.push(Line::raw(format!("Default features: {}", shown(&list))));
                lines.push(Line::styled("Space turns them on or off.", dim));
            }
            RowKind::Feature(f) => {
                let enables = self
                    .setup
                    .features
                    .get(f)
                    .map(|e| e.join(", "))
                    .unwrap_or_default();
                lines.push(Line::raw(format!("Feature {}", shown(f))));
                lines.push(Line::raw(format!(
                    "Enables: {}",
                    if enables.is_empty() {
                        "nothing else".into()
                    } else {
                        shown(&enables)
                    }
                )));
                lines.push(Line::styled("Space turns it on or off.", dim));
            }
            RowKind::Tool | RowKind::Dep(_) => {
                let Some((name, version, tool)) = self.selected_package() else {
                    return lines;
                };
                lines.push(Line::raw(format!("{} {version}", shown(&name))).bold());
                if let RowKind::Dep(i) = row.kind {
                    let d = &self.setup.tree[i];
                    if !d.source.is_empty() {
                        lines.push(Line::styled(shown(&d.source), dim));
                    }
                    if d.depth > 0 {
                        if d.build {
                            lines.push(Line::styled(
                                "A build dependency of the package above it.",
                                dim,
                            ));
                        }
                        match (&d.target, d.shared) {
                            (Some(t), true) => lines.push(Line::styled(
                                format!(
                                    "Built for every target, here on the way to what {} adds.",
                                    shown(t)
                                ),
                                dim,
                            )),
                            (Some(t), false) => lines.push(Line::styled(
                                format!("Pulled in this way only for {}.", shown(t)),
                                dim,
                            )),
                            (None, _) => {}
                        }
                    }
                }
                let findings = self
                    .setup
                    .findings
                    .get(&(name.clone(), version.clone()))
                    .cloned()
                    .unwrap_or_default();
                for f in &findings {
                    let style = match (f.ignored, f.is_vulnerability()) {
                        (true, _) => dim,
                        (false, true) => Style::default().fg(Color::Red),
                        (false, false) => Style::default().fg(Color::Yellow),
                    };
                    let mut text = format!("{} {}: {}", f.kind, shown(&f.id), shown(&f.title));
                    if !f.patched.is_empty() {
                        let patched: Vec<String> =
                            f.patched.iter().map(ToString::to_string).collect();
                        text.push_str(&format!(" (patched: {})", patched.join(", ")));
                    }
                    if f.ignored {
                        text.push_str(" (ignored)");
                    }
                    let mut line = Line::from(Span::styled(text, style));
                    if self.setup.advisory(&f.id).is_some() {
                        line.push_span(Span::raw(" "));
                        self.areas.views.push(((lines.len(), 2), f.id.clone()));
                        line.push_span(Span::styled(
                            "[view]",
                            Style::default().fg(Color::Cyan).underlined(),
                        ));
                    }
                    lines.push(line);
                }
                if tool && self.setup.git {
                    lines.push(Line::raw(""));
                    lines.push(Line::raw("Enter follows the branch or tag again."));
                } else if self.detail.is_none() {
                    lines.push(Line::raw(""));
                    lines.push(Line::styled(
                        "Enter lists its versions and why it's built.",
                        dim,
                    ));
                }
                if let Some(d) = &self.detail
                    && !d.why.is_empty()
                {
                    lines.push(Line::raw(""));
                    lines.push(Line::raw("Why it's built:").bold());
                    for l in &d.why {
                        lines.push(Line::raw(shown(l)));
                    }
                }
            }
        }
        lines
    }

    fn render_details(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .title(" Details ")
            .title_bottom(scroll_hint(&self.details))
            .border_style(self.border(Focus::Details));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let count = self.detail.as_ref().map_or(0, |d| d.versions.len());
        let list_height = if count > 0 {
            (count as u16 + 2).min(12)
        } else {
            0
        };
        let [text, versions] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(list_height)]).areas(inner);
        let lines = self.detail_lines();
        self.details.render(frame, text, &lines);
        self.areas.versions = Rect::default();
        let Some(d) = &self.detail else {
            return;
        };
        if d.versions.is_empty() {
            return;
        }
        let items: Vec<ListItem> = d
            .versions
            .iter()
            .map(|c| {
                let mut spans = vec![Span::raw(c.version.to_string())];
                if c.prerelease {
                    spans.push(Span::styled(
                        "  pre-release",
                        Style::default().fg(Color::Yellow),
                    ));
                }
                if c.current {
                    spans.push(Span::styled("  current", Style::default().dim()));
                }
                if c.original {
                    spans.push(Span::styled(
                        "  original",
                        Style::default().fg(Color::Magenta),
                    ));
                }
                if c.suggested {
                    spans.push(Span::styled(
                        "  ★ fixes its advisories",
                        Style::default().fg(Color::Green).bold(),
                    ));
                }
                if !c.compatible {
                    spans.push(Span::styled(
                        "  not semver-compatible",
                        Style::default().dim(),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let block = Block::bordered()
            .title(" Versions: Enter uses one ")
            .border_style(self.border(Focus::Details));
        self.areas.versions = block.inner(versions);
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, versions, &mut self.versions_state);
    }

    fn render_bottom(&mut self, frame: &mut Frame, area: Rect) {
        self.areas.buttons.clear();
        let [message, row] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
        let labels: Vec<String> = BUTTONS
            .iter()
            .map(|b| format!("[ {} ]", b.label()))
            .collect();
        let total: u16 = labels.iter().map(|l| l.chars().count() as u16).sum::<u16>()
            + labels.len().saturating_sub(1) as u16;
        let focused = self.focus == Focus::Buttons;
        let mut x = row.x + row.width.saturating_sub(total) / 2;
        for (i, (button, label)) in BUTTONS.iter().zip(&labels).enumerate() {
            let width = label.chars().count() as u16;
            let rect = Rect::new(x, row.y, width, 1).intersection(row);
            let mut style = Style::default().bold();
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
            frame.render_widget(
                Paragraph::new(format!(" {}", shown(text))).style(style),
                message,
            );
        }
    }
}

/// `  (was 1.2.3)`, for a package moved from its original version.
/// `↑`/`↓` when there's more of the text that way.
fn scroll_hint(view: &TextView) -> Line<'static> {
    let hint = match (view.more_above(), view.more_below()) {
        (true, true) => " ↑↓ more ",
        (true, false) => " ↑ more ",
        (false, true) => " ↓ more ",
        (false, false) => "",
    };
    Line::styled(hint, Style::default().dim()).right_aligned()
}

fn was(versions: &[semver::Version]) -> Option<Span<'static>> {
    if versions.is_empty() {
        return None;
    }
    let list: Vec<String> = versions.iter().map(ToString::to_string).collect();
    Some(Span::styled(
        format!("  (was {})", list.join(", ")),
        Style::default().fg(Color::Magenta),
    ))
}
