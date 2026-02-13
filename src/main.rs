mod app;
mod backend;
mod errors;
mod fs;
mod jobs;
mod model;
mod runtime;
mod smoke;
mod terminal;
mod theme;
mod ui;
mod viewer;

use std::env;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::unbounded;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    init_tracing();
    terminal::install_panic_hook();

    if is_smoke_mode() {
        let report = smoke::run_smoke()?;
        println!("{}", report.to_text());
        return Ok(());
    }

    let cwd = env::current_dir()?;
    let (event_tx, event_rx) = unbounded();
    let mut app = app::App::bootstrap(cwd, event_tx.clone())?;
    let (mut terminal, mut guard) = terminal::init_terminal()?;
    let runtime_handle = runtime::spawn_event_pump(event_tx, Duration::from_millis(150));

    terminal.draw(|frame| ui::render(frame, app.state(), app.theme()))?;

    while app.is_running() {
        let event = event_rx.recv()?;
        let mut should_redraw = app.on_event(event);
        if app.take_force_full_redraw() {
            terminal.clear()?;
            should_redraw = true;
        }
        if should_redraw {
            terminal.draw(|frame| ui::render(frame, app.state(), app.theme()))?;
        }
    }

    guard.restore()?;
    drop(event_rx);

    if runtime_handle.join().is_err() {
        debug!("runtime thread finished with panic");
    }

    info!("vcmc shutdown complete");
    Ok(())
}

fn is_smoke_mode() -> bool {
    env::args().any(|arg| arg == "--smoke")
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
