use std::path::PathBuf;

use anyhow::Result;
use crossbeam_channel::Sender;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::fs::FsAdapter;
use crate::jobs::WorkerPool;
use crate::model::{
    AppState, Command, Event, FsEntryType, Job, JobKind, JobRequest, JobStatus, JobUpdate, PanelId,
    TerminalSize,
};

pub struct App {
    state: AppState,
    running: bool,
    fs: FsAdapter,
    workers: WorkerPool,
    next_job_id: u64,
    pending_confirmation: Option<PendingConfirmation>,
    input_mode: Option<InputMode>,
}

enum PendingConfirmation {
    Delete { path: PathBuf, name: String },
}

enum InputMode {
    Search(PanelId),
}

impl App {
    pub fn bootstrap(cwd: PathBuf, event_tx: Sender<Event>) -> Result<Self> {
        let fs = FsAdapter::default();
        let normalized_cwd = fs.normalize_existing_path("bootstrap", &cwd)?;
        let workers = WorkerPool::new(2, event_tx);
        let mut app = Self {
            state: AppState::new(normalized_cwd),
            running: true,
            fs,
            workers,
            next_job_id: 1,
            pending_confirmation: None,
            input_mode: None,
        };

        let _ = app.reload_panel(PanelId::Left, true)?;
        let _ = app.reload_panel(PanelId::Right, false)?;
        Ok(app)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn on_event(&mut self, event: Event) -> bool {
        match event {
            Event::Input(key) => {
                if let Some(redraw) = self.handle_confirmation_input(&key) {
                    return redraw;
                }

                if let Some(redraw) = self.handle_search_input(&key) {
                    return redraw;
                }

                if let Some(cmd) = map_key_to_command(&key) {
                    self.apply_command(cmd)
                } else {
                    false
                }
            }
            Event::Tick => false,
            Event::Resize { width, height } => {
                self.state.terminal_size = TerminalSize { width, height };
                true
            }
            Event::Job(update) => self.handle_job_update(update),
        }
    }

    fn apply_command(&mut self, command: Command) -> bool {
        let command_result: Result<bool> = match command {
            Command::Quit => {
                self.running = false;
                Ok(false)
            }
            Command::SwitchPanel => {
                self.state.active_panel = match self.state.active_panel {
                    PanelId::Left => PanelId::Right,
                    PanelId::Right => PanelId::Left,
                };
                self.input_mode = None;
                Ok(true)
            }
            Command::MoveSelectionUp => {
                self.active_panel_mut().move_selection_up();
                Ok(true)
            }
            Command::MoveSelectionDown => {
                self.active_panel_mut().move_selection_down();
                Ok(true)
            }
            Command::Refresh => self
                .reload_panel(PanelId::Left, true)
                .and_then(|_| self.reload_panel(PanelId::Right, false)),
            Command::OpenSelected => self.open_selected_directory(),
            Command::GoToParent => self.go_to_parent(),
            Command::GoHome => self.go_to_home(),
            Command::Copy => self.queue_copy(),
            Command::Move => self.queue_move(),
            Command::Delete => self.queue_delete(),
            Command::Mkdir => self.queue_mkdir(),
            Command::ToggleSort => self.toggle_sort(),
            Command::StartSearch => self.start_search(),
        };

        match command_result {
            Ok(should_redraw) => should_redraw,
            Err(err) => {
                self.push_log(err.to_string());
                true
            }
        }
    }

    fn handle_job_update(&mut self, update: JobUpdate) -> bool {
        let needs_reload = update.status == JobStatus::Done;
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
            if update.destination.is_some() {
                job.destination = update.destination.clone();
            }
        } else {
            self.state.jobs.push(update.into_job());
        }

        self.push_log(next_status_line);

        if needs_reload {
            if let Err(err) = self.reload_panel(PanelId::Left, false) {
                self.push_log(format!("refresh left failed: {err}"));
            }
            if let Err(err) = self.reload_panel(PanelId::Right, false) {
                self.push_log(format!("refresh right failed: {err}"));
            }
        }

        true
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

    fn reload_panel(&mut self, panel_id: PanelId, update_status: bool) -> Result<bool> {
        let (cwd, sort_mode, show_hidden) = {
            let panel = self.panel_mut(panel_id);
            (panel.cwd.clone(), panel.sort_mode, panel.show_hidden)
        };
        match self.fs.list_dir(&cwd, sort_mode, show_hidden) {
            Ok(entries) => {
                let panel = self.panel_mut(panel_id);
                panel.set_entries(entries);
                panel.error_message = None;
                if update_status {
                    self.state.status_line = format!("Loaded {}", cwd.display());
                }
                Ok(true)
            }
            Err(err) => {
                let panel = self.panel_mut(panel_id);
                panel.all_entries.clear();
                panel.entries.clear();
                panel.selected_index = 0;
                panel.error_message = Some(err.to_string());
                Err(err.into())
            }
        }
    }

    fn open_selected_directory(&mut self) -> Result<bool> {
        let selected = self.active_panel().selected_entry().cloned();
        let Some(entry) = selected else {
            return Ok(false);
        };

        if entry.entry_type != FsEntryType::Directory {
            self.push_log(format!("{} is not a directory", entry.name));
            return Ok(true);
        }

        let next_path = self.fs.normalize_existing_path("open", &entry.path)?;
        self.active_panel_mut().cwd = next_path;
        self.reload_panel(self.state.active_panel, true)
    }

    fn go_to_parent(&mut self) -> Result<bool> {
        let current = self.active_panel().cwd.clone();
        let Some(parent) = current.parent() else {
            return Ok(false);
        };

        let normalized = self.fs.normalize_existing_path("parent", parent)?;
        self.active_panel_mut().cwd = normalized;
        self.reload_panel(self.state.active_panel, true)
    }

    fn go_to_home(&mut self) -> Result<bool> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("HOME environment variable is not set"))?;
        let normalized = self.fs.normalize_existing_path("home", &home)?;
        self.active_panel_mut().cwd = normalized;
        self.reload_panel(self.state.active_panel, true)
    }

    fn toggle_sort(&mut self) -> Result<bool> {
        let next_sort = self.active_panel().sort_mode.next();
        self.active_panel_mut().sort_mode = next_sort;
        self.reload_panel(self.state.active_panel, true)
    }

    fn start_search(&mut self) -> Result<bool> {
        let panel_id = self.state.active_panel;
        self.input_mode = Some(InputMode::Search(panel_id));
        let query = self.panel_mut(panel_id).search_query.clone();
        if query.is_empty() {
            self.state.status_line = "search: type to filter, Enter apply, Esc clear".to_string();
        } else {
            self.state.status_line = format!("search: {query}");
        }
        Ok(true)
    }

    fn queue_copy(&mut self) -> Result<bool> {
        let selected = self.selected_action_target_path()?;
        let target = self.inactive_panel_cwd();
        self.enqueue_job(JobKind::Copy, selected, Some(target), "copy queued")
    }

    fn queue_move(&mut self) -> Result<bool> {
        let selected = self.selected_action_target_path()?;
        let target = self.inactive_panel_cwd();
        self.enqueue_job(JobKind::Move, selected, Some(target), "move queued")
    }

    fn queue_delete(&mut self) -> Result<bool> {
        let (path, name) = self.selected_action_target()?;
        self.pending_confirmation = Some(PendingConfirmation::Delete {
            path,
            name: name.clone(),
        });
        self.state.confirm_prompt = Some(format!("Delete '{name}' permanently? [y/N]"));
        Ok(true)
    }

    fn queue_mkdir(&mut self) -> Result<bool> {
        let target = self.find_available_directory_name(self.active_panel().cwd.clone(), "new_dir");
        self.enqueue_job(JobKind::Mkdir, target, None, "mkdir queued")
    }

    fn enqueue_job(
        &mut self,
        kind: JobKind,
        source: PathBuf,
        destination: Option<PathBuf>,
        queued_message: impl Into<String>,
    ) -> Result<bool> {
        let queued_message = queued_message.into();
        let request = JobRequest {
            id: self.next_job_id,
            kind,
            source: source.clone(),
            destination: destination.clone(),
        };
        self.next_job_id += 1;

        self.state.jobs.push(Job {
            id: request.id,
            kind: request.kind,
            status: JobStatus::Queued,
            source,
            destination,
            message: Some(queued_message.to_string()),
        });
        self.push_log(queued_message);
        self.workers.submit(request)?;
        Ok(true)
    }

    fn inactive_panel_cwd(&self) -> PathBuf {
        match self.state.active_panel {
            PanelId::Left => self.state.right_panel.cwd.clone(),
            PanelId::Right => self.state.left_panel.cwd.clone(),
        }
    }

    fn find_available_directory_name(&self, base_dir: PathBuf, stem: &str) -> PathBuf {
        let mut candidate = base_dir.join(stem);
        let mut index = 1_u32;
        while candidate.exists() {
            candidate = base_dir.join(format!("{stem}_{index}"));
            index += 1;
        }
        candidate
    }

    fn selected_action_target(&self) -> Result<(PathBuf, String)> {
        let entry = self
            .active_panel()
            .selected_entry()
            .ok_or_else(|| anyhow::anyhow!("no selected entry"))?;
        if entry.is_virtual {
            return Err(anyhow::anyhow!(
                "action is not allowed for navigation entry '{}'",
                entry.name
            ));
        }

        Ok((entry.path.clone(), entry.name.clone()))
    }

    fn selected_action_target_path(&self) -> Result<PathBuf> {
        let (path, _) = self.selected_action_target()?;
        Ok(path)
    }

    fn handle_confirmation_input(&mut self, key: &KeyEvent) -> Option<bool> {
        self.pending_confirmation.as_ref()?;

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let confirmation = self.pending_confirmation.take();
                self.state.confirm_prompt = None;
                if let Some(PendingConfirmation::Delete { path, name }) = confirmation {
                    let result = self.enqueue_job(
                        JobKind::Delete,
                        path,
                        None,
                        format!("delete queued: {name}"),
                    );
                    return Some(match result {
                        Ok(redraw) => redraw,
                        Err(err) => {
                            self.push_log(err.to_string());
                            true
                        }
                    });
                }
                Some(true)
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                self.pending_confirmation = None;
                self.state.confirm_prompt = None;
                self.push_log("delete canceled");
                Some(true)
            }
            _ => Some(false),
        }
    }

    fn handle_search_input(&mut self, key: &KeyEvent) -> Option<bool> {
        let panel_id = match self.input_mode {
            Some(InputMode::Search(panel_id)) => panel_id,
            None => return None,
        };

        match key.code {
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                let panel = self.panel_mut(panel_id);
                panel.search_query.push(c);
                panel.apply_search_filter();
                self.state.status_line = format!("search: {}", panel.search_query);
                Some(true)
            }
            KeyCode::Backspace => {
                let panel = self.panel_mut(panel_id);
                panel.search_query.pop();
                panel.apply_search_filter();
                if panel.search_query.is_empty() {
                    self.state.status_line =
                        "search: type to filter, Enter apply, Esc clear".to_string();
                } else {
                    self.state.status_line = format!("search: {}", panel.search_query);
                }
                Some(true)
            }
            KeyCode::Esc => {
                self.input_mode = None;
                let panel = self.panel_mut(panel_id);
                panel.clear_search();
                self.state.status_line = "search cleared".to_string();
                Some(true)
            }
            KeyCode::Enter => {
                self.input_mode = None;
                let query = self.panel_mut(panel_id).search_query.clone();
                if query.is_empty() {
                    self.state.status_line = "search off".to_string();
                } else {
                    self.state.status_line = format!("search applied: {query}");
                }
                Some(true)
            }
            _ => Some(false),
        }
    }

    fn push_log(&mut self, message: impl Into<String>) {
        const MAX_ACTIVITY_LOG: usize = 16;

        let message = message.into();
        self.state.status_line = message.clone();
        self.state.activity_log.push(message);
        if self.state.activity_log.len() > MAX_ACTIVITY_LOG {
            self.state.activity_log.remove(0);
        }
    }
}

fn map_key_to_command(key: &KeyEvent) -> Option<Command> {
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
        KeyCode::Char('/') if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Some(Command::StartSearch)
        }
        KeyCode::Char('~') => Some(Command::GoHome),
        KeyCode::Home => Some(Command::GoHome),
        _ => None,
    }
}
