use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::model::{AppState, Command, Event, JobStatus, JobUpdate, PanelId, TerminalSize};

pub struct App {
    state: AppState,
    running: bool,
}

impl App {
    pub fn bootstrap(cwd: PathBuf) -> Self {
        Self {
            state: AppState::new(cwd),
            running: true,
        }
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn on_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Input(key) => {
                if let Some(cmd) = map_key_to_command(key) {
                    self.apply_command(cmd);
                }
            }
            Event::Tick => {}
            Event::Resize { width, height } => {
                self.state.terminal_size = TerminalSize { width, height };
            }
            Event::Job(update) => self.handle_job_update(update),
        }

        Ok(())
    }

    fn apply_command(&mut self, command: Command) {
        match command {
            Command::Quit => self.running = false,
            Command::SwitchPanel => {
                self.state.active_panel = match self.state.active_panel {
                    PanelId::Left => PanelId::Right,
                    PanelId::Right => PanelId::Left,
                };
            }
            Command::MoveSelectionUp => self.active_panel_mut().move_selection_up(),
            Command::MoveSelectionDown => self.active_panel_mut().move_selection_down(),
            Command::Refresh => {
                self.state.status_line = "refresh requested".to_string();
            }
            Command::OpenSelected
            | Command::GoToParent
            | Command::GoHome
            | Command::Copy
            | Command::Move
            | Command::Delete
            | Command::Mkdir
            | Command::ToggleSort => {}
        }
    }

    fn handle_job_update(&mut self, update: JobUpdate) {
        let next_status_line = match update.status {
            JobStatus::Failed => update
                .message
                .clone()
                .unwrap_or_else(|| "job failed".to_string()),
            JobStatus::Done => update
                .message
                .clone()
                .unwrap_or_else(|| "job finished".to_string()),
            JobStatus::Queued | JobStatus::Running => "job updated".to_string(),
        };

        if let Some(job) = self.state.jobs.iter_mut().find(|job| job.id == update.id) {
            job.status = update.status;
            job.message = update.message.clone();
        } else {
            self.state.jobs.push(update.into_job());
        }

        self.state.status_line = next_status_line;
    }

    fn active_panel_mut(&mut self) -> &mut crate::model::PanelState {
        match self.state.active_panel {
            PanelId::Left => &mut self.state.left_panel,
            PanelId::Right => &mut self.state.right_panel,
        }
    }
}

fn map_key_to_command(key: KeyEvent) -> Option<Command> {
    match key.code {
        KeyCode::Char('q') => Some(Command::Quit),
        KeyCode::Tab => Some(Command::SwitchPanel),
        KeyCode::Up => Some(Command::MoveSelectionUp),
        KeyCode::Down => Some(Command::MoveSelectionDown),
        KeyCode::Enter => Some(Command::OpenSelected),
        KeyCode::Backspace => Some(Command::GoToParent),
        KeyCode::F(5) => Some(Command::Copy),
        KeyCode::F(6) => Some(Command::Move),
        KeyCode::F(7) => Some(Command::Mkdir),
        KeyCode::F(8) => Some(Command::Delete),
        KeyCode::F(2) => Some(Command::ToggleSort),
        KeyCode::Char('r') => Some(Command::Refresh),
        _ => None,
    }
}
