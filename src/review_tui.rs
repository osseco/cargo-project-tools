//! The terminal screen for reviewing build-time code piece by piece: a tree of what's
//! pending, the decision for the selected piece, and its files in a vim-style editor.
//!
//! - `text`: reading files, making their text safe to draw, writing overrides.
//! - `tree`: the tree of pieces and their files.
//! - `app`: the screen's state and what its actions do.
//! - `input` and `render`: keys and mouse, and drawing.

use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType};

use crate::review::{Action, Pending};

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

pub use text::shown;

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

/// Restores the terminal however the screen is left.
struct Terminal(ratatui::DefaultTerminal);

impl Terminal {
    fn start() -> Result<Terminal> {
        let terminal = ratatui::try_init().context("starting the review screen")?;
        // Built before the terminal state it guards, so a failure below restores it too.
        let terminal = Terminal(terminal);
        // Not every terminal starts the alternate screen blank, and the first draw only
        // writes cells that aren't.
        execute!(io::stdout(), EnableMouseCapture, Clear(ClearType::All))
            .context("preparing the terminal")?;
        Ok(terminal)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}

/// Show the review screen for `pending`. Returns a decision (or none) per item, or `None`
/// when cancelled.
pub fn run(
    tool: &str,
    pending: &[Pending],
    crate_dir: &Path,
) -> Result<Option<Vec<Option<Choice>>>> {
    let mut app = app::App::new(tool, pending, crate_dir)?;
    let mut terminal = Terminal::start()?;
    while app.exit.is_none() {
        terminal.0.draw(|f| app.render(f))?;
        app.handle(event::read()?);
    }
    Ok(app.result())
}
