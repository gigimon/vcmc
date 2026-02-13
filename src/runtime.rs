use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use crossterm::event::{self, Event as CrosstermEvent};
use tracing::warn;

use crate::model::Event;

static INPUT_POLL_PAUSED: AtomicBool = AtomicBool::new(false);

pub fn set_input_poll_paused(paused: bool) {
    INPUT_POLL_PAUSED.store(paused, Ordering::SeqCst);
}

pub fn spawn_event_pump(tx: Sender<Event>, tick_rate: Duration) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut last_tick = Instant::now();

        loop {
            if INPUT_POLL_PAUSED.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(16));
                last_tick = Instant::now();
                continue;
            }

            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            match event::poll(timeout) {
                Ok(true) => match event::read() {
                    Ok(CrosstermEvent::Key(key)) => {
                        if tx.send(Event::Input(key)).is_err() {
                            break;
                        }
                    }
                    Ok(CrosstermEvent::Resize(width, height)) => {
                        if tx.send(Event::Resize { width, height }).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!("failed to read terminal event: {err}");
                    }
                },
                Ok(false) => {}
                Err(err) => {
                    warn!("failed to poll terminal event: {err}");
                }
            }

            if last_tick.elapsed() >= tick_rate {
                if tx.send(Event::Tick).is_err() {
                    break;
                }
                last_tick = Instant::now();
            }
        }
    })
}
