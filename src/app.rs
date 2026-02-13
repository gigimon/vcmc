use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use crossbeam_channel::Sender;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::fs::FsAdapter;
use crate::jobs::WorkerPool;
use crate::model::{
    AppState, Command, DialogButton, DialogButtonRole, DialogState, DialogTone, Event, FsEntry,
    FsEntryType, Job, JobKind, JobRequest, JobStatus, JobUpdate, PanelId, TerminalSize,
};

pub struct App {
    state: AppState,
    running: bool,
    fs: FsAdapter,
    workers: WorkerPool,
    next_job_id: u64,
    next_batch_id: u64,
    pending_confirmation: Option<PendingConfirmation>,
    pending_rename: Option<PendingRename>,
    pending_mask: Option<PendingMask>,
    batch_progress: HashMap<u64, BatchProgress>,
    input_mode: Option<InputMode>,
}

enum PendingConfirmation {
    DeleteOne {
        path: PathBuf,
        name: String,
        is_directory: bool,
    },
    Batch(BatchPlan),
}

struct PendingRename {
    kind: JobKind,
    source_path: PathBuf,
    source_name: String,
    destination_dir: PathBuf,
}

struct PendingMask {
    panel_id: PanelId,
    select: bool,
}

#[derive(Clone)]
struct BatchOpItem {
    source: PathBuf,
    destination: Option<PathBuf>,
    name: String,
}

struct BatchPlan {
    batch_id: u64,
    kind: JobKind,
    items: Vec<BatchOpItem>,
    summary: String,
}

