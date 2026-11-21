//! The review screen's state and what its actions do. Input and drawing are in `input` and
//! `render`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use edtui::{EditorEventHandler, EditorState, Highlight, Lines};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use super::Choice;
use super::text::{self, Content, commented_copy, override_rel, read_content, shown};
use super::tree::{self, Node, NodeKind};
use crate::highlight;
use crate::review::{Action, Item, Kind, Pending};
use crate::source::registry::sha256_hex;

/// The file in the editor.
pub(super) struct Open {
    pub(super) item: usize,
    /// What's shown: the shipped file or its override.
    pub(super) path: PathBuf,
    /// The piece's own file, the one an override replaces.
    pub(super) own: bool,
    pub(super) showing_override: bool,
    /// sha256 of the override file as loaded or last saved, to notice changes on disk.
    pub(super) on_disk: Option<String>,
    pub(super) content: OpenContent,
}

pub(super) enum OpenContent {
    Text {
        state: Box<EditorState>,
        /// The text as loaded or last saved, to tell edits apart.
        baseline: String,
        replaced: usize,
        rows: Vec<Vec<Highlight>>,
    },
    Binary(u64),
    Link(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Tree,
    Radios,
    Editor,
    Buttons,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Button {
    Tree,
    Editor,
    Save,
    Done,
    Cancel,
}

pub(super) const BUTTONS: [Button; 5] = [
    Button::Tree,
    Button::Editor,
    Button::Save,
    Button::Done,
    Button::Cancel,
];

impl Button {
    pub(super) fn label(self) -> &'static str {
        match self {
            Button::Tree => "Tree",
            Button::Editor => "Editor",
            Button::Save => "Save override",
            Button::Done => "Done",
            Button::Cancel => "Cancel",
        }
    }
}

pub(super) const RADIOS: [Action; 3] = [Action::Approve, Action::Strip, Action::Override];

/// What to do once a question is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Then {
    Select(usize),
    Radio(usize, Action),
    Done,
}

/// A question the screen waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ask {
    /// The edited override isn't saved; asked before leaving it.
    Unsaved(Then),
    /// Saving would replace an override file that wasn't the one opened.
    Overwrite(Option<Then>),
}

impl Ask {
    pub(super) fn title(self) -> &'static str {
        match self {
            Ask::Unsaved(_) => " Unsaved override ",
            Ask::Overwrite(_) => " Override exists ",
        }
    }

    pub(super) fn text(self) -> &'static str {
        match self {
            Ask::Unsaved(_) => "Save the edited file as the override?",
            Ask::Overwrite(_) => "An override file you didn't open is there. Replace it?",
        }
    }

    pub(super) fn answers(self) -> &'static [Answer] {
        match self {
            Ask::Unsaved(_) => &[Answer::Save, Answer::Discard, Answer::Stay],
            Ask::Overwrite(_) => &[Answer::Overwrite, Answer::Stay],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Answer {
    Save,
    Discard,
    Overwrite,
    Stay,
}

impl Answer {
    pub(super) fn label(self) -> &'static str {
        match self {
            Answer::Save => "Save",
            Answer::Discard => "Discard",
            Answer::Overwrite => "Replace",
            Answer::Stay => "Stay",
        }
    }
}

pub(super) struct Dialog {
    pub(super) ask: Ask,
    pub(super) cursor: usize,
}

/// Where things were drawn, for mouse clicks.
#[derive(Default)]
pub(super) struct Areas {
    pub(super) tree: Rect,
    pub(super) radios: Vec<(Rect, Action)>,
    pub(super) editor: Rect,
    pub(super) buttons: Vec<(Rect, Button)>,
    pub(super) answers: Vec<(Rect, Answer)>,
}

/// How the screen was left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exit {
    Done,
    Cancel,
}

/// What saving the override did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Saved {
    Yes,
    /// Nothing saved: nothing to save, or an error, which is in the message.
    No,
    /// An override file that wasn't opened is in the way; saving needs a yes.
    NeedsYes,
}

