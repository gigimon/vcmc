mod app;
mod errors;
mod model;
mod runtime;
mod terminal;
mod ui;

use std::env;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::unbounded;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    init_tracing();
    terminal::install_panic_hook();

    let (mut terminal, mut guard) = terminal::init_terminal()?;
    let cwd = env::current_dir()?;
    let mut app = app::App::bootstrap(cwd);

    let (event_tx, event_rx) = unbounded();
    let runtime_handle = runtime::spawn_event_pump(event_tx, Duration::from_millis(150));

    while app.is_running() {
        terminal.draw(|frame| ui::render(frame, app.state()))?;
        let event = event_rx.recv()?;
        app.on_event(event)?;
    }

    guard.restore()?;
    drop(event_rx);

    if runtime_handle.join().is_err() {
        debug!("runtime thread finished with panic");
    }

    info!("vcmc shutdown complete");
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
