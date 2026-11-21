//! Keys and mouse on the review screen.

use edtui::EditorMode;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use super::app::{Answer, App, BUTTONS, Exit, Focus, Open, OpenContent, RADIOS, Then};
use crate::highlight;

impl App {
    pub(super) fn handle(&mut self, event: Event) {
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
        if self.dialog.is_some() {
            self.dialog_key(key);
            return;
        }
        if self.chord {
            self.chord = false;
            match key.code {
                KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::Tree,
                KeyCode::Char('l') | KeyCode::Right => self.focus = Focus::Editor,
                KeyCode::Char('k') | KeyCode::Up => self.focus = Focus::Radios,
                KeyCode::Char('j') | KeyCode::Down => self.focus = Focus::Buttons,
                _ => {}
            }
            return;
        }
        if ctrl && key.code == KeyCode::Char('w') {
            self.chord = true;
            return;
        }
        match self.focus {
            Focus::Tree => self.tree_key(key),
            Focus::Radios => self.radio_key(key),
            Focus::Editor => self.editor_key(key),
            Focus::Buttons => self.button_key(key),
        }
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        let answers = dialog.ask.answers();
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => dialog.cursor = dialog.cursor.saturating_sub(1),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Tab => {
                dialog.cursor = (dialog.cursor + 1).min(answers.len() - 1)
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let answer = answers[dialog.cursor];
                self.answer(answer);
            }
            KeyCode::Esc => self.answer(Answer::Stay),
            _ => {}
        }
    }

    pub(super) fn tree_key(&mut self, key: KeyEvent) {
        let visible = self.visible();
        let pos = visible
            .iter()
            .position(|&n| n == self.selected)
            .unwrap_or(0);
        let node = &self.nodes[self.selected];
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(&next) = visible.get(pos + 1) {
                    self.guarded(Then::Select(next));
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if pos > 0 {
                    self.guarded(Then::Select(visible[pos - 1]));
                }
            }
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
                if node.folds() && !node.expanded {
                    self.nodes[self.selected].expanded = true;
                } else if self.open.is_some() {
                    self.focus = Focus::Editor;
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if node.folds() && node.expanded {
                    self.nodes[self.selected].expanded = false;
                } else if let Some(parent) = super::tree::parent(&self.nodes, self.selected) {
                    self.guarded(Then::Select(parent));
                }
            }
            _ => {}
        }
    }

    fn radio_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => {
                self.radio_cursor = self.radio_cursor.saturating_sub(1)
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.radio_cursor = (self.radio_cursor + 1).min(RADIOS.len() - 1)
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(item) = self.selected_item() {
                    self.guarded(Then::Radio(item, RADIOS[self.radio_cursor]));
                }
            }
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

    fn editor_key(&mut self, key: KeyEvent) {
        // edtui panics on keys it doesn't know.
        let known = matches!(
            key.code,
            KeyCode::Char(_)
                | KeyCode::Enter
                | KeyCode::Esc
                | KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::Tab
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
        );
        if !known {
            return;
        }
        let editable = self.open.as_ref().is_some_and(|o| self.editable(o));
        let Some(Open {
            path,
            content: OpenContent::Text { state, rows, .. },
            ..
        }) = &mut self.open
        else {
            return;
        };
        let before = (state.lines.clone(), state.cursor);
        self.keys.on_key_event(key, state);
        if !editable {
            // View only: undo whatever the key changed, and stay out of insert mode.
            if state.lines != before.0 {
                state.lines = before.0;
                state.cursor = before.1;
            }
            if state.mode == EditorMode::Insert {
                state.mode = EditorMode::Normal;
            }
        } else if state.lines != before.0 {
            *rows = highlight::rows(path, &state.lines.to_string());
        }
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        let at = Position::new(mouse.column, mouse.row);
        if self.dialog.is_some() {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some(&(_, answer)) = self.areas.answers.iter().find(|(r, _)| r.contains(at))
            {
                self.answer(answer);
            }
            return;
        }
        let in_tree = self.areas.tree.contains(at);
        let in_editor = self.areas.editor.contains(at);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if in_tree {
                    self.focus = Focus::Tree;
                    let row = (mouse.row - self.areas.tree.y) as usize + self.tree_state.offset();
                    if let Some(&node) = self.visible().get(row) {
                        if node == self.selected && self.nodes[node].folds() {
                            self.nodes[node].expanded = !self.nodes[node].expanded;
                        } else {
                            self.guarded(Then::Select(node));
                        }
                    }
                } else if let Some(&(_, action)) =
                    self.areas.radios.iter().find(|(r, _)| r.contains(at))
                {
                    self.focus = Focus::Radios;
                    if let Some(item) = self.selected_item() {
                        self.radio_cursor = RADIOS.iter().position(|r| *r == action).unwrap_or(0);
                        self.guarded(Then::Radio(item, action));
                    }
                } else if let Some(&(_, button)) =
                    self.areas.buttons.iter().find(|(r, _)| r.contains(at))
                {
                    self.button_cursor = BUTTONS.iter().position(|b| *b == button).unwrap_or(0);
                    self.press(button);
                } else if in_editor {
                    self.focus = Focus::Editor;
                    self.editor_mouse(mouse);
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp if in_tree => {
                let code = if mouse.kind == MouseEventKind::ScrollDown {
                    KeyCode::Down
                } else {
                    KeyCode::Up
                };
                self.tree_key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            _ if in_editor => self.editor_mouse(mouse),
            _ => {}
        }
    }

    fn editor_mouse(&mut self, mouse: MouseEvent) {
        if let Some(Open {
            content: OpenContent::Text { state, .. },
            ..
        }) = &mut self.open
        {
            self.keys.on_mouse_event(mouse, state);
        }
    }
}