pub(super) struct App {
    pub(super) tool: String,
    pub(super) items: Vec<Item>,
    /// The action of the entry for the same place whose content changed.
    pub(super) previous: Vec<Option<Action>>,
    pub(super) radio: Vec<Option<Action>>,
    pub(super) files: Vec<Option<String>>,
    pub(super) nodes: Vec<Node>,
    pub(super) selected: usize,
    pub(super) tree_state: ListState,
    pub(super) focus: Focus,
    pub(super) radio_cursor: usize,
    pub(super) button_cursor: usize,
    /// `Ctrl-w` was pressed; the next key picks a pane.
    pub(super) chord: bool,
    pub(super) open: Option<Open>,
    pub(super) keys: EditorEventHandler,
    pub(super) dialog: Option<Dialog>,
    pub(super) message: Option<(String, bool)>,
    pub(super) crate_dir: PathBuf,
    pub(super) areas: Areas,
    pub(super) exit: Option<Exit>,
}

impl App {
    pub(super) fn new(tool: &str, pending: &[Pending], crate_dir: &Path) -> Result<App> {
        let items: Vec<Item> = pending.iter().map(|(i, _)| i.clone()).collect();
        let nodes = tree::build(&items)?;
        let mut app = App {
            tool: tool.to_string(),
            previous: pending.iter().map(|(_, p)| p.map(|e| e.action)).collect(),
            radio: pending.iter().map(|(_, p)| p.map(|e| e.action)).collect(),
            files: pending
                .iter()
                .map(|(_, p)| p.and_then(|e| e.file.clone()))
                .collect(),
            items,
            nodes,
            selected: 0,
            tree_state: ListState::default(),
            focus: Focus::Tree,
            radio_cursor: 0,
            button_cursor: 3,
            chord: false,
            open: None,
            keys: EditorEventHandler::vim_mode(),
            dialog: None,
            message: None,
            crate_dir: crate_dir.to_path_buf(),
            areas: Areas::default(),
            exit: None,
        };
        // Start on the first piece rather than its group.
        if let Some(first) = app.nodes.iter().position(|n| n.item.is_some()) {
            app.selected = first;
        }
        app.open_selected();
        Ok(app)
    }

    /// The decisions, one per pending item; `None` when cancelled.
    pub(super) fn result(&self) -> Option<Vec<Option<Choice>>> {
        if self.exit != Some(Exit::Done) {
            return None;
        }
        Some((0..self.items.len()).map(|i| self.choice(i)).collect())
    }

    pub(super) fn choice(&self, i: usize) -> Option<Choice> {
        match self.radio[i]? {
            Action::Override => Some(Choice {
                action: Action::Override,
                file: Some(self.files[i].clone()?),
            }),
            action => Some(Choice { action, file: None }),
        }
    }

    pub(super) fn allowed(&self, i: usize, action: Action) -> bool {
        self.items[i].kind != Kind::ProcMacro || action == Action::Approve
    }

    pub(super) fn visible(&self) -> Vec<usize> {
        tree::visible(&self.nodes)
    }

    pub(super) fn selected_item(&self) -> Option<usize> {
        self.nodes[self.selected].item
    }

    pub(super) fn editable(&self, open: &Open) -> bool {
        open.own
            && self.radio[open.item] == Some(Action::Override)
            && matches!(open.content, OpenContent::Text { replaced: 0, .. })
    }

    pub(super) fn modified(&self) -> bool {
        self.open.as_ref().is_some_and(|o| {
            self.editable(o)
                && matches!(&o.content, OpenContent::Text { state, baseline, .. }
                    if state.lines.to_string() != *baseline)
        })
    }

