//! The setup screen for `add -i` and `update -i`: the tool's version, its features and the
//! dependency tree they build, with advisories, so features can be turned off and versions
//! moved before the build review.
//!
//! - `app`: the screen's state and what its actions do.
//! - `input` and `render`: keys and mouse, and drawing.
//! - `text_view`: text that scrolls and that a mouse drag selects and copies.
//! - `advisory`: an advisory as text for its dialog.

use anyhow::Result;
use ratatui::crossterm::event;

use crate::output;
use crate::screen::Terminal;
use crate::setup::Setup;

mod advisory;
mod app;
mod input;
mod render;
mod text_view;

pub use app::{App, Mover};

/// Show the setup screen. Returns the setup as chosen, or `None` when cancelled.
pub fn run(setup: Setup, mover: Mover) -> Result<Option<Setup>> {
    let mut app = App::new(setup, mover);
    // What a version move prints waits until the screen is gone.
    let _hold = output::hold();
    let mut terminal = Terminal::start()?;
    while app.exit.is_none() {
        terminal.draw(|f| app.render(f))?;
        if app.pending.is_some() {
            // `working…` is on screen; now do the slow part.
            app.run_pending();
        } else {
            app.handle(event::read()?);
        }
    }
    drop(terminal);
    Ok(app.finish())
}
