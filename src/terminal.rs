use std::io::{self, Stdout};
use std::panic;

use anyhow::Result;
use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

pub type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct TerminalGuard {
    restored: bool,
}

impl TerminalGuard {
    pub fn new() -> Self {
        Self { restored: false }
    }

    pub fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        restore_stdio_terminal()?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub fn init_terminal() -> Result<(AppTerminal, TerminalGuard)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok((terminal, TerminalGuard::new()))
}

pub fn install_panic_hook() {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_stdio_terminal();
        previous_hook(panic_info);
    }));
}

fn restore_stdio_terminal() -> Result<()> {
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, Show, LeaveAlternateScreen)?;
    Ok(())
}
