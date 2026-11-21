//! The setup screen's state and what its actions do. Input and drawing are in `input` and
//! `render`.

use std::collections::BTreeSet;

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use semver::Version;

use crate::sanitize::shown_line;
use crate::setup::{Choice, Setup};
use crate::source::Checkout;

/// Moves the tool: to this version, or for `None`, to the newest (a git tool: the head of
/// its branch or tag again). Returns the new checkout with its lock.
pub type Mover<'a> = Box<dyn FnMut(Option<&Version>) -> Result<Checkout> + 'a>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Tool,
    Features,
    Dependencies,
}

impl Section {
    pub(super) fn label(self) -> &'static str {
        match self {
            Section::Tool => "Tool",
            Section::Features => "Features",
            Section::Dependencies => "Dependencies",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RowKind {
    Header(Section),
    Tool,
    Default,
    Feature(String),
    /// Index into `Setup::tree`.
    Dep(usize),
}

#[derive(Debug, Clone)]
pub(super) struct Row {
    pub(super) depth: usize,
    pub(super) kind: RowKind,
    pub(super) folds: bool,
    pub(super) expanded: bool,
    /// A stable name for the row (`row_key`), worked out when the row is built: `Dep`
    /// indexes the tree as it was then.
    pub(super) key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Tree,
    Details,
    Buttons,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Button {
    Tree,
    Details,
    NextFinding,
    Done,
    Cancel,
}

pub(super) const BUTTONS: [Button; 5] = [
    Button::Tree,
    Button::Details,
    Button::NextFinding,
    Button::Done,
    Button::Cancel,
];

impl Button {
    pub(super) fn label(self) -> &'static str {
        match self {
            Button::Tree => "Tree",
            Button::Details => "Details",
            Button::NextFinding => "Next finding",
            Button::Done => "Done",
            Button::Cancel => "Cancel",
        }
    }
}

/// Something slow (cargo, the index, a checkout), run after `working…` has been drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Op {
    ToggleFeature(String),
    ToggleDefault,
    /// Versions and the why path for the selected package, or the tool's versions.
    Load,
    SetVersion(String, Version, Version),
    /// `None`: the newest, or the branch/tag head for a git tool.
    MoveTool(Option<Version>),
}

/// What's loaded for the selected row.
pub(super) struct Detail {
    pub(super) key: String,
    pub(super) why: Vec<String>,
    pub(super) versions: Vec<Choice>,
}

/// Where things were drawn, for mouse clicks.
#[derive(Default)]
pub(super) struct Areas {
    pub(super) tree: Rect,
    pub(super) versions: Rect,
    pub(super) buttons: Vec<(Rect, Button)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exit {
    Done,
    Cancel,
}

pub struct App<'a> {
    pub(super) setup: Setup,
    pub(super) mover: Mover<'a>,
    pub(super) rows: Vec<Row>,
    pub(super) selected: usize,
    pub(super) tree_state: ListState,
    pub(super) versions_state: ListState,
    pub(super) focus: Focus,
    pub(super) button_cursor: usize,
    /// `Ctrl-w` was pressed; the next key picks a pane.
    pub(super) chord: bool,
    pub(super) detail: Option<Detail>,
    pub(super) pending: Option<Op>,
    pub(super) message: Option<(String, bool)>,
    pub(super) areas: Areas,
    pub(super) exit: Option<Exit>,
}

impl<'a> App<'a> {
    pub fn new(setup: Setup, mover: Mover<'a>) -> App<'a> {
        let mut app = App {
            setup,
            mover,
            rows: Vec::new(),
            selected: 0,
            tree_state: ListState::default(),
            versions_state: ListState::default(),
            focus: Focus::Tree,
            button_cursor: 3,
            chord: false,
            detail: None,
            pending: None,
            message: None,
            areas: Areas::default(),
            exit: None,
        };
        app.rebuild();
        app.selected = app
            .rows
            .iter()
            .position(|r| matches!(r.kind, RowKind::Feature(_) | RowKind::Default))
            .unwrap_or(0);
        app
    }

    /// The setup as chosen, after Done.
    pub fn finish(self) -> Option<Setup> {
        (self.exit == Some(Exit::Done)).then_some(self.setup)
    }

    /// Messages quote paths, names and cargo's errors: drawn safe, on one line.
    pub(super) fn say(&mut self, text: impl Into<String>) {
        self.message = Some((shown_line(&text.into()), false));
    }

    pub(super) fn warn(&mut self, text: impl Into<String>) {
        self.message = Some((shown_line(&text.into()), true));
    }

    /// A stable name for a row, to find it again after the tree is rebuilt.
    pub(super) fn row_key(&self, kind: &RowKind) -> String {
        match kind {
            RowKind::Header(s) => format!("h:{}", s.label()),
            RowKind::Tool => "tool".into(),
            RowKind::Default => "default".into(),
            RowKind::Feature(f) => format!("f:{f}"),
            RowKind::Dep(i) => {
                let d = &self.setup.tree[*i];
                // The tree's root is the tool, which a move changes.
                if d.depth == 0 {
                    "root".into()
                } else {
                    format!("d:{}@{}#{}", d.name, d.version, self.dep_path(*i))
                }
            }
        }
    }

    /// The names from the root down to dependency `i`, so repeats in different places
    /// stay apart.
    fn dep_path(&self, i: usize) -> String {
        let tree = &self.setup.tree;
        let mut names = vec![tree[i].name.as_str()];
        let mut depth = tree[i].depth;
        for d in tree[..i].iter().rev() {
            if d.depth < depth {
                names.push(&d.name);
                depth = d.depth;
            }
        }
        names.reverse();
        names.join("/")
    }

    /// Rows for the current setup, keeping what was unfolded and selected.
    pub(super) fn rebuild(&mut self) {
        let old_selected = self.rows.get(self.selected).map(|r| r.key.clone());
        let open: BTreeSet<String> = self
            .rows
            .iter()
            .filter(|r| r.folds && r.expanded)
            .map(|r| r.key.clone())
            .collect();
        let folded: BTreeSet<String> = self
            .rows
            .iter()
            .filter(|r| r.folds && !r.expanded)
            .map(|r| r.key.clone())
            .collect();
        let mut rows = vec![
            Row {
                depth: 0,
                kind: RowKind::Header(Section::Tool),
                folds: true,
                expanded: true,
                key: String::new(),
            },
            Row {
                depth: 1,
                kind: RowKind::Tool,
                folds: false,
                expanded: false,
                key: String::new(),
            },
            Row {
                depth: 0,
                kind: RowKind::Header(Section::Features),
                folds: true,
                expanded: true,
                key: String::new(),
            },
            Row {
                depth: 1,
                kind: RowKind::Default,
                folds: false,
                expanded: false,
                key: String::new(),
            },
        ];
        for f in self.setup.features.keys().filter(|f| *f != "default") {
            rows.push(Row {
                depth: 1,
                kind: RowKind::Feature(f.clone()),
                folds: false,
                expanded: false,
                key: String::new(),
            });
        }
        rows.push(Row {
            depth: 0,
            kind: RowKind::Header(Section::Dependencies),
            folds: true,
            expanded: true,
            key: String::new(),
        });
        let tree = &self.setup.tree;
        for (i, d) in tree.iter().enumerate() {
            let folds = !d.repeat && tree.get(i + 1).is_some_and(|n| n.depth > d.depth);
            rows.push(Row {
                depth: d.depth + 1,
                kind: RowKind::Dep(i),
                folds,
                // The tool's own dependencies show; deeper levels start folded.
                expanded: d.depth == 0,
                key: String::new(),
            });
        }
        for row in &mut rows {
            row.key = self.row_key(&row.kind);
        }
        for row in rows.iter_mut().filter(|r| r.folds) {
            let key = &row.key;
            if open.contains(key) {
                row.expanded = true;
            } else if folded.contains(key) {
                row.expanded = false;
            }
        }
        self.rows = rows;
        self.selected = old_selected
            .and_then(|k| self.rows.iter().position(|r| r.key == k))
            .unwrap_or(self.selected.min(self.rows.len() - 1));
        if self.detail.as_ref().is_some_and(|d| {
            Some(&d.key) != self.rows.get(self.selected).map(|r| r.key.clone()).as_ref()
        }) {
            self.detail = None;
        }
    }

    pub(super) fn visible(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut hidden_below: Option<usize> = None;
        for (i, r) in self.rows.iter().enumerate() {
            if let Some(d) = hidden_below {
                if r.depth > d {
                    continue;
                }
                hidden_below = None;
            }
            out.push(i);
            if r.folds && !r.expanded {
                hidden_below = Some(r.depth);
            }
        }
        out
    }

    pub(super) fn parent(&self, row: usize) -> Option<usize> {
        let depth = self.rows[row].depth;
        (0..row).rev().find(|&i| self.rows[i].depth < depth)
    }

    pub(super) fn select(&mut self, row: usize) {
        if row != self.selected {
            self.selected = row;
            self.versions_state.select(None);
            if self
                .detail
                .as_ref()
                .is_some_and(|d| d.key != self.rows[row].key)
            {
                self.detail = None;
            }
        }
    }

    /// Queue `op`; the loop draws `working…` and then runs it.
    pub(super) fn queue(&mut self, op: Op) {
        self.say("working…");
        self.pending = Some(op);
    }

    /// Run the queued operation, if any.
    pub fn run_pending(&mut self) {
        let Some(op) = self.pending.take() else {
            return;
        };
        self.message = None;
        if let Err(e) = self.perform(op) {
            self.warn(format!("{e:#}"));
        }
    }

    fn perform(&mut self, op: Op) -> Result<()> {
        match op {
            Op::ToggleFeature(f) => {
                let mut selected = self.setup.selected.clone();
                if !selected.remove(&f) {
                    selected.insert(f.clone());
                }
                let default = self.setup.default_features;
                self.change_features(selected, default)?;
            }
            Op::ToggleDefault => {
                let selected = self.setup.selected.clone();
                let default = !self.setup.default_features;
                self.change_features(selected, default)?;
            }
            Op::Load => self.load()?,
            Op::SetVersion(name, from, to) => {
                self.setup.set_version(&name, &from, &to)?;
                self.detail = None;
                self.rebuild();
                self.say(format!("{name} {from} → {to}"));
            }
            Op::MoveTool(to) => {
                let from = self.setup.checkout.version.clone();
                let checkout = (self.mover)(to.as_ref())?;
                self.setup.set_checkout(checkout)?;
                self.detail = None;
                self.rebuild();
                self.say(format!(
                    "{} {from} → {}",
                    self.setup.name, self.setup.checkout.version
                ));
            }
        }
        Ok(())
    }

    fn change_features(&mut self, selected: BTreeSet<String>, default: bool) -> Result<()> {
        let (old, old_default) = (self.setup.selected.clone(), self.setup.default_features);
        if let Err(e) = self.setup.set_features(selected, default) {
            // Put the last working selection back.
            self.setup.set_features(old, old_default)?;
            return Err(e);
        }
        self.rebuild();
        let (built, vulns, warnings) = self.setup.counts();
        self.say(format!(
            "{built} packages, {vulns} vulnerable, {warnings} with warnings"
        ));
        Ok(())
    }

    /// Load versions (and for a package, why it's built) for the selected row.
    fn load(&mut self) -> Result<()> {
        let Some(row) = self.rows.get(self.selected) else {
            return Ok(());
        };
        let key = row.key.clone();
        let (name, version, why) = match row.kind {
            RowKind::Dep(i) if self.setup.tree[i].depth > 0 => {
                let d = self.setup.tree[i].clone();
                let why = self.setup.why(&d.name, &d.version)?;
                (d.name, d.version, why)
            }
            RowKind::Tool | RowKind::Dep(_) => {
                let version = Version::parse(&self.setup.checkout.version)?;
                (self.setup.name.clone(), version, Vec::new())
            }
            _ => return Ok(()),
        };
        let versions = if self.setup.git && name == self.setup.name {
            Vec::new()
        } else {
            self.setup.versions(&name, &version)
        };
        let cursor = versions
            .iter()
            .position(|c| c.suggested)
            .or_else(|| versions.iter().position(|c| c.current));
        self.versions_state.select(cursor);
        self.detail = Some(Detail { key, why, versions });
        Ok(())
    }

    /// The package of the selected row: (name, version), the tool's for its own rows.
    pub(super) fn selected_package(&self) -> Option<(String, Version, bool)> {
        match self.rows.get(self.selected)?.kind {
            RowKind::Dep(i) if self.setup.tree[i].depth > 0 => {
                let d = &self.setup.tree[i];
                Some((d.name.clone(), d.version.clone(), false))
            }
            RowKind::Tool | RowKind::Dep(_) => Some((
                self.setup.name.clone(),
                Version::parse(&self.setup.checkout.version).ok()?,
                true,
            )),
            _ => None,
        }
    }

    /// Act on the selected row: toggle a feature, or load a package's details.
    pub(super) fn activate(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        match &row.kind {
            RowKind::Feature(f) => self.queue(Op::ToggleFeature(f.clone())),
            RowKind::Default => self.queue(Op::ToggleDefault),
            RowKind::Tool | RowKind::Dep(_) => {
                if self.setup.git && self.selected_package().is_some_and(|p| p.2) {
                    self.focus = Focus::Details;
                    return;
                }
                self.queue(Op::Load);
                self.focus = Focus::Details;
            }
            RowKind::Header(_) => {
                let r = &mut self.rows[self.selected];
                r.expanded = !r.expanded;
            }
        }
    }

    /// Use the version under the cursor in the details.
    pub(super) fn use_version(&mut self) {
        let Some((name, current, tool)) = self.selected_package() else {
            return;
        };
        if tool && self.setup.git {
            self.queue(Op::MoveTool(None));
            return;
        }
        let Some(choice) = self
            .versions_state
            .selected()
            .and_then(|i| self.detail.as_ref()?.versions.get(i))
            .cloned()
        else {
            self.say("press Enter on a package in the tree to list its versions");
            return;
        };
        if choice.current {
            self.say(format!("{name} is already at {current}"));
        } else if tool {
            self.queue(Op::MoveTool(Some(choice.version)));
        } else {
            self.queue(Op::SetVersion(name, current, choice.version));
        }
    }

    /// Select the next package with an open finding, unfolding its way there.
    pub(super) fn next_finding(&mut self) {
        let flagged = |app: &App, row: &Row| match row.kind {
            RowKind::Dep(i) => {
                let d = &app.setup.tree[i];
                !d.repeat
                    && app
                        .setup
                        .findings
                        .get(&(d.name.clone(), d.version.clone()))
                        .is_some_and(|f| f.iter().any(|f| !f.ignored))
            }
            _ => false,
        };
        let n = self.rows.len();
        let found = (1..=n)
            .map(|k| (self.selected + k) % n)
            .find(|&i| flagged(self, &self.rows[i]));
        let Some(target) = found else {
            self.say("no open findings");
            return;
        };
        let mut p = self.parent(target);
        while let Some(i) = p {
            self.rows[i].expanded = true;
            p = self.parent(i);
        }
        self.select(target);
        self.focus = Focus::Tree;
    }

    pub(super) fn press(&mut self, button: Button) {
        match button {
            Button::Tree => self.focus = Focus::Tree,
            Button::Details => self.focus = Focus::Details,
            Button::NextFinding => self.next_finding(),
            Button::Done => self.exit = Some(Exit::Done),
            Button::Cancel => self.exit = Some(Exit::Cancel),
        }
    }

    /// Whether `feature` is on: chosen, or one of the defaults while they're on.
    pub(super) fn feature_on(&self, feature: &str) -> (bool, bool) {
        let chosen = self.setup.selected.contains(feature);
        let by_default = self.setup.default_features
            && self
                .setup
                .features
                .get("default")
                .is_some_and(|d| d.iter().any(|x| x == feature));
        (chosen, by_default)
    }
}

#[cfg(test)]
#[path = "app_test.rs"]
mod tests;