    pub(super) fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    pub(super) fn warn(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), true));
    }

    fn ask(&mut self, ask: Ask) {
        self.dialog = Some(Dialog { ask, cursor: 0 });
    }

    /// Run `then`, first asking about an edited override.
    pub(super) fn guarded(&mut self, then: Then) {
        if self.modified() {
            self.ask(Ask::Unsaved(then));
        } else {
            self.run(then);
        }
    }

    pub(super) fn run(&mut self, then: Then) {
        match then {
            Then::Select(node) => {
                self.selected = node;
                self.open_selected();
            }
            Then::Radio(item, action) => self.set_radio(item, action),
            Then::Done => self.exit = Some(Exit::Done),
        }
    }

    pub(super) fn answer(&mut self, answer: Answer) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match (dialog.ask, answer) {
            (Ask::Unsaved(then), Answer::Save) => match self.save_override(false) {
                Saved::Yes => self.run(then),
                Saved::NeedsYes => self.ask(Ask::Overwrite(Some(then))),
                Saved::No => {}
            },
            (Ask::Unsaved(then), Answer::Discard) => {
                self.reopen();
                self.run(then);
            }
            (Ask::Overwrite(then), Answer::Overwrite) => {
                if self.save_override(true) == Saved::Yes
                    && let Some(then) = then
                {
                    self.run(then);
                }
            }
            _ => {}
        }
    }

    /// The file the selected node shows, and whether it's the piece's own file.
    fn file_of(&self, node: usize) -> Option<(usize, PathBuf)> {
        let n = &self.nodes[node];
        let item = n.item?;
        match &n.kind {
            NodeKind::ItemFile(p) | NodeKind::File(p) => Some((item, p.clone())),
            NodeKind::ItemDir if self.items[item].kind == Kind::BuildScript => {
                Some((item, self.items[item].location.clone()))
            }
            _ => None,
        }
    }

    pub(super) fn open_selected(&mut self) {
        self.radio_cursor = self
            .selected_item()
            .and_then(|i| self.radio[i])
            .and_then(|a| RADIOS.iter().position(|r| *r == a))
            .unwrap_or(0);
        self.open = None;
        let Some((item, upstream)) = self.file_of(self.selected) else {
            return;
        };
        let own = upstream == self.items[item].location && self.items[item].kind != Kind::ProcMacro;
        let override_file = self.files[item]
            .as_ref()
            .filter(|_| own && self.radio[item] == Some(Action::Override))
            .map(|f| self.crate_dir.join(f))
            .filter(|p| p.exists());
        let showing_override = override_file.is_some();
        let path = override_file.unwrap_or(upstream);
        let on_disk = if showing_override {
            fs::read(&path).ok().map(|b| sha256_hex(&b))
        } else {
            None
        };
        let content = match read_content(&path) {
            Ok(Content::Text(text, replaced)) => {
                let state = Box::new(EditorState::new(Lines::from(text.as_str())));
                let baseline = state.lines.to_string();
                let rows = highlight::rows(&path, &baseline);
                OpenContent::Text {
                    state,
                    baseline,
                    replaced,
                    rows,
                }
            }
            Ok(Content::Binary(size)) => OpenContent::Binary(size),
            Ok(Content::Link(target)) => OpenContent::Link(target),
            Err(e) => {
                self.warn(format!("{e:#}"));
                return;
            }
        };
        self.open = Some(Open {
            item,
            path,
            own,
            showing_override,
            on_disk,
            content,
        });
    }

    /// Load the open file again, dropping edits.
    pub(super) fn reopen(&mut self) {
        if let Some(o) = &mut self.open
            && let OpenContent::Text {
                state, baseline, ..
            } = &mut o.content
        {
            *baseline = state.lines.to_string();
        }
        self.open_selected();
    }

    pub(super) fn set_radio(&mut self, item: usize, action: Action) {
        if !self.allowed(item, action) {
            self.warn("a proc macro can only be approved");
            return;
        }
        if action == Action::Override {
            if let Err(e) = self.start_override(item) {
                self.warn(format!("{e:#}"));
                return;
            }
        } else {
            self.radio[item] = Some(action);
            self.files[item] = None;
        }
        if self.open.as_ref().is_some_and(|o| o.item == item) {
            self.open_selected();
        }
    }

    /// Choose override for `item`. A file that can't be shown safely gets a commented-out
    /// copy to edit outside, unless an override file is already there; any other is edited
    /// here and saved with the Save button.
    fn start_override(&mut self, item: usize) -> Result<()> {
        let it = &self.items[item];
        let rel = match &self.files[item] {
            Some(f) => f.clone(),
            None => override_rel(it)?,
        };
        let dest = self.crate_dir.join(&rel);
        match read_content(&it.location)? {
            Content::Binary(_) | Content::Link(_) => {
                bail!(
                    "{} isn't a text file; it can't be overridden here",
                    shown(&it.path)
                )
            }
            Content::Text(_, 0) => {
                self.radio[item] = Some(Action::Override);
                if dest.exists() {
                    self.files[item] = Some(rel);
                }
                self.say("edit the file, then press Save override");
            }
            Content::Text(..) => {
                let note = if dest.exists() {
                    "kept the override already in"
                } else {
                    let bytes = fs::read(&it.location)?;
                    let rust = it.location.extension().is_some_and(|e| e == "rs");
                    text::write_override(&self.crate_dir, &rel, &commented_copy(&bytes, rust))?;
                    "wrote a commented-out copy to"
                };
                self.radio[item] = Some(Action::Override);
                self.files[item] = Some(rel.clone());
                self.say(format!(
                    "{note} {}; edit it yourself before installing",
                    shown(&dest.to_string_lossy())
                ));
            }
        }
        Ok(())
    }

    /// Save the editor's text as the override of its piece. Without `replace`, an override
    /// file that isn't the one opened is left alone and a yes is needed.
    pub(super) fn save_override(&mut self, replace: bool) -> Saved {
        match self.try_save(replace) {
            Ok(saved) => saved,
            Err(e) => {
                self.warn(format!("{e:#}"));
                Saved::No
            }
        }
    }

    fn try_save(&mut self, replace: bool) -> Result<Saved> {
        if !self.modified() {
            return Ok(Saved::No);
        }
        let Some(open) = &self.open else {
            return Ok(Saved::No);
        };
        let item = open.item;
        let OpenContent::Text { state, .. } = &open.content else {
            return Ok(Saved::No);
        };
        let mut text = state.lines.to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let rel = match &self.files[item] {
            Some(f) => f.clone(),
            None => override_rel(&self.items[item])?,
        };
        let dest = self.crate_dir.join(&rel);
        if let Ok(current) = fs::read(&dest) {
            let current = sha256_hex(&current);
            match (&open.on_disk, open.showing_override) {
                (Some(loaded), true) if *loaded == current => {}
                (_, true) => bail!(
                    "{} changed on disk since it was opened; it wasn't replaced. Discard your \
                     edits to load it again",
                    shown(&dest.to_string_lossy())
                ),
                (_, false) if !replace => return Ok(Saved::NeedsYes),
                (_, false) => {}
            }
        }
        let dest = text::write_override(&self.crate_dir, &rel, text.as_bytes())?;
        self.files[item] = Some(rel);
        self.say(format!("saved override {}", shown(&dest.to_string_lossy())));
        if let Some(Open {
            path,
            showing_override,
            on_disk,
            content: OpenContent::Text {
                state, baseline, ..
            },
            ..
        }) = &mut self.open
        {
            *baseline = state.lines.to_string();
            *path = dest;
            *showing_override = true;
            *on_disk = Some(sha256_hex(text.as_bytes()));
        }
        Ok(Saved::Yes)
    }

    pub(super) fn press(&mut self, button: Button) {
        match button {
            Button::Tree => self.focus = Focus::Tree,
            Button::Editor => self.focus = Focus::Editor,
            Button::Save => match self.save_override(false) {
                Saved::NeedsYes => self.ask(Ask::Overwrite(None)),
                // An error leaves the edits, and its message, in place.
                Saved::No if !self.modified() => {
                    self.say("nothing to save: choose Override and edit the build script or config")
                }
                _ => {}
            },
            Button::Done => self.guarded(Then::Done),
            Button::Cancel => self.exit = Some(Exit::Cancel),
        }
    }

    pub(super) fn counts(&self) -> [usize; 4] {
        let mut c = [0; 4];
        for i in 0..self.items.len() {
            match self.choice(i).map(|c| c.action) {
                Some(Action::Approve) => c[0] += 1,
                Some(Action::Override) => c[1] += 1,
                Some(Action::Strip) => c[2] += 1,
                None => c[3] += 1,
            }
        }
        c
    }

    pub(super) fn glyph(&self, i: usize) -> &'static str {
        match (self.radio[i], &self.files[i]) {
            (None, _) => "?",
            (Some(Action::Approve), _) => "✓",
            (Some(Action::Strip), _) => "⊘",
            (Some(Action::Override), Some(_)) => "✎",
            (Some(Action::Override), None) => "✎?",
        }
    }
}

#[cfg(test)]
#[path = "app_test.rs"]
mod tests;