struct BatchProgress {
    kind: JobKind,
    total: usize,
    completed: usize,
    failed: usize,
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
            next_batch_id: 1,
            pending_confirmation: None,
            pending_rename: None,
            pending_mask: None,
            batch_progress: HashMap::new(),
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
                if let Some(redraw) = self.handle_dialog_input(&key) {
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
                self.pending_confirmation = None;
                self.pending_rename = None;
                self.pending_mask = None;
                self.state.dialog = None;
                Ok(true)
            }
            Command::MoveSelectionUp => {
                self.active_panel_mut().move_selection_up();
                self.active_panel_mut().clear_selection_anchor();
                Ok(true)
            }
            Command::MoveSelectionDown => {
                self.active_panel_mut().move_selection_down();
                self.active_panel_mut().clear_selection_anchor();
                Ok(true)
            }
            Command::SelectRangeUp => self.select_range_up(),
            Command::SelectRangeDown => self.select_range_down(),
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
            Command::ToggleSelectCurrent => self.toggle_select_current(),
            Command::StartSelectByMask => self.start_mask_prompt(true),
            Command::StartDeselectByMask => self.start_mask_prompt(false),
            Command::InvertSelection => self.invert_selection(),
        };

        match command_result {
            Ok(should_redraw) => should_redraw,
            Err(err) => {
                self.show_alert(err.to_string());
                true
            }
        }
    }

    fn handle_job_update(&mut self, update: JobUpdate) -> bool {
        if let Some(batch_id) = update.batch_id {
            return self.handle_batch_job_update(batch_id, update);
        }

        let needs_reload = update.status == JobStatus::Done;
        let has_failed = update.status == JobStatus::Failed;
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

        self.upsert_job(update);

        if has_failed {
            self.show_alert(next_status_line);
        } else {
            self.push_log(next_status_line);
        }

        if needs_reload {
            if let Err(err) = self.reload_panel(PanelId::Left, false) {
                self.show_alert(format!("refresh left failed: {err}"));
            }
            if let Err(err) = self.reload_panel(PanelId::Right, false) {
                self.show_alert(format!("refresh right failed: {err}"));
            }
        }

        true
    }

    fn handle_batch_job_update(&mut self, batch_id: u64, update: JobUpdate) -> bool {
        let has_failed = update.status == JobStatus::Failed;
        let is_terminal = matches!(update.status, JobStatus::Done | JobStatus::Failed);
        let message = update
            .message
            .clone()
            .unwrap_or_else(|| "batch job updated".to_string());
        self.upsert_job(update);

        let mut should_log_failure = false;
        let mut finished: Option<(JobKind, usize, usize)> = None;

        if let Some(progress) = self.batch_progress.get_mut(&batch_id) {
            if is_terminal {
                progress.completed = progress.completed.saturating_add(1);
                if has_failed {
                    progress.failed = progress.failed.saturating_add(1);
                }
            }

            if has_failed {
                should_log_failure = true;
            }

            if progress.completed >= progress.total {
                finished = Some((progress.kind, progress.total, progress.failed));
            }
        }

        if should_log_failure {
            self.push_log(message);
        }

        if let Some((kind, total, failed)) = finished {
            self.batch_progress.remove(&batch_id);
            if failed > 0 {
                self.show_alert(format!(
                    "batch {} finished: total {} / failed {}",
                    operation_name(kind),
                    total,
                    failed
                ));
            } else {
                self.push_log(format!(
                    "batch {} finished: {} item(s)",
                    operation_name(kind),
                    total
                ));
            }

            if let Err(err) = self.reload_panel(PanelId::Left, false) {
                self.show_alert(format!("refresh left failed: {err}"));
            }
            if let Err(err) = self.reload_panel(PanelId::Right, false) {
                self.show_alert(format!("refresh right failed: {err}"));
            }
        }

        true
    }

    fn upsert_job(&mut self, update: JobUpdate) {
        if let Some(job) = self.state.jobs.iter_mut().find(|job| job.id == update.id) {
            job.status = update.status;
            job.message = update.message.clone();
            job.batch_id = update.batch_id;
            if update.destination.is_some() {
                job.destination = update.destination.clone();
            }
        } else {
            self.state.jobs.push(update.into_job());
        }
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
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.state.dialog = None;
        self.input_mode = Some(InputMode::Search(panel_id));
        let query = self.panel_mut(panel_id).search_query.clone();
        if query.is_empty() {
            self.state.status_line = "search: type to filter, Enter apply, Esc clear".to_string();
        } else {
            self.state.status_line = format!("search: {query}");
        }
        Ok(true)
    }

    fn toggle_select_current(&mut self) -> Result<bool> {
        let panel = self.active_panel_mut();
        let changed = panel.toggle_current_selection();
        panel.clear_selection_anchor();
        self.update_selection_status();
        Ok(changed)
    }

    fn select_range_up(&mut self) -> Result<bool> {
        let panel = self.active_panel_mut();
        let previous = panel.selected_index;
        panel.move_selection_up();
        let current = panel.selected_index;
        let changed = panel.select_range_from_anchor(previous, current);
        self.update_selection_status();
        Ok(changed > 0 || previous != current)
    }

    fn select_range_down(&mut self) -> Result<bool> {
        let panel = self.active_panel_mut();
        let previous = panel.selected_index;
        panel.move_selection_down();
        let current = panel.selected_index;
        let changed = panel.select_range_from_anchor(previous, current);
        self.update_selection_status();
        Ok(changed > 0 || previous != current)
    }

    fn start_mask_prompt(&mut self, select: bool) -> Result<bool> {
        let panel_id = self.state.active_panel;
        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = Some(PendingMask { panel_id, select });
        let title = if select {
            "Select by mask"
        } else {
            "Deselect by mask"
        };
        self.state.dialog = Some(input_dialog(
            title,
            "Wildcard: * and ?",
            "*".to_string(),
            DialogTone::Warning,
        ));
        self.state.status_line = if select {
            "select by mask".to_string()
        } else {
            "deselect by mask".to_string()
        };
        Ok(true)
    }

    fn invert_selection(&mut self) -> Result<bool> {
        let changed = self.active_panel_mut().invert_selection();
        self.active_panel_mut().clear_selection_anchor();
        self.update_selection_status();
        Ok(changed > 0)
    }

    fn queue_copy(&mut self) -> Result<bool> {
        if let Some(plan) = self.build_batch_plan_from_selection(JobKind::Copy)? {
            self.state.dialog = Some(confirm_dialog(plan.summary.clone()));
            self.pending_confirmation = Some(PendingConfirmation::Batch(plan));
            return Ok(true);
        }

        let entry = self.selected_action_target_entry()?;
        self.open_rename_prompt(JobKind::Copy, &entry)
    }

    fn queue_move(&mut self) -> Result<bool> {
        if let Some(plan) = self.build_batch_plan_from_selection(JobKind::Move)? {
            self.state.dialog = Some(confirm_dialog(plan.summary.clone()));
            self.pending_confirmation = Some(PendingConfirmation::Batch(plan));
            return Ok(true);
        }

        let entry = self.selected_action_target_entry()?;
        self.open_rename_prompt(JobKind::Move, &entry)
    }

    fn queue_delete(&mut self) -> Result<bool> {
        if let Some(plan) = self.build_batch_plan_from_selection(JobKind::Delete)? {
            self.state.dialog = Some(confirm_dialog(plan.summary.clone()));
            self.pending_confirmation = Some(PendingConfirmation::Batch(plan));
            return Ok(true);
        }

        let entry = self.selected_action_target_entry()?;
        let path = self.fs.normalize_existing_path("delete", &entry.path)?;
        self.guard_delete_target(&path)?;

        self.pending_confirmation = Some(PendingConfirmation::DeleteOne {
            path,
            name: entry.name.clone(),
            is_directory: entry.entry_type == FsEntryType::Directory,
        });
        self.state.dialog = Some(confirm_dialog(
            if entry.entry_type == FsEntryType::Directory {
                format!(
                    "Delete directory '{}' recursively and permanently?",
                    entry.name
                )
            } else {
                format!("Delete '{}' permanently?", entry.name)
            },
        ));
        Ok(true)
    }

    fn build_batch_plan_from_selection(&mut self, kind: JobKind) -> Result<Option<BatchPlan>> {
        let selected_entries = self.active_panel_selected_entries();
        if selected_entries.is_empty() {
            return Ok(None);
        }

        let destination_dir = self.inactive_panel_cwd();
        let mut unique_sources = HashSet::new();
        let mut unique_destinations = HashSet::new();
        let mut items = Vec::with_capacity(selected_entries.len());
        let mut total_bytes = 0u64;
        let mut dir_count = 0usize;

        for entry in selected_entries {
            if entry.is_virtual {
                return Err(anyhow::anyhow!(
                    "virtual entries cannot be used in batch operations"
                ));
            }

            let source = self.fs.normalize_existing_path("batch", &entry.path)?;
            if !unique_sources.insert(source.clone()) {
                return Err(anyhow::anyhow!(
                    "duplicate source in batch: {}",
                    source.display()
                ));
            }

            let destination = match kind {
                JobKind::Copy | JobKind::Move => {
                    let target = destination_dir.join(&entry.name);
                    if source == target {
                        return Err(anyhow::anyhow!(
                            "batch {} target equals source for {}",
                            operation_name(kind),
                            source.display()
                        ));
                    }
                    if !unique_destinations.insert(target.clone()) {
                        return Err(anyhow::anyhow!(
                            "duplicate destination in batch: {}",
                            target.display()
                        ));
                    }
                    if target.try_exists()? {
                        return Err(anyhow::anyhow!(
                            "destination already exists: {}",
                            target.display()
                        ));
                    }
                    Some(target)
                }
                JobKind::Delete | JobKind::Mkdir => None,
            };

            if kind == JobKind::Delete {
                self.guard_delete_target(&source)?;
            }

            total_bytes = total_bytes.saturating_add(entry.size_bytes);
            if entry.entry_type == FsEntryType::Directory {
                dir_count += 1;
            }
            items.push(BatchOpItem {
                source,
                destination,
                name: entry.name,
            });
        }

        let batch_id = self.next_batch_id;
        self.next_batch_id += 1;
        let summary = batch_summary(kind, items.len(), total_bytes, dir_count, &destination_dir);

        Ok(Some(BatchPlan {
            batch_id,
            kind,
            items,
            summary,
        }))
    }

    fn active_panel_selected_entries(&self) -> Vec<FsEntry> {
        let panel = self.active_panel();
        if panel.selected_paths.is_empty() {
            return Vec::new();
        }

        panel
            .all_entries
            .iter()
            .filter(|entry| panel.selected_paths.contains(&entry.path))
            .cloned()
            .collect()
    }

    fn queue_mkdir(&mut self) -> Result<bool> {
        let target = self.find_available_directory_name(self.active_panel().cwd.clone(), "new_dir");
        self.enqueue_job(JobKind::Mkdir, target, None, "mkdir queued")
    }

    fn open_rename_prompt(&mut self, kind: JobKind, entry: &FsEntry) -> Result<bool> {
        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_mask = None;
        let destination_dir = self.inactive_panel_cwd();
        self.pending_rename = Some(PendingRename {
            kind,
            source_path: entry.path.clone(),
            source_name: entry.name.clone(),
            destination_dir,
        });

        let verb = match kind {
            JobKind::Copy => "Copy as",
            JobKind::Move => "Move as",
            _ => "Rename as",
        };
        let body = match kind {
            JobKind::Copy => format!("Copy '{}' with new name:", entry.name),
            JobKind::Move => format!("Move '{}' with new name:", entry.name),
            _ => "Edit target name:".to_string(),
        };
        self.state.dialog = Some(input_dialog(
            verb,
            body.as_str(),
            entry.name.clone(),
            DialogTone::Default,
        ));
        self.state.status_line = format!("{verb}: {}", entry.name);
        Ok(true)
    }

    fn enqueue_job(
        &mut self,
        kind: JobKind,
        source: PathBuf,
        destination: Option<PathBuf>,
        queued_message: impl Into<String>,
    ) -> Result<bool> {
        self.enqueue_job_with_options(kind, source, destination, None, queued_message, true)
    }

    fn enqueue_job_with_options(
        &mut self,
        kind: JobKind,
        source: PathBuf,
        destination: Option<PathBuf>,
        batch_id: Option<u64>,
        queued_message: impl Into<String>,
        log_message: bool,
    ) -> Result<bool> {
        let queued_message = queued_message.into();
        let request = JobRequest {
            id: self.next_job_id,
            batch_id,
            kind,
            source: source.clone(),
            destination: destination.clone(),
        };
        self.next_job_id += 1;

        self.state.jobs.push(Job {
            id: request.id,
            batch_id,
            kind: request.kind,
            status: JobStatus::Queued,
            source,
            destination,
            message: Some(queued_message.to_string()),
        });
        if log_message {
            self.push_log(queued_message);
        }
        self.workers.submit(request)?;
        Ok(true)
    }

    fn execute_batch_plan(&mut self, plan: BatchPlan) -> Result<bool> {
        let total = plan.items.len();
        let kind = plan.kind;
        let batch_id = plan.batch_id;
        if total == 0 {
            return Ok(false);
        }

        self.batch_progress.insert(
            batch_id,
            BatchProgress {
                kind,
                total,
                completed: 0,
                failed: 0,
            },
        );

        for item in plan.items {
            self.enqueue_job_with_options(
                kind,
                item.source,
                item.destination,
                Some(batch_id),
                format!("{} queued: {}", operation_name(kind), item.name),
                false,
            )?;
        }

        self.push_log(format!(
            "batch {} queued: {} item(s)",
            operation_name(kind),
            total
        ));
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

    fn selected_action_target_entry(&self) -> Result<FsEntry> {
        let entry = self
            .active_panel()
            .selected_entry()
            .ok_or_else(|| anyhow::anyhow!("no selected entry"))?
            .clone();
        if entry.is_virtual {
            return Err(anyhow::anyhow!(
                "action is not allowed for navigation entry '{}'",
                entry.name
            ));
        }

        Ok(entry)
    }

    fn guard_delete_target(&self, target: &PathBuf) -> Result<()> {
        if target == Path::new("/") {
            return Err(anyhow::anyhow!(
                "refusing to delete root directory '/' in interactive mode"
            ));
        }

        if let Some(home) = std::env::var_os("HOME") {
            let home_path = self
                .fs
                .normalize_existing_path("delete", &PathBuf::from(home))?;
            if *target == home_path {
                return Err(anyhow::anyhow!(
                    "refusing to delete HOME directory: {}",
                    home_path.display()
                ));
            }
        }

        Ok(())
    }

    fn handle_dialog_input(&mut self, key: &KeyEvent) -> Option<bool> {
        self.state.dialog.as_ref()?;

        if let Some(accel) = accelerator_from_key(key) {
            if let Some(role) = self.find_dialog_button_by_accelerator(accel) {
                return Some(self.activate_dialog_button(role));
            }
        }

        match key.code {
            KeyCode::Esc => Some(self.cancel_dialog()),
            KeyCode::Tab | KeyCode::Right => {
                if let Some(dialog) = self.state.dialog.as_mut() {
                    dialog.focus_next();
                }
                Some(true)
            }
            KeyCode::BackTab | KeyCode::Left => {
                if let Some(dialog) = self.state.dialog.as_mut() {
                    dialog.focus_prev();
                }
                Some(true)
            }
            KeyCode::Enter => {
                let role = self
                    .state
                    .dialog
                    .as_ref()
                    .and_then(DialogState::focused_button)
                    .map(|button| button.role)
                    .unwrap_or(DialogButtonRole::Primary);
                Some(self.activate_dialog_button(role))
            }
            KeyCode::Backspace => Some(self.edit_dialog_input_backspace()),
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                Some(self.edit_dialog_input_char(c))
            }
            _ => Some(false),
        }
    }

    fn activate_dialog_button(&mut self, role: DialogButtonRole) -> bool {
        if self.pending_confirmation.is_some() {
            return if role == DialogButtonRole::Primary {
                self.apply_confirmation()
            } else {
                self.pending_confirmation = None;
                self.state.dialog = None;
                self.push_log("operation canceled");
                true
            };
        }

        if self.pending_rename.is_some() {
            return if role == DialogButtonRole::Primary {
                self.apply_rename()
            } else {
                self.pending_rename = None;
                self.state.dialog = None;
                self.push_log("copy/move canceled");
                true
            };
        }

        if self.pending_mask.is_some() {
            return if role == DialogButtonRole::Primary {
                self.apply_mask()
            } else {
                self.pending_mask = None;
                self.state.dialog = None;
                self.push_log("mask selection canceled");
                true
            };
        }

        self.state.dialog = None;
        true
    }

    fn cancel_dialog(&mut self) -> bool {
        if self.pending_confirmation.is_some() {
            self.pending_confirmation = None;
            self.state.dialog = None;
            self.push_log("operation canceled");
            return true;
        }

        if self.pending_rename.is_some() {
            self.pending_rename = None;
            self.state.dialog = None;
            self.push_log("copy/move canceled");
            return true;
        }

        if self.pending_mask.is_some() {
            self.pending_mask = None;
            self.state.dialog = None;
            self.push_log("mask selection canceled");
            return true;
        }

        self.state.dialog = None;
        true
    }

    fn apply_confirmation(&mut self) -> bool {
        let confirmation = self.pending_confirmation.take();
        self.state.dialog = None;
        if let Some(confirmation) = confirmation {
            let result = match confirmation {
                PendingConfirmation::DeleteOne {
                    path,
                    name,
                    is_directory,
                } => {
                    let description = if is_directory {
                        format!("delete queued (recursive): {name}")
                    } else {
                        format!("delete queued: {name}")
                    };
                    self.enqueue_job(JobKind::Delete, path, None, description)
                }
                PendingConfirmation::Batch(plan) => self.execute_batch_plan(plan),
            };

            return match result {
                Ok(redraw) => redraw,
                Err(err) => {
                    self.show_alert(err.to_string());
                    true
                }
            };
        }

        true
    }

    fn apply_rename(&mut self) -> bool {
        let pending = self.pending_rename.take();
        let requested_name = self
            .state
            .dialog
            .as_ref()
            .and_then(|dialog| dialog.input_value.as_ref())
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        self.state.dialog = None;

        let Some(pending) = pending else {
            return true;
        };

        if requested_name.is_empty() {
            self.show_alert("name cannot be empty");
            return true;
        }
        if requested_name.contains('/') {
            self.show_alert("name cannot contain '/'");
            return true;
        }

        let destination = pending.destination_dir.join(&requested_name);
        let verb = if pending.kind == JobKind::Copy {
            "copy queued"
        } else {
            "move queued"
        };
        let message = if requested_name == pending.source_name {
            format!("{verb}: {}", pending.source_name)
        } else {
            format!("{verb}: {} -> {}", pending.source_name, requested_name)
        };

        match self.enqueue_job(
            pending.kind,
            pending.source_path,
            Some(destination),
            message,
        ) {
            Ok(redraw) => redraw,
            Err(err) => {
                self.show_alert(err.to_string());
                true
            }
        }
    }

    fn apply_mask(&mut self) -> bool {
        let pending = self.pending_mask.take();
        let mask = self
            .state
            .dialog
            .as_ref()
            .and_then(|dialog| dialog.input_value.clone())
            .unwrap_or_default();
        self.state.dialog = None;

        let Some(pending) = pending else {
            return true;
        };

        let changed = if pending.select {
            self.panel_mut(pending.panel_id)
                .select_by_mask(mask.as_str())
        } else {
            self.panel_mut(pending.panel_id)
                .deselect_by_mask(mask.as_str())
        };
        self.update_selection_status();
        self.push_log(if pending.select {
            format!("selected {changed} by mask")
        } else {
            format!("deselected {changed} by mask")
        });
        true
    }

    fn edit_dialog_input_backspace(&mut self) -> bool {
        if self.pending_rename.is_none() && self.pending_mask.is_none() {
            return false;
        }
        if let Some(dialog) = self.state.dialog.as_mut() {
            if let Some(value) = dialog.input_value.as_mut() {
                value.pop();
                return true;
            }
        }
        false
    }

    fn edit_dialog_input_char(&mut self, c: char) -> bool {
        if self.pending_rename.is_none() && self.pending_mask.is_none() {
            return false;
        }
        if c == '\0' {
            return false;
        }

        if self.pending_rename.is_some() && c == '/' {
            return true;
        }

        if let Some(dialog) = self.state.dialog.as_mut() {
            if let Some(value) = dialog.input_value.as_mut() {
                value.push(c);
                return true;
            }
        }
        false
    }

    fn find_dialog_button_by_accelerator(&self, accelerator: char) -> Option<DialogButtonRole> {
        let normalized = accelerator.to_ascii_lowercase();
        self.state.dialog.as_ref().and_then(|dialog| {
            dialog
                .buttons
                .iter()
                .find(|button| {
                    button.accelerator.map(|c| c.to_ascii_lowercase()) == Some(normalized)
                })
                .map(|button| button.role)
        })
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

    fn update_selection_status(&mut self) {
        let panel = self.active_panel();
        let (count, bytes) = panel.selection_summary();
        if count == 0 {
            self.state.status_line = "selection: none".to_string();
        } else {
            self.state.status_line = format!("selection: {count} item(s), {}", format_bytes(bytes));
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

    fn show_alert(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.state.dialog = Some(alert_dialog(message.clone()));
        self.push_log(message);
    }
}

fn map_key_to_command(key: &KeyEvent) -> Option<Command> {
    match key.code {
        KeyCode::Char('q') => Some(Command::Quit),
        KeyCode::Tab => Some(Command::SwitchPanel),
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => Some(Command::SelectRangeUp),
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(Command::SelectRangeDown)
        }
        KeyCode::Up => Some(Command::MoveSelectionUp),
        KeyCode::Down => Some(Command::MoveSelectionDown),
        KeyCode::Enter => Some(Command::OpenSelected),
        KeyCode::Backspace => Some(Command::GoToParent),
        KeyCode::Char(' ') | KeyCode::Insert => Some(Command::ToggleSelectCurrent),
        KeyCode::Char('+') => Some(Command::StartSelectByMask),
        KeyCode::Char('-') => Some(Command::StartDeselectByMask),
        KeyCode::Char('*') => Some(Command::InvertSelection),
        KeyCode::F(5) => Some(Command::Copy),
        KeyCode::F(6) => Some(Command::Move),
        KeyCode::F(7) => Some(Command::Mkdir),
        KeyCode::F(8) => Some(Command::Delete),
        KeyCode::F(10) => Some(Command::Quit),
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

fn operation_name(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Copy => "copy",
        JobKind::Move => "move",
        JobKind::Delete => "delete",
        JobKind::Mkdir => "mkdir",
    }
}

fn batch_summary(
    kind: JobKind,
    count: usize,
    total_bytes: u64,
    dir_count: usize,
    destination_dir: &Path,
) -> String {
    match kind {
        JobKind::Copy => format!(
            "Copy {} item(s), {} to {}?",
            count,
            format_bytes(total_bytes),
            destination_dir.display()
        ),
        JobKind::Move => format!(
            "Move {} item(s), {} to {}?",
            count,
            format_bytes(total_bytes),
            destination_dir.display()
        ),
        JobKind::Delete => {
            if dir_count > 0 {
                format!(
                    "Delete {} item(s), {} (includes {} dir) permanently?",
                    count,
                    format_bytes(total_bytes),
                    dir_count
                )
            } else {
                format!(
                    "Delete {} item(s), {} permanently?",
                    count,
                    format_bytes(total_bytes)
                )
            }
        }
        JobKind::Mkdir => format!("Run mkdir batch for {} item(s)? [y/N]", count),
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut size = bytes as f64;
    let mut idx = 0usize;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes}{}", UNITS[idx])
    } else {
        format!("{size:.1}{}", UNITS[idx])
    }
}

fn accelerator_from_key(key: &KeyEvent) -> Option<char> {
    if !key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    Some(c.to_ascii_lowercase())
}

fn input_dialog(title: &str, body: &str, value: String, tone: DialogTone) -> DialogState {
    DialogState {
        title: title.to_string(),
        body: body.to_string(),
        input_value: Some(value),
        buttons: vec![
            DialogButton {
                label: "Apply".to_string(),
                accelerator: Some('a'),
                role: DialogButtonRole::Primary,
            },
            DialogButton {
                label: "Cancel".to_string(),
                accelerator: Some('c'),
                role: DialogButtonRole::Secondary,
            },
        ],
        focused_button: 0,
        tone,
    }
}

fn confirm_dialog(body: String) -> DialogState {
    DialogState {
        title: "Confirm".to_string(),
        body,
        input_value: None,
        buttons: vec![
            DialogButton {
                label: "Yes".to_string(),
                accelerator: Some('y'),
                role: DialogButtonRole::Primary,
            },
            DialogButton {
                label: "No".to_string(),
                accelerator: Some('n'),
                role: DialogButtonRole::Secondary,
            },
        ],
        focused_button: 1,
        tone: DialogTone::Warning,
    }
}

fn alert_dialog(body: String) -> DialogState {
    DialogState {
        title: "Error".to_string(),
        body,
        input_value: None,
        buttons: vec![DialogButton {
            label: "OK".to_string(),
            accelerator: Some('o'),
            role: DialogButtonRole::Primary,
        }],
        focused_button: 0,
        tone: DialogTone::Danger,
    }
}
