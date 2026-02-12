use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};

use crate::fs::FsAdapter;
use crate::model::{
    AppState, Command, Event, FsEntryType, JobStatus, JobUpdate, PanelId, TerminalSize,
};

pub struct App {
    state: AppState,
    running: bool,
    fs: FsAdapter,
}

impl App {
    pub fn bootstrap(cwd: PathBuf) -> anyhow::Result<Self> {
        let fs = FsAdapter::default();
        let normalized_cwd = fs.normalize_existing_path("bootstrap", &cwd)?;
        let mut app = Self {
            state: AppState::new(normalized_cwd),
            running: true,
            fs,
        };
        app.reload_panel(PanelId::Left)?;
        app.reload_panel(PanelId::Right)?;
        Ok(app)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn on_event(&mut self, event: Event) {
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
    }

    fn apply_command(&mut self, command: Command) {
        let command_result: anyhow::Result<()> = match command {
            Command::Quit => {
                self.running = false;
                Ok(())
            }
            Command::SwitchPanel => {
                self.state.active_panel = match self.state.active_panel {
                    PanelId::Left => PanelId::Right,
                    PanelId::Right => PanelId::Left,
                };
                Ok(())
            }
            Command::MoveSelectionUp => {
                self.active_panel_mut().move_selection_up();
                Ok(())
            }
            Command::MoveSelectionDown => {
                self.active_panel_mut().move_selection_down();
                Ok(())
            }
            Command::Refresh => self
                .reload_panel(PanelId::Left)
                .and_then(|_| self.reload_panel(PanelId::Right)),
            Command::OpenSelected => self.open_selected_directory(),
            Command::GoToParent => self.go_to_parent(),
            Command::GoHome => self.go_to_home(),
            Command::Copy | Command::Move | Command::Delete | Command::Mkdir => Ok(()),
            Command::ToggleSort => self.toggle_sort(),
        };

        if let Err(err) = command_result {
            self.state.status_line = err.to_string();
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

    fn active_panel(&self) -> &crate::model::PanelState {
        match self.state.active_panel {
            PanelId::Left => &self.state.left_panel,
            PanelId::Right => &self.state.right_panel,
        }
    }

    fn panel_mut(&mut self, id: PanelId) -> &mut crate::model::PanelState {
        match id {
            PanelId::Left => &mut self.state.left_panel,
            PanelId::Right => &mut self.state.right_panel,
        }
    }

    fn reload_panel(&mut self, panel_id: PanelId) -> anyhow::Result<()> {
        let (cwd, sort_mode, show_hidden) = {
            let panel = self.panel_mut(panel_id);
            (panel.cwd.clone(), panel.sort_mode, panel.show_hidden)
        };
        let entries = self.fs.list_dir(&cwd, sort_mode, show_hidden)?;

        let panel = self.panel_mut(panel_id);
        panel.entries = entries;
        panel.normalize_selection();
        self.state.status_line = format!("Loaded {}", cwd.display());
        Ok(())
    }

    fn open_selected_directory(&mut self) -> anyhow::Result<()> {
        let selected = self.active_panel().selected_entry().cloned();
        let Some(entry) = selected else {
            return Ok(());
        };

        if entry.entry_type != FsEntryType::Directory {
            self.state.status_line = format!("{} is not a directory", entry.name);
            return Ok(());
        }

        let next_path = self.fs.normalize_existing_path("open", &entry.path)?;
        self.active_panel_mut().cwd = next_path;
        self.reload_panel(self.state.active_panel)?;
        Ok(())
    }

    fn go_to_parent(&mut self) -> anyhow::Result<()> {
        let current = self.active_panel().cwd.clone();
        let Some(parent) = current.parent() else {
            return Ok(());
        };
        let normalized = self.fs.normalize_existing_path("parent", parent)?;
        self.active_panel_mut().cwd = normalized;
        self.reload_panel(self.state.active_panel)?;
        Ok(())
    }

    fn go_to_home(&mut self) -> anyhow::Result<()> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("HOME environment variable is not set"))?;
        let normalized = self.fs.normalize_existing_path("home", &home)?;
        self.active_panel_mut().cwd = normalized;
        self.reload_panel(self.state.active_panel)?;
        Ok(())
    }

    fn toggle_sort(&mut self) -> anyhow::Result<()> {
        let next_sort = self.active_panel().sort_mode.next();
        self.active_panel_mut().sort_mode = next_sort;
        self.reload_panel(self.state.active_panel)
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
        KeyCode::Home => Some(Command::GoHome),
        _ => None,
    }
}
