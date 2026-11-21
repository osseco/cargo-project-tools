//! The terminal for full-screen views: raw mode, the alternate screen and the mouse, put
//! back however the view is left.

use std::io::{self, IsTerminal};

use anyhow::{Context, Result};
use ratatui::Frame;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType};

/// Whether a full-screen view can run: both input and output are a terminal.
pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Restores the terminal when dropped, on errors and panics too (`ratatui::try_init`
/// installs a panic hook).
pub struct Terminal(ratatui::DefaultTerminal);

impl Terminal {
    pub fn start() -> Result<Terminal> {
        let terminal = ratatui::try_init().context("starting the screen")?;
        // Built before the terminal state it guards, so a failure below restores it too.
        let terminal = Terminal(terminal);
        // Not every terminal starts the alternate screen blank, and the first draw only
        // writes cells that aren't.
        execute!(io::stdout(), EnableMouseCapture, Clear(ClearType::All))
            .context("preparing the terminal")?;
        Ok(terminal)
    }

    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<()> {
        self.0.draw(render)?;
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}
