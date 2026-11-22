//! Keys and mouse on the setup screen.

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use super::app::{App, BUTTONS, Exit, Focus, RowKind};

impl App<'_> {
    pub fn handle(&mut self, event: Event) {
        // One slow step at a time.
        if self.pending.is_some() {
            return;
        }
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.key(key),
            Event::Mouse(mouse) => self.mouse(mouse),
            _ => {}
        }
    }

    fn key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.exit = Some(Exit::Cancel);
            return;
        }
        if self.chord {
            self.chord = false;
            match key.code {
                KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::Tree,
                KeyCode::Char('l') | KeyCode::Right => self.focus = Focus::Details,
                KeyCode::Char('j') | KeyCode::Down => self.focus = Focus::Buttons,
                _ => {}
            }
            return;
        }
        if self.dialog.is_some() {
            self.dialog_key(key);
            return;
        }
        if ctrl && key.code == KeyCode::Char('w') {
            self.chord = true;
            return;
        }
        if key.code == KeyCode::Char('v') {
            self.open_advisory(None);
            return;
        }
        match self.focus {
            Focus::Tree => self.tree_key(key),
            Focus::Details => self.details_key(key),
            Focus::Buttons => self.button_key(key),
        }
    }

    fn tree_key(&mut self, key: KeyEvent) {
        let visible = self.visible();
        let pos = visible
            .iter()
            .position(|&r| r == self.selected)
            .unwrap_or(0);
        let row = self.rows[self.selected].clone();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(&next) = visible.get(pos + 1) {
                    self.select(next);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if pos > 0 {
                    self.select(visible[pos - 1]);
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if row.folds && !row.expanded {
                    self.rows[self.selected].expanded = true;
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if row.folds && row.expanded {
                    self.rows[self.selected].expanded = false;
                } else if let Some(parent) = self.parent(self.selected) {
                    self.select(parent);
                }
            }
            KeyCode::Char(' ') if matches!(row.kind, RowKind::Feature(_) | RowKind::Default) => {
                self.activate()
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.activate(),
            KeyCode::Char('n') => self.next_finding(),
            _ => {}
        }
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let n = dialog.ids.len();
        let view = &mut dialog.view;
        match key.code {
            KeyCode::Esc if view.selected().is_some() => view.clear_selection(),
            KeyCode::Esc | KeyCode::Char('q') => self.dialog = None,
            KeyCode::Right | KeyCode::Char('l' | 'n') => {
                dialog.index = (dialog.index + 1) % n;
                dialog.view.reset();
            }
            KeyCode::Left | KeyCode::Char('h' | 'p') => {
                dialog.index = (dialog.index + n - 1) % n;
                dialog.view.reset();
            }
            KeyCode::Down | KeyCode::Char('j') => view.scroll(1),
            KeyCode::Up | KeyCode::Char('k') => view.scroll(-1),
            KeyCode::PageDown | KeyCode::Char(' ') => view.scroll(view.page()),
            KeyCode::PageUp => view.scroll(-view.page()),
            KeyCode::Char('y') => {
                if let Some(text) = view.selected() {
                    self.copy(&text);
                }
            }
            _ => {}
        }
    }

    fn details_key(&mut self, key: KeyEvent) {
        let count = self.detail.as_ref().map_or(0, |d| d.versions.len());
        let cursor = self.versions_state.selected();
        match key.code {
            KeyCode::PageDown => self.details.scroll(self.details.page()),
            KeyCode::PageUp => self.details.scroll(-self.details.page()),
            // Without a versions list, j and k scroll the text.
            KeyCode::Char('j') | KeyCode::Down if count == 0 => self.details.scroll(1),
            KeyCode::Char('k') | KeyCode::Up if count == 0 => self.details.scroll(-1),
            KeyCode::Char('y') => {
                if let Some(text) = self.details.selected() {
                    self.copy(&text);
                }
            }
            KeyCode::Esc if self.details.selected().is_some() => self.details.clear_selection(),
            KeyCode::Char('j') | KeyCode::Down if count > 0 => {
                let next = cursor.map_or(0, |c| (c + 1).min(count - 1));
                self.versions_state.select(Some(next));
            }
            KeyCode::Char('k') | KeyCode::Up if count > 0 => {
                let next = cursor.map_or(0, |c| c.saturating_sub(1));
                self.versions_state.select(Some(next));
            }
            KeyCode::Enter => self.use_version(),
            KeyCode::Esc | KeyCode::Char('h') => self.focus = Focus::Tree,
            _ => {}
        }
    }

    fn button_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => {
                self.button_cursor = self.button_cursor.saturating_sub(1)
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.button_cursor = (self.button_cursor + 1).min(BUTTONS.len() - 1)
            }
            KeyCode::Enter | KeyCode::Char(' ') => self.press(BUTTONS[self.button_cursor]),
            _ => {}
        }
    }

    fn dialog_mouse(&mut self, mouse: MouseEvent) {
        let at = Position::new(mouse.column, mouse.row);
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let view = &mut dialog.view;
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !dialog.area.contains(at) {
                    self.dialog = None;
                } else {
                    view.press(at);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => view.drag(at),
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(text) = view.release() {
                    self.copy(&text);
                }
            }
            MouseEventKind::ScrollDown => view.scroll(3),
            MouseEventKind::ScrollUp => view.scroll(-3),
            _ => {}
        }
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        if self.dialog.is_some() {
            self.dialog_mouse(mouse);
            return;
        }
        let at = Position::new(mouse.column, mouse.row);
        let in_tree = self.areas.tree.contains(at);
        let in_versions = self.areas.versions.contains(at);
        let in_text = self.details.area.contains(at);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if in_text {
                    self.focus = Focus::Details;
                    let view = self
                        .details
                        .hit(at)
                        .and_then(|hit| self.areas.views.iter().find(|(h, _)| *h == hit))
                        .map(|(_, id)| id.clone());
                    match view {
                        Some(id) => self.open_advisory(Some(&id)),
                        None => {
                            self.details.press(at);
                        }
                    }
                } else if in_tree {
                    self.focus = Focus::Tree;
                    let row = (mouse.row - self.areas.tree.y) as usize + self.tree_state.offset();
                    let Some(&r) = self.visible().get(row) else {
                        return;
                    };
                    let again = r == self.selected;
                    self.select(r);
                    let row = &self.rows[r];
                    match row.kind {
                        // A checkbox: a click toggles it.
                        RowKind::Feature(_) | RowKind::Default => self.activate(),
                        _ if again && row.folds => {
                            self.rows[r].expanded = !self.rows[r].expanded;
                        }
                        RowKind::Tool | RowKind::Dep(_) if again => self.activate(),
                        _ => {}
                    }
                } else if in_versions {
                    self.focus = Focus::Details;
                    let i =
                        (mouse.row - self.areas.versions.y) as usize + self.versions_state.offset();
                    let count = self.detail.as_ref().map_or(0, |d| d.versions.len());
                    if i < count {
                        if self.versions_state.selected() == Some(i) {
                            self.use_version();
                        } else {
                            self.versions_state.select(Some(i));
                        }
                    }
                } else if let Some(&(_, button)) =
                    self.areas.buttons.iter().find(|(r, _)| r.contains(at))
                {
                    self.button_cursor = BUTTONS.iter().position(|b| *b == button).unwrap_or(0);
                    self.press(button);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => self.details.drag(at),
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(text) = self.details.release() {
                    self.copy(&text);
                }
            }
            MouseEventKind::ScrollDown if in_text => self.details.scroll(3),
            MouseEventKind::ScrollUp if in_text => self.details.scroll(-3),
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if in_tree => {
                let code = if mouse.kind == MouseEventKind::ScrollDown {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                };
                self.tree_key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            _ => {}
        }
    }
}
