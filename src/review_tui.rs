//! The terminal screen for reviewing build-time code piece by piece: a tree of what's
//! pending, the decision for the selected piece, and its files in a vim-style editor.
//!
//! - `text`: reading files, making their text safe to draw, writing overrides.
//! - `tree`: the tree of pieces and their files.
//! - `app`: the screen's state and what its actions do.
//! - `input` and `render`: keys and mouse, and drawing.

use std::path::Path;

use anyhow::Result;
use ratatui::crossterm::event;

use crate::review::{Action, Known, Pending};
use crate::screen::Terminal;

#[path = "review_tui/app.rs"]
mod app;
#[cfg(test)]
#[path = "review_tui/fixtures.rs"]
mod fixtures;
#[path = "review_tui/input.rs"]
mod input;
#[path = "review_tui/render.rs"]
mod render;
#[path = "review_tui/text.rs"]
mod text;
#[path = "review_tui/tree.rs"]
mod tree;

pub use text::override_rel;

/// First line of an override copied from a file that can't be shown safely. `apply` refuses
/// an override that still has it.
pub const UNREVIEWED: &str =
    "UNREVIEWED COPY: contains characters that can't be shown safely; edit before use";

/// A decision made on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub action: Action,
    /// With `override`: the replacement, relative to the tool's crate directory.
    pub file: Option<String>,
}

/// Show the review screen for `pending`, with the decisions `known` from other tools
/// pre-selected. Returns a decision (or none) per item, or `None` when cancelled.
pub fn run(
    tool: &str,
    pending: &[Pending],
    known: &[Option<Known>],
    crate_dir: &Path,
) -> Result<Option<Vec<Option<Choice>>>> {
    let mut app = app::App::new(tool, pending, known, crate_dir)?;
    let mut terminal = Terminal::start()?;
    while app.exit.is_none() {
        terminal.draw(|f| app.render(f))?;
        app.handle(event::read()?);
    }
    if app.exit == Some(app::Exit::Done) {
        app.copy_sources()?;
    }
    Ok(app.result())
}
