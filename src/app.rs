use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossbeam_channel::Sender;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ssh2::Session;

use crate::backend::{FsBackend, backend_from_spec, is_archive_file_path};
use crate::find::{is_fd_available, parse_find_input, spawn_fd_search};
use crate::jobs::WorkerPool;
use crate::menu::{MenuAction, menu_group_index_by_hotkey, top_menu_groups};
use crate::model::{
    AppState, ArchiveConnectionInfo, BackendSpec, BatchProgressState, Command, DialogButton,
    DialogButtonRole, DialogState, DialogTone, Event, FindPanelState, FindProgressState,
    FindRequest, FindUpdate, FsEntry, FsEntryType, Job, JobKind, JobRequest, JobStatus, JobUpdate,
    PanelId, ScreenMode, SftpAuth, SftpConnectionInfo, SortMode, TerminalSize, ViewerMode,
    ViewerState,
};
use crate::theme::{DirColorsTheme, load_theme_from_environment};
use crate::viewer::{
    VIEWER_PREVIEW_LIMIT_BYTES, jump_to_next_match, load_viewer_state_from_preview,
    refresh_viewer_search, set_viewer_mode,
};
use crate::{runtime, terminal};

pub struct App {
    state: AppState,
    running: bool,
    event_tx: Sender<Event>,
    workers: WorkerPool,
    left_backend: Arc<dyn FsBackend>,
    right_backend: Arc<dyn FsBackend>,
    left_backend_spec: BackendSpec,
    right_backend_spec: BackendSpec,
    theme: DirColorsTheme,
    next_job_id: u64,
    next_batch_id: u64,
    next_find_id: u64,
    pending_confirmation: Option<PendingConfirmation>,
    pending_rename: Option<PendingRename>,
    pending_mask: Option<PendingMask>,
    pending_sftp_connect: Option<PendingSftpConnect>,
    pending_conflict: Option<PendingConflict>,
    pending_find: Option<PendingFind>,
    pending_editor_choice: Option<PendingEditorChoice>,
    pending_viewer_search: bool,
    batch_progress: HashMap<u64, BatchProgress>,
    left_active_find_id: Option<u64>,
    right_active_find_id: Option<u64>,
    force_full_redraw: bool,
    last_left_local_cwd: PathBuf,
    last_right_local_cwd: PathBuf,
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

struct PendingSftpConnect {
    panel_id: PanelId,
    stage: SftpConnectStage,
    draft: SftpConnectDraft,
}

struct PendingFind {
    panel_id: PanelId,
    root: PathBuf,
    default_hidden: bool,
}

struct PendingEditorChoice {
    context: EditorChoiceContext,
    options: Vec<EditorCandidate>,
}

enum EditorChoiceContext {
    OpenFile(PathBuf),
    SettingsOnly,
}

#[derive(Clone)]
struct EditorCandidate {
    label: String,
    command: String,
}

#[derive(Clone)]
struct SftpConnectDraft {
    host: String,
    port: u16,
    root_path: PathBuf,
    user: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SftpConnectStage {
    Address,
    Login,
    Password,
    KeyPath,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SftpAuthHint {
    PasswordOnly,
    KeyOnly,
    Either,
}

#[derive(Clone)]
struct BatchOpItem {
    source: PathBuf,
    destination: Option<PathBuf>,
    name: String,
    overwrite_destination: bool,
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
    current_file: String,
}

struct PendingConflict {
    kind: JobKind,
    batch_id: Option<u64>,
    items: Vec<BatchOpItem>,
    next_index: usize,
    ready: Vec<BatchOpItem>,
    skipped: usize,
    apply_all: Option<ConflictPolicy>,
}

#[derive(Clone, Copy)]
enum ConflictPolicy {
    Overwrite,
    Skip,
    OverwriteIfNewer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConflictAction {
    Overwrite,
    Skip,
    Rename,
    OverwriteIfNewer,
    OverwriteAll,
    SkipAll,
    OverwriteIfNewerAll,
    Cancel,
}

enum InputMode {
    Search(PanelId),
}

impl App {
    pub fn bootstrap(cwd: PathBuf, event_tx: Sender<Event>) -> Result<Self> {
        let local_spec = BackendSpec::Local;
        let local_backend = backend_from_spec(&local_spec);
        let normalized_cwd = local_backend.normalize_existing_path("bootstrap", &cwd)?;
        let workers = WorkerPool::new(2, event_tx.clone());
        let theme = load_theme_from_environment();
        let mut app = Self {
            state: AppState::new(normalized_cwd.clone()),
            running: true,
            event_tx,
            workers,
            left_backend: local_backend.clone(),
            right_backend: local_backend,
            left_backend_spec: local_spec.clone(),
            right_backend_spec: local_spec,
            theme,
            next_job_id: 1,
            next_batch_id: 1,
            next_find_id: 1,
            pending_confirmation: None,
            pending_rename: None,
            pending_mask: None,
            pending_sftp_connect: None,
            pending_conflict: None,
            pending_find: None,
            pending_editor_choice: None,
            pending_viewer_search: false,
            batch_progress: HashMap::new(),
            left_active_find_id: None,
            right_active_find_id: None,
            force_full_redraw: false,
            last_left_local_cwd: normalized_cwd.clone(),
            last_right_local_cwd: normalized_cwd,
            input_mode: None,
        };

        let _ = app.reload_panel(PanelId::Left, true)?;
        let _ = app.reload_panel(PanelId::Right, false)?;
        Ok(app)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn theme(&self) -> &DirColorsTheme {
        &self.theme
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn take_force_full_redraw(&mut self) -> bool {
        let redraw = self.force_full_redraw;
        self.force_full_redraw = false;
        redraw
    }

    pub fn on_event(&mut self, event: Event) -> bool {
        match event {
            Event::Input(key) => {
                if let Some(redraw) = self.handle_dialog_input(&key) {
                    return redraw;
                }

                if let Some(redraw) = self.handle_top_menu_input(&key) {
                    return redraw;
                }

                if self.state.screen_mode == ScreenMode::Viewer {
                    if let Some(cmd) = map_viewer_key_to_command(&key) {
                        return self.apply_command(cmd);
                    }
                    return false;
                }

                if let Some(redraw) = self.handle_search_input(&key) {
                    return redraw;
                }

                if let Some(redraw) = self.handle_command_line_input(&key) {
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
            Event::Find(update) => self.handle_find_update(update),
        }
    }

    fn apply_command(&mut self, command: Command) -> bool {
        let command_result: Result<bool> = match command {
            Command::Quit => {
                self.running = false;
                Ok(false)
            }
            Command::OpenTopMenu => self.open_top_menu(),
            Command::SwitchPanel => {
                self.state.active_panel = match self.state.active_panel {
                    PanelId::Left => PanelId::Right,
                    PanelId::Right => PanelId::Left,
                };
                self.state.top_menu.open = false;
                self.input_mode = None;
                self.pending_confirmation = None;
                self.pending_rename = None;
                self.pending_mask = None;
                self.pending_sftp_connect = None;
                self.pending_conflict = None;
                self.pending_find = None;
                self.pending_editor_choice = None;
                self.pending_viewer_search = false;
                self.state.dialog = None;
                Ok(true)
            }
            Command::ConnectSftp => self.handle_sftp_action(),
            Command::OpenShell => self.open_shell_mode(),
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
            Command::MoveSelectionTop => {
                self.active_panel_mut().selected_index = 0;
                self.active_panel_mut().clear_selection_anchor();
                Ok(true)
            }
            Command::MoveSelectionBottom => {
                let last_index = self.active_panel().entries.len().saturating_sub(1);
                if self.active_panel().entries.is_empty() {
                    self.active_panel_mut().selected_index = 0;
                } else {
                    self.active_panel_mut().selected_index = last_index;
                }
                self.active_panel_mut().clear_selection_anchor();
                Ok(true)
            }
            Command::OpenViewer => self.open_viewer(),
            Command::CloseViewer => self.close_viewer(),
            Command::ViewerScrollUp => self.viewer_scroll_up(),
            Command::ViewerScrollDown => self.viewer_scroll_down(),
            Command::ViewerPageUp => self.viewer_page_up(),
            Command::ViewerPageDown => self.viewer_page_down(),
            Command::ViewerTop => self.viewer_top(),
            Command::ViewerBottom => self.viewer_bottom(),
            Command::ViewerToggleMode => self.viewer_toggle_mode(),
            Command::ViewerStartSearch => self.viewer_start_search_prompt(),
            Command::ViewerSearchNext => self.viewer_search_next(),
            Command::ViewerSearchPrev => self.viewer_search_prev(),
            Command::OpenEditor => self.open_editor(),
            Command::SelectRangeUp => self.select_range_up(),
            Command::SelectRangeDown => self.select_range_down(),
            Command::Refresh => self.refresh_all(),
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

        if self.batch_progress.is_empty() {
            match update.status {
                JobStatus::Running => {
                    let total = update.batch_total.unwrap_or(1).max(1);
                    let completed = update.batch_completed.unwrap_or(0).min(total);
                    let current_file = update
                        .current_item
                        .clone()
                        .unwrap_or_else(|| source_item_label(&update.source));
                    self.state.batch_progress = Some(BatchProgressState {
                        batch_id: update.id,
                        operation: update.kind,
                        current_file,
                        completed,
                        total,
                        failed: 0,
                    });
                }
                JobStatus::Done | JobStatus::Failed => {
                    if self
                        .state
                        .batch_progress
                        .as_ref()
                        .is_some_and(|progress| progress.batch_id == update.id)
                    {
                        self.state.batch_progress = None;
                    }
                }
                JobStatus::Queued => {}
            }
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

    fn handle_find_update(&mut self, update: FindUpdate) -> bool {
        match update {
            FindUpdate::Progress {
                id,
                panel_id,
                query,
                matches,
            } => {
                if self.active_find_id(panel_id) != Some(id) {
                    return false;
                }
                self.state.find_progress = Some(FindProgressState {
                    panel_id,
                    query,
                    matches,
                    running: true,
                });
                self.state.status_line = format!("find running: {} match(es)", matches);
                true
            }
            FindUpdate::Done {
                id,
                panel_id,
                query,
                root,
                glob,
                hidden,
                follow_symlinks,
                entries,
            } => {
                if self.active_find_id(panel_id) != Some(id) {
                    return false;
                }
                self.set_active_find_id(panel_id, None);
                self.state.find_progress = Some(FindProgressState {
                    panel_id,
                    query: query.clone(),
                    matches: entries.len(),
                    running: false,
                });
                match self.apply_find_results(
                    panel_id,
                    root,
                    query,
                    glob,
                    hidden,
                    follow_symlinks,
                    entries,
                ) {
                    Ok(redraw) => redraw,
                    Err(err) => {
                        self.show_alert(format!("find failed: {err}"));
                        true
                    }
                }
            }
            FindUpdate::Failed {
                id,
                panel_id,
                query,
                error,
            } => {
                if self.active_find_id(panel_id) != Some(id) {
                    return false;
                }
                self.set_active_find_id(panel_id, None);
                self.state.find_progress = None;
                self.show_alert(format!("find '{query}' failed: {error}"));
                true
            }
        }
    }

    fn handle_batch_job_update(&mut self, batch_id: u64, mut update: JobUpdate) -> bool {
        let has_failed = update.status == JobStatus::Failed;
        let is_terminal = matches!(update.status, JobStatus::Done | JobStatus::Failed);
        let has_running = update.status == JobStatus::Running;
        let message = update
            .message
            .clone()
            .unwrap_or_else(|| "batch job updated".to_string());

        let mut should_log_failure = false;
        let mut finished: Option<(JobKind, usize, usize)> = None;

        if let Some(progress) = self.batch_progress.get_mut(&batch_id) {
            let item_label = update
                .current_item
                .clone()
                .unwrap_or_else(|| source_item_label(&update.source));
            if has_running || is_terminal {
                progress.current_file = item_label.clone();
            }

            if is_terminal {
                progress.completed = progress.completed.saturating_add(1);
                if has_failed {
                    progress.failed = progress.failed.saturating_add(1);
                }
            }

            if has_failed {
                should_log_failure = true;
            }

            update.current_item = Some(progress.current_file.clone());
            update.batch_completed = Some(progress.completed);
            update.batch_total = Some(progress.total);
            self.state.batch_progress = Some(BatchProgressState {
                batch_id,
                operation: progress.kind,
                current_file: progress.current_file.clone(),
                completed: progress.completed,
                total: progress.total,
                failed: progress.failed,
            });

            if progress.completed >= progress.total {
                finished = Some((progress.kind, progress.total, progress.failed));
            }
        }

        self.upsert_job(update);

        if should_log_failure {
            self.push_log(message);
        }

        if let Some((kind, total, failed)) = finished {
            self.batch_progress.remove(&batch_id);
            self.sync_visible_batch_progress(None);
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
            job.current_item = update.current_item.clone();
            job.batch_completed = update.batch_completed;
            job.batch_total = update.batch_total;
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

    fn panel(&self, id: PanelId) -> &crate::model::PanelState {
        match id {
            PanelId::Left => &self.state.left_panel,
            PanelId::Right => &self.state.right_panel,
        }
    }

    fn backend(&self, id: PanelId) -> &dyn FsBackend {
        match id {
            PanelId::Left => self.left_backend.as_ref(),
            PanelId::Right => self.right_backend.as_ref(),
        }
    }

    fn active_backend(&self) -> &dyn FsBackend {
        self.backend(self.state.active_panel)
    }

    fn inactive_backend(&self) -> &dyn FsBackend {
        match self.state.active_panel {
            PanelId::Left => self.right_backend.as_ref(),
            PanelId::Right => self.left_backend.as_ref(),
        }
    }

    fn backend_spec(&self, id: PanelId) -> &BackendSpec {
        match id {
            PanelId::Left => &self.left_backend_spec,
            PanelId::Right => &self.right_backend_spec,
        }
    }

    fn active_backend_spec(&self) -> &BackendSpec {
        self.backend_spec(self.state.active_panel)
    }

    fn inactive_backend_spec(&self) -> &BackendSpec {
        match self.state.active_panel {
            PanelId::Left => &self.right_backend_spec,
            PanelId::Right => &self.left_backend_spec,
        }
    }

    fn set_panel_backend(&mut self, id: PanelId, spec: BackendSpec) {
        let backend = backend_from_spec(&spec);
        let label = backend_spec_label(&spec);
        match id {
            PanelId::Left => {
                self.left_backend = backend;
                self.left_backend_spec = spec;
                self.state.left_panel.backend_label = label;
            }
            PanelId::Right => {
                self.right_backend = backend;
                self.right_backend_spec = spec;
                self.state.right_panel.backend_label = label;
            }
        }
    }

    fn set_last_local_cwd(&mut self, id: PanelId, cwd: PathBuf) {
        match id {
            PanelId::Left => self.last_left_local_cwd = cwd,
            PanelId::Right => self.last_right_local_cwd = cwd,
        }
    }

    fn last_local_cwd(&self, id: PanelId) -> PathBuf {
        match id {
            PanelId::Left => self.last_left_local_cwd.clone(),
            PanelId::Right => self.last_right_local_cwd.clone(),
        }
    }

    fn active_find_id(&self, panel_id: PanelId) -> Option<u64> {
        match panel_id {
            PanelId::Left => self.left_active_find_id,
            PanelId::Right => self.right_active_find_id,
        }
    }

    fn set_active_find_id(&mut self, panel_id: PanelId, value: Option<u64>) {
        match panel_id {
            PanelId::Left => self.left_active_find_id = value,
            PanelId::Right => self.right_active_find_id = value,
        }
    }

    fn path_exists_on_backend(&self, backend: &dyn FsBackend, path: &Path) -> bool {
        backend.stat_entry(path).is_ok()
    }

    fn reload_panel(&mut self, panel_id: PanelId, update_status: bool) -> Result<bool> {
        let (cwd, sort_mode, show_hidden, find_view) = {
            let panel = self.panel_mut(panel_id);
            (
                panel.cwd.clone(),
                panel.sort_mode,
                panel.show_hidden,
                panel.find_view.clone(),
            )
        };
        if let Some(find_view) = find_view {
            let panel = self.panel_mut(panel_id);
            panel.apply_search_filter();
            panel.error_message = None;
            if update_status {
                let mode = if find_view.glob { "glob" } else { "name" };
                self.state.status_line = format!(
                    "Find view ({mode}): '{}' @ {}",
                    find_view.query,
                    find_view.root.display()
                );
            }
            return Ok(true);
        }
        match self
            .backend(panel_id)
            .list_dir(&cwd, sort_mode, show_hidden)
        {
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

    fn reload_theme(&mut self) -> Result<bool> {
        self.theme = load_theme_from_environment();
        Ok(true)
    }

    fn refresh_all(&mut self) -> Result<bool> {
        self.reload_theme()
            .and_then(|_| self.reload_panel(PanelId::Left, true))
            .and_then(|_| self.reload_panel(PanelId::Right, false))
    }

    fn open_selected_directory(&mut self) -> Result<bool> {
        let panel_id = self.state.active_panel;
        let in_find_view = self.panel(panel_id).find_view.is_some();
        let selected = self.active_panel().selected_entry().cloned();
        let Some(entry) = selected else {
            return Ok(false);
        };

        if in_find_view {
            if entry.is_virtual && entry.name == ".." {
                return self.exit_find_view(panel_id);
            }
            return self.open_find_result_entry(panel_id, entry);
        }

        if entry.is_virtual
            && entry.name == ".."
            && matches!(self.active_backend_spec(), BackendSpec::Archive(_))
            && self.active_panel().cwd == Path::new("/")
        {
            return self.detach_archive_panel(panel_id);
        }

        if entry.entry_type != FsEntryType::Directory && !entry.is_virtual {
            if matches!(self.active_backend_spec(), BackendSpec::Local)
                && is_archive_file_path(entry.path.as_path())
            {
                return self.attach_panel_to_archive(self.state.active_panel, entry.path.clone());
            }
        }

        if entry.entry_type != FsEntryType::Directory {
            self.push_log(format!("{} is not a directory", entry.name));
            return Ok(true);
        }

        let next_path = self
            .active_backend()
            .normalize_existing_path("open", &entry.path)?;
        let panel = self.active_panel_mut();
        panel.cwd = next_path;
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        self.reload_panel(self.state.active_panel, true)
    }

    fn go_to_parent(&mut self) -> Result<bool> {
        let panel_id = self.state.active_panel;
        if self.panel(panel_id).find_view.is_some() {
            return self.exit_find_view(panel_id);
        }
        let current = self.active_panel().cwd.clone();
        if matches!(self.active_backend_spec(), BackendSpec::Archive(_))
            && current == Path::new("/")
        {
            return self.detach_archive_panel(panel_id);
        }

        let Some(parent) = current.parent() else {
            return Ok(false);
        };

        let normalized = self
            .active_backend()
            .normalize_existing_path("parent", parent)?;
        let panel = self.active_panel_mut();
        panel.cwd = normalized;
        panel.find_view = None;
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        self.reload_panel(self.state.active_panel, true)
    }

    fn go_to_home(&mut self) -> Result<bool> {
        let normalized = match self.active_backend_spec() {
            BackendSpec::Local => {
                let home = env::var_os("HOME")
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("HOME environment variable is not set"))?;
                self.active_backend()
                    .normalize_existing_path("home", &home)?
            }
            BackendSpec::Sftp(info) => self
                .active_backend()
                .normalize_existing_path("home", &info.root_path)?,
            BackendSpec::Archive(_) => self
                .active_backend()
                .normalize_existing_path("home", Path::new("/"))?,
        };
        let panel = self.active_panel_mut();
        panel.cwd = normalized;
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        self.reload_panel(self.state.active_panel, true)
    }

    fn open_viewer(&mut self) -> Result<bool> {
        let entry = self.selected_action_target_entry()?;
        if entry.entry_type == FsEntryType::Directory {
            return Err(anyhow::anyhow!(
                "viewer is available for files only (selected '{}')",
                entry.name
            ));
        }

        let path = self
            .active_backend()
            .normalize_existing_path("viewer", &entry.path)?;
        let (bytes, truncated) = self
            .active_backend()
            .read_file_preview(path.as_path(), VIEWER_PREVIEW_LIMIT_BYTES)?;
        let viewer_state = load_viewer_state_from_preview(
            path.clone(),
            entry.name,
            entry.size_bytes,
            bytes,
            truncated,
        );

        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = None;
        self.pending_viewer_search = false;
        self.state.dialog = None;
        self.state.viewer = Some(viewer_state);
        self.state.screen_mode = ScreenMode::Viewer;
        self.state.status_line = format!("viewer opened: {}", path.display());
        Ok(true)
    }

    fn open_editor(&mut self) -> Result<bool> {
        if !matches!(self.active_backend_spec(), BackendSpec::Local) {
            self.show_alert("external editor is available only for local files");
            return Ok(true);
        }

        let entry = self.selected_action_target_entry()?;
        if entry.entry_type == FsEntryType::Directory {
            return Err(anyhow::anyhow!(
                "editor is available for files only (selected '{}')",
                entry.name
            ));
        }

        let path = self
            .active_backend()
            .normalize_existing_path("edit", &entry.path)?;
        if let Some(editor) = self.resolve_editor_command() {
            return self.run_editor_with_command(editor.as_str(), path.as_path());
        }

        self.start_editor_chooser(EditorChoiceContext::OpenFile(path))
    }

    fn run_editor_with_command(&mut self, editor: &str, path: &Path) -> Result<bool> {
        if editor.trim().is_empty() {
            self.show_alert("editor command is empty");
            return Ok(true);
        }

        runtime::set_input_poll_paused(true);
        if let Err(err) = terminal::suspend_for_external_process() {
            runtime::set_input_poll_paused(false);
            return Err(err);
        }

        let run_result = run_external_editor_command(editor, path);
        let resume_result = terminal::resume_after_external_process();
        runtime::set_input_poll_paused(false);

        resume_result?;
        self.force_full_redraw = true;
        let status = run_result?;
        if !status.success() {
            self.show_alert(format!("editor exited with status: {status}"));
            return Ok(true);
        }

        self.reload_panel(PanelId::Left, false)?;
        self.reload_panel(PanelId::Right, false)?;
        self.push_log(format!("editor closed ({editor}): {}", path.display()));
        Ok(true)
    }

    fn open_editor_settings(&mut self) -> Result<bool> {
        self.start_editor_chooser(EditorChoiceContext::SettingsOnly)
    }

    fn start_editor_chooser(&mut self, context: EditorChoiceContext) -> Result<bool> {
        let candidates = detect_editor_candidates();
        if candidates.is_empty() {
            self.show_alert(
                "No supported editors found in PATH (nvim/vim/nano/hx/micro/emacs/code)",
            );
            return Ok(true);
        }

        let preferred = self.resolve_editor_command();
        let default_choice = preferred
            .as_ref()
            .and_then(|value| candidates.iter().position(|item| item.command == *value))
            .map(|idx| idx + 1)
            .unwrap_or(1);

        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = Some(PendingEditorChoice {
            context,
            options: candidates.clone(),
        });
        self.pending_viewer_search = false;
        self.state.dialog = Some(input_dialog(
            "Editor Setup",
            build_editor_choice_body(candidates.as_slice()).as_str(),
            default_choice.to_string(),
            DialogTone::Warning,
        ));
        self.state.status_line = "editor setup: choose preferred editor number".to_string();
        Ok(true)
    }

    fn resolve_editor_command(&self) -> Option<String> {
        env::var("EDITOR")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(load_saved_editor_command)
    }

    fn close_viewer(&mut self) -> Result<bool> {
        if self.state.screen_mode != ScreenMode::Viewer {
            return Ok(false);
        }

        self.state.screen_mode = ScreenMode::Normal;
        self.state.viewer = None;
        self.pending_viewer_search = false;
        self.state.status_line = "viewer closed".to_string();
        Ok(true)
    }

    fn viewer_scroll_up(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        let previous = viewer.scroll_offset;
        viewer.scroll_offset = viewer.scroll_offset.saturating_sub(1);
        Ok(previous != viewer.scroll_offset)
    }

    fn viewer_scroll_down(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        let previous = viewer.scroll_offset;
        let max_offset = viewer.lines.len().saturating_sub(1);
        viewer.scroll_offset = (viewer.scroll_offset + 1).min(max_offset);
        Ok(previous != viewer.scroll_offset)
    }

    fn viewer_page_up(&mut self) -> Result<bool> {
        let step = self.viewer_page_step();
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        let previous = viewer.scroll_offset;
        viewer.scroll_offset = viewer.scroll_offset.saturating_sub(step);
        Ok(previous != viewer.scroll_offset)
    }

    fn viewer_page_down(&mut self) -> Result<bool> {
        let step = self.viewer_page_step();
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        let previous = viewer.scroll_offset;
        let max_offset = viewer.lines.len().saturating_sub(1);
        viewer.scroll_offset = (viewer.scroll_offset.saturating_add(step)).min(max_offset);
        Ok(previous != viewer.scroll_offset)
    }

    fn viewer_top(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        let previous = viewer.scroll_offset;
        viewer.scroll_offset = 0;
        Ok(previous != viewer.scroll_offset)
    }

    fn viewer_bottom(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        let previous = viewer.scroll_offset;
        viewer.scroll_offset = viewer.lines.len().saturating_sub(1);
        Ok(previous != viewer.scroll_offset)
    }

    fn viewer_toggle_mode(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        let next_mode = match viewer.mode {
            ViewerMode::Text => ViewerMode::Hex,
            ViewerMode::Hex => ViewerMode::Text,
        };
        set_viewer_mode(viewer, next_mode);
        self.state.status_line = format!("viewer mode: {}", viewer_mode_label(next_mode));
        Ok(true)
    }

    fn viewer_start_search_prompt(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_ref() else {
            return Ok(false);
        };
        self.pending_viewer_search = true;
        self.state.dialog = Some(input_dialog(
            "Viewer Search",
            "Find in current viewer mode",
            viewer.search_query.clone(),
            DialogTone::Default,
        ));
        self.state.status_line = "viewer search: enter pattern".to_string();
        Ok(true)
    }

    fn viewer_search_next(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        if jump_to_next_match(viewer, true).is_none() {
            self.state.status_line = "viewer search: no matches".to_string();
            return Ok(true);
        }
        self.state.status_line = viewer_match_status(viewer);
        Ok(true)
    }

    fn viewer_search_prev(&mut self) -> Result<bool> {
        let Some(viewer) = self.state.viewer.as_mut() else {
            return Ok(false);
        };
        if jump_to_next_match(viewer, false).is_none() {
            self.state.status_line = "viewer search: no matches".to_string();
            return Ok(true);
        }
        self.state.status_line = viewer_match_status(viewer);
        Ok(true)
    }

    fn viewer_page_step(&self) -> usize {
        self.state.terminal_size.height.saturating_sub(4).max(1) as usize
    }

    fn handle_sftp_action(&mut self) -> Result<bool> {
        let panel_id = self.state.active_panel;
        if matches!(self.backend_spec(panel_id), BackendSpec::Sftp(_)) {
            return self.disconnect_sftp_panel(panel_id);
        }
        self.start_sftp_connect()
    }

    fn toggle_sort(&mut self) -> Result<bool> {
        let next_sort = self.active_panel().sort_mode.next();
        self.active_panel_mut().sort_mode = next_sort;
        self.reload_panel(self.state.active_panel, true)
    }

    fn start_sftp_connect(&mut self) -> Result<bool> {
        let panel_id = self.state.active_panel;
        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = None;
        self.pending_viewer_search = false;
        let default_value = match self.backend_spec(panel_id) {
            BackendSpec::Sftp(info) => {
                format!("{}:{}{}", info.host, info.port, info.root_path.display())
            }
            BackendSpec::Local | BackendSpec::Archive(_) => "192.168.1.250:22/".to_string(),
        };
        self.pending_sftp_connect = Some(PendingSftpConnect {
            panel_id,
            stage: SftpConnectStage::Address,
            draft: SftpConnectDraft {
                host: String::new(),
                port: 22,
                root_path: PathBuf::from("/"),
                user: None,
            },
        });
        self.state.dialog = Some(input_dialog(
            "SFTP Connect",
            "Enter address: host[:port][/path] or 'local'",
            default_value,
            DialogTone::Default,
        ));
        self.state.status_line = "sftp connect: enter address".to_string();
        Ok(true)
    }

    fn disconnect_sftp_panel(&mut self, panel_id: PanelId) -> Result<bool> {
        if !matches!(self.backend_spec(panel_id), BackendSpec::Sftp(_)) {
            return Ok(false);
        }
        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = None;
        self.pending_viewer_search = false;
        self.state.dialog = None;

        let local_cwd = self.last_local_cwd(panel_id);
        let (redraw, used_path) = match self.attach_panel_to_local(panel_id, local_cwd.clone()) {
            Ok(redraw) => (redraw, local_cwd),
            Err(_) => {
                let fallback = env::current_dir()
                    .map_err(|err| anyhow::anyhow!("cannot resolve local cwd: {err}"))?;
                let redraw = self.attach_panel_to_local(panel_id, fallback.clone())?;
                (redraw, fallback)
            }
        };
        self.push_log(format!("sftp disconnected: {}", used_path.display()));
        Ok(redraw)
    }

    fn start_search(&mut self) -> Result<bool> {
        let panel_id = self.state.active_panel;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = None;
        self.pending_viewer_search = false;
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

    fn start_find_fd_prompt(&mut self) -> Result<bool> {
        let panel_id = self.state.active_panel;
        if !matches!(self.backend_spec(panel_id), BackendSpec::Local) {
            self.show_alert("Find via fd is available for local panel only");
            return Ok(true);
        }

        if !is_fd_available() {
            self.show_alert("fd is not installed. Install: brew install fd or apt install fd-find");
            return Ok(true);
        }

        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_viewer_search = false;
        let default_hidden = self.panel(panel_id).show_hidden;
        self.pending_find = Some(PendingFind {
            panel_id,
            root: self.panel(panel_id).cwd.clone(),
            default_hidden,
        });
        self.state.dialog = Some(input_dialog(
            "Find (fd)",
            "Pattern [--glob] [--hidden] [--follow]",
            String::new(),
            DialogTone::Default,
        ));
        self.state.status_line = "find: enter pattern and optional fd flags".to_string();
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
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = None;
        self.pending_viewer_search = false;
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
        if is_archive_backend(self.inactive_backend_spec()) {
            self.show_alert("copy into archive VFS is not supported yet");
            return Ok(true);
        }

        if let Some(plan) = self.build_batch_plan_from_selection(JobKind::Copy)? {
            self.state.dialog = Some(confirm_dialog(plan.summary.clone()));
            self.pending_confirmation = Some(PendingConfirmation::Batch(plan));
            return Ok(true);
        }

        let entry = self.selected_action_target_entry()?;
        self.open_rename_prompt(JobKind::Copy, &entry)
    }

    fn queue_move(&mut self) -> Result<bool> {
        if is_archive_backend(self.active_backend_spec()) {
            self.show_alert("move from archive VFS is not supported (read-only)");
            return Ok(true);
        }
        if is_archive_backend(self.inactive_backend_spec()) {
            self.show_alert("move into archive VFS is not supported yet");
            return Ok(true);
        }

        if let Some(plan) = self.build_batch_plan_from_selection(JobKind::Move)? {
            self.state.dialog = Some(confirm_dialog(plan.summary.clone()));
            self.pending_confirmation = Some(PendingConfirmation::Batch(plan));
            return Ok(true);
        }

        let entry = self.selected_action_target_entry()?;
        self.open_rename_prompt(JobKind::Move, &entry)
    }

    fn queue_delete(&mut self) -> Result<bool> {
        if is_archive_backend(self.active_backend_spec()) {
            self.show_alert("delete inside archive VFS is not supported (read-only)");
            return Ok(true);
        }

        if let Some(plan) = self.build_batch_plan_from_selection(JobKind::Delete)? {
            self.state.dialog = Some(confirm_dialog(plan.summary.clone()));
            self.pending_confirmation = Some(PendingConfirmation::Batch(plan));
            return Ok(true);
        }

        let entry = self.selected_action_target_entry()?;
        let path = self
            .active_backend()
            .normalize_existing_path("delete", &entry.path)?;
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

            let source = self
                .active_backend()
                .normalize_existing_path("batch", &entry.path)?;
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
                overwrite_destination: false,
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
        if is_archive_backend(self.active_backend_spec()) {
            self.show_alert("mkdir inside archive VFS is not supported (read-only)");
            return Ok(true);
        }
        let target = self.find_available_directory_name(self.active_panel().cwd.clone(), "new_dir");
        self.enqueue_job(JobKind::Mkdir, target, None, "mkdir queued")
    }

    fn open_rename_prompt(&mut self, kind: JobKind, entry: &FsEntry) -> Result<bool> {
        self.input_mode = None;
        self.pending_confirmation = None;
        self.pending_mask = None;
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = None;
        self.pending_viewer_search = false;
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
        let source_backend = self.active_backend_spec().clone();
        let destination_backend = match kind {
            JobKind::Copy | JobKind::Move => Some(self.inactive_backend_spec().clone()),
            JobKind::Delete | JobKind::Mkdir => None,
        };
        let request = JobRequest {
            id: self.next_job_id,
            batch_id,
            kind,
            source_backend,
            destination_backend,
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
            current_item: None,
            batch_completed: None,
            batch_total: None,
            message: Some(queued_message.to_string()),
        });
        if log_message {
            self.push_log(queued_message);
        }
        self.workers.submit(request)?;
        Ok(true)
    }

    fn execute_batch_plan(&mut self, plan: BatchPlan) -> Result<bool> {
        match plan.kind {
            JobKind::Copy | JobKind::Move => {
                self.start_conflict_resolution(plan.kind, Some(plan.batch_id), plan.items)
            }
            JobKind::Delete | JobKind::Mkdir => {
                self.enqueue_batch_jobs(plan.kind, Some(plan.batch_id), plan.items, 0)
            }
        }
    }

    fn start_conflict_resolution(
        &mut self,
        kind: JobKind,
        batch_id: Option<u64>,
        items: Vec<BatchOpItem>,
    ) -> Result<bool> {
        if items.is_empty() {
            return Ok(false);
        }

        self.pending_conflict = Some(PendingConflict {
            kind,
            batch_id,
            items,
            next_index: 0,
            ready: Vec::new(),
            skipped: 0,
            apply_all: None,
        });
        self.continue_conflict_resolution()
    }

    fn continue_conflict_resolution(&mut self) -> Result<bool> {
        loop {
            let Some(mut pending) = self.pending_conflict.take() else {
                return Ok(false);
            };

            if pending.next_index >= pending.items.len() {
                self.state.dialog = None;
                return self.finalize_conflict_resolution(pending);
            }

            let current = pending.items[pending.next_index].clone();
            let Some(destination) = current.destination.clone() else {
                pending.ready.push(current);
                pending.next_index += 1;
                self.pending_conflict = Some(pending);
                continue;
            };

            if !self.path_exists_on_backend(self.inactive_backend(), destination.as_path()) {
                pending.ready.push(current);
                pending.next_index += 1;
                self.pending_conflict = Some(pending);
                continue;
            }

            if let Some(policy) = pending.apply_all {
                self.pending_conflict = Some(pending);
                let action = match policy {
                    ConflictPolicy::Overwrite => ConflictAction::Overwrite,
                    ConflictPolicy::Skip => ConflictAction::Skip,
                    ConflictPolicy::OverwriteIfNewer => ConflictAction::OverwriteIfNewer,
                };
                self.apply_conflict_action(action)?;
                continue;
            }

            let title = format!(
                "Conflict {}/{}",
                pending.next_index + 1,
                pending.items.len()
            );
            let body = self.build_conflict_dialog_body(&current, destination.as_path());
            self.state.dialog = Some(conflict_dialog(title, body));
            self.pending_conflict = Some(pending);
            return Ok(true);
        }
    }

    fn apply_conflict_dialog_action(&mut self, button_idx: usize) -> bool {
        let Some(action) = self.dialog_conflict_action(button_idx) else {
            return false;
        };

        if action == ConflictAction::Cancel {
            self.pending_conflict = None;
            self.state.dialog = None;
            self.push_log("conflict resolution canceled");
            return true;
        }

        match self
            .apply_conflict_action(action)
            .and_then(|_| self.continue_conflict_resolution())
        {
            Ok(redraw) => redraw,
            Err(err) => {
                self.show_alert(err.to_string());
                true
            }
        }
    }

    fn apply_conflict_action(&mut self, action: ConflictAction) -> Result<()> {
        let Some(mut pending) = self.pending_conflict.take() else {
            return Ok(());
        };
        if pending.next_index >= pending.items.len() {
            self.pending_conflict = Some(pending);
            return Ok(());
        }

        if action == ConflictAction::OverwriteAll {
            pending.apply_all = Some(ConflictPolicy::Overwrite);
        } else if action == ConflictAction::SkipAll {
            pending.apply_all = Some(ConflictPolicy::Skip);
        } else if action == ConflictAction::OverwriteIfNewerAll {
            pending.apply_all = Some(ConflictPolicy::OverwriteIfNewer);
        }

        let mut item = pending.items[pending.next_index].clone();
        let destination = item
            .destination
            .clone()
            .ok_or_else(|| anyhow::anyhow!("copy/move item has no destination"))?;

        let effective = match action {
            ConflictAction::OverwriteAll => ConflictAction::Overwrite,
            ConflictAction::SkipAll => ConflictAction::Skip,
            ConflictAction::OverwriteIfNewerAll => ConflictAction::OverwriteIfNewer,
            _ => action,
        };

        match effective {
            ConflictAction::Overwrite => {
                item.overwrite_destination = true;
                pending.ready.push(item);
                pending.next_index += 1;
            }
            ConflictAction::Skip => {
                pending.skipped += 1;
                pending.next_index += 1;
            }
            ConflictAction::Rename => {
                let renamed = self.suggest_renamed_destination(destination.as_path())?;
                item.destination = Some(renamed.clone());
                pending.ready.push(item);
                pending.next_index += 1;
                self.push_log(format!(
                    "conflict rename auto: {} -> {}",
                    destination.display(),
                    renamed.display()
                ));
            }
            ConflictAction::OverwriteIfNewer => {
                if self.should_overwrite_if_newer(item.source.as_path(), destination.as_path())? {
                    item.overwrite_destination = true;
                    pending.ready.push(item);
                } else {
                    pending.skipped += 1;
                }
                pending.next_index += 1;
            }
            ConflictAction::Cancel
            | ConflictAction::OverwriteAll
            | ConflictAction::SkipAll
            | ConflictAction::OverwriteIfNewerAll => {}
        }

        self.state.dialog = None;
        self.pending_conflict = Some(pending);
        Ok(())
    }

    fn finalize_conflict_resolution(&mut self, pending: PendingConflict) -> Result<bool> {
        let destination_backend = backend_from_spec(self.inactive_backend_spec());
        let mut ready = Vec::with_capacity(pending.ready.len());
        let mut skipped = pending.skipped;

        for mut item in pending.ready {
            if item.overwrite_destination {
                if let Some(destination) = item.destination.as_ref() {
                    if destination_backend
                        .stat_entry(destination.as_path())
                        .is_ok()
                    {
                        match destination_backend.remove_path(destination.as_path()) {
                            Ok(()) => {}
                            Err(err) => {
                                skipped += 1;
                                item.overwrite_destination = false;
                                self.push_log(format!(
                                    "overwrite skipped: {} ({err})",
                                    destination.display()
                                ));
                                continue;
                            }
                        }
                    }
                }
            }
            ready.push(item);
        }

        self.enqueue_batch_jobs(pending.kind, pending.batch_id, ready, skipped)
    }

    fn enqueue_batch_jobs(
        &mut self,
        kind: JobKind,
        batch_id: Option<u64>,
        items: Vec<BatchOpItem>,
        skipped: usize,
    ) -> Result<bool> {
        if items.is_empty() {
            if skipped > 0 {
                self.push_log(format!(
                    "batch {}: all {} item(s) skipped",
                    operation_name(kind),
                    skipped
                ));
                return Ok(true);
            }
            return Ok(false);
        }

        if let Some(batch_id) = batch_id {
            let total = items.len();
            let first_file = items
                .first()
                .map(|item| item.name.clone())
                .unwrap_or_else(|| "-".to_string());
            self.batch_progress.insert(
                batch_id,
                BatchProgress {
                    kind,
                    total,
                    completed: 0,
                    failed: 0,
                    current_file: first_file,
                },
            );
            self.sync_visible_batch_progress(Some(batch_id));

            for item in items {
                if let Err(err) = self.enqueue_job_with_options(
                    kind,
                    item.source,
                    item.destination,
                    Some(batch_id),
                    format!("{} queued: {}", operation_name(kind), item.name),
                    false,
                ) {
                    self.batch_progress.remove(&batch_id);
                    self.sync_visible_batch_progress(None);
                    return Err(err);
                }
            }

            if skipped > 0 {
                self.push_log(format!(
                    "batch {} queued: {} item(s), skipped {}",
                    operation_name(kind),
                    total,
                    skipped
                ));
            } else {
                self.push_log(format!(
                    "batch {} queued: {} item(s)",
                    operation_name(kind),
                    total
                ));
            }
            Ok(true)
        } else {
            let mut queued = 0usize;
            for item in items {
                let queue_message = copy_move_item_message(kind, &item);
                self.enqueue_job_with_options(
                    kind,
                    item.source,
                    item.destination,
                    None,
                    queue_message,
                    true,
                )?;
                queued += 1;
            }
            if skipped > 0 {
                self.push_log(format!(
                    "{} queued: {} item(s), skipped {}",
                    operation_name(kind),
                    queued,
                    skipped
                ));
            }
            Ok(true)
        }
    }

    fn build_conflict_dialog_body(&self, item: &BatchOpItem, destination: &Path) -> String {
        let source_meta = self.active_backend().stat_entry(item.source.as_path()).ok();
        let target_meta = self.inactive_backend().stat_entry(destination).ok();
        let source_size = source_meta
            .as_ref()
            .map(|entry| entry.size_bytes)
            .unwrap_or(0);
        let target_size = target_meta
            .as_ref()
            .map(|entry| entry.size_bytes)
            .unwrap_or(0);
        let source_mtime = source_meta.as_ref().and_then(|entry| entry.modified_at);
        let target_mtime = target_meta.as_ref().and_then(|entry| entry.modified_at);
        let newer_hint = match compare_mtime(source_mtime, target_mtime) {
            Some(std::cmp::Ordering::Greater) => "source newer",
            Some(std::cmp::Ordering::Less) => "target newer",
            Some(std::cmp::Ordering::Equal) => "same mtime",
            None => "mtime unavailable",
        };

        format!(
            "{}\nDestination exists:\n{}\n\nsrc: size={} mtime={}\ndst: size={} mtime={}\nhint: {}\n\n[O]verwrite [S]kip [R]ename [N]ewer\n[W]OverwriteAll [K]SkipAll [A]NewerAll [C]ancel",
            item.name,
            destination.display(),
            format_bytes(source_size),
            format_mtime_hint(source_mtime),
            format_bytes(target_size),
            format_mtime_hint(target_mtime),
            newer_hint,
        )
    }

    fn dialog_conflict_action(&self, button_idx: usize) -> Option<ConflictAction> {
        let label = self
            .state
            .dialog
            .as_ref()
            .and_then(|dialog| dialog.buttons.get(button_idx))
            .map(|button| button.label.as_str())?;

        Some(match label {
            "Overwrite" => ConflictAction::Overwrite,
            "Skip" => ConflictAction::Skip,
            "Rename" => ConflictAction::Rename,
            "Newer" => ConflictAction::OverwriteIfNewer,
            "OverAll" => ConflictAction::OverwriteAll,
            "SkipAll" => ConflictAction::SkipAll,
            "NewerAll" => ConflictAction::OverwriteIfNewerAll,
            "Cancel" => ConflictAction::Cancel,
            _ => return None,
        })
    }

    fn suggest_renamed_destination(&self, destination: &Path) -> Result<PathBuf> {
        let parent = destination.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "destination has no parent for rename fallback: {}",
                destination.display()
            )
        })?;
        let file_name = destination
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .ok_or_else(|| anyhow::anyhow!("destination has no file name"))?;
        let (stem, ext) = split_name_and_extension(file_name.as_str());
        let backend = backend_from_spec(self.inactive_backend_spec());

        for idx in 1..=10_000usize {
            let candidate_name = if ext.is_empty() {
                format!("{stem}_copy{idx}")
            } else {
                format!("{stem}_copy{idx}.{ext}")
            };
            let candidate = parent.join(candidate_name);
            if backend.stat_entry(candidate.as_path()).is_err() {
                return Ok(candidate);
            }
        }

        Err(anyhow::anyhow!(
            "cannot allocate unique conflict rename near {}",
            destination.display()
        ))
    }

    fn should_overwrite_if_newer(&self, source: &Path, destination: &Path) -> Result<bool> {
        let source_entry = self.active_backend().stat_entry(source)?;
        let destination_entry = self.inactive_backend().stat_entry(destination)?;
        if let Some(ordering) =
            compare_mtime(source_entry.modified_at, destination_entry.modified_at)
        {
            return Ok(ordering == std::cmp::Ordering::Greater);
        }
        Ok(source_entry.size_bytes > destination_entry.size_bytes)
    }

    fn sync_visible_batch_progress(&mut self, preferred_batch_id: Option<u64>) {
        let preferred = preferred_batch_id.and_then(|batch_id| {
            self.batch_progress.get(&batch_id).map(|progress| {
                (
                    batch_id,
                    progress.kind,
                    progress.current_file.clone(),
                    progress.completed,
                    progress.total,
                    progress.failed,
                )
            })
        });

        let selected = preferred.or_else(|| {
            self.batch_progress
                .iter()
                .max_by_key(|(batch_id, _)| *batch_id)
                .map(|(batch_id, progress)| {
                    (
                        *batch_id,
                        progress.kind,
                        progress.current_file.clone(),
                        progress.completed,
                        progress.total,
                        progress.failed,
                    )
                })
        });

        self.state.batch_progress = selected.map(
            |(batch_id, operation, current_file, completed, total, failed)| BatchProgressState {
                batch_id,
                operation,
                current_file,
                completed,
                total,
                failed,
            },
        );
    }

    fn apply_find_results(
        &mut self,
        panel_id: PanelId,
        root: PathBuf,
        query: String,
        glob: bool,
        hidden: bool,
        follow_symlinks: bool,
        mut entries: Vec<FsEntry>,
    ) -> Result<bool> {
        let sort_mode = self.panel(panel_id).sort_mode;
        sort_find_entries(entries.as_mut_slice(), sort_mode);
        let mut panel_entries = Vec::with_capacity(entries.len().saturating_add(1));
        panel_entries.push(parent_link_entry(root.clone()));
        panel_entries.extend(entries);
        let matches = panel_entries.len().saturating_sub(1);

        {
            let panel = self.panel_mut(panel_id);
            panel.cwd = root.clone();
            panel.find_view = Some(FindPanelState {
                root: root.clone(),
                query: query.clone(),
                glob,
                hidden,
                follow_symlinks,
            });
            panel.search_query.clear();
            panel.selected_paths.clear();
            panel.selected_index = 0;
            panel.clear_selection_anchor();
            panel.set_entries(panel_entries);
            panel.error_message = None;
        }
        self.state.active_panel = panel_id;
        self.state.status_line = format!("find done: '{}' => {matches} match(es)", query);
        self.push_log(format!(
            "find done [{}]: '{}' in {}",
            panel_name(panel_id),
            query,
            root.display()
        ));
        Ok(true)
    }

    fn exit_find_view(&mut self, panel_id: PanelId) -> Result<bool> {
        let Some(find_view) = self.panel(panel_id).find_view.clone() else {
            return Ok(false);
        };
        self.set_active_find_id(panel_id, None);
        if self
            .state
            .find_progress
            .as_ref()
            .is_some_and(|progress| progress.panel_id == panel_id)
        {
            self.state.find_progress = None;
        }
        let panel = self.panel_mut(panel_id);
        panel.find_view = None;
        panel.cwd = find_view.root.clone();
        panel.selected_paths.clear();
        panel.clear_search();
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        self.reload_panel(panel_id, true)
    }

    fn open_find_result_entry(&mut self, panel_id: PanelId, entry: FsEntry) -> Result<bool> {
        if entry.is_virtual {
            return Ok(false);
        }

        let target_dir = if entry.entry_type == FsEntryType::Directory {
            entry.path.clone()
        } else {
            entry
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.panel(panel_id).cwd.clone())
        };
        let normalized = self
            .backend(panel_id)
            .normalize_existing_path("find_jump", target_dir.as_path())?;
        {
            let panel = self.panel_mut(panel_id);
            panel.find_view = None;
            panel.cwd = normalized;
            panel.clear_search();
            panel.selected_paths.clear();
            panel.selected_index = 0;
            panel.clear_selection_anchor();
        }
        let redraw = self.reload_panel(panel_id, true)?;
        if entry.entry_type != FsEntryType::Directory {
            if let Some(position) = self
                .panel(panel_id)
                .entries
                .iter()
                .position(|candidate| candidate.path == entry.path)
            {
                self.panel_mut(panel_id).selected_index = position;
            }
        }
        Ok(redraw)
    }

    fn attach_panel_to_local(&mut self, panel_id: PanelId, cwd: PathBuf) -> Result<bool> {
        let backend_spec = BackendSpec::Local;
        let backend = backend_from_spec(&backend_spec);
        let normalized = backend.normalize_existing_path("connect_local", &cwd)?;
        self.set_panel_backend(panel_id, backend_spec);
        self.set_last_local_cwd(panel_id, normalized.clone());
        self.set_active_find_id(panel_id, None);
        if self
            .state
            .find_progress
            .as_ref()
            .is_some_and(|progress| progress.panel_id == panel_id)
        {
            self.state.find_progress = None;
        }
        let panel = self.panel_mut(panel_id);
        panel.cwd = normalized;
        panel.find_view = None;
        panel.search_query.clear();
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        self.reload_panel(panel_id, true)
    }

    fn attach_panel_to_sftp(
        &mut self,
        panel_id: PanelId,
        conn: SftpConnectionInfo,
    ) -> Result<bool> {
        if matches!(self.backend_spec(panel_id), BackendSpec::Local) {
            self.set_last_local_cwd(panel_id, self.panel(panel_id).cwd.clone());
        }
        let backend_spec = BackendSpec::Sftp(conn.clone());
        let backend = backend_from_spec(&backend_spec);
        let normalized = backend.normalize_existing_path("connect_sftp", &conn.root_path)?;
        self.set_panel_backend(panel_id, backend_spec);
        self.set_active_find_id(panel_id, None);
        if self
            .state
            .find_progress
            .as_ref()
            .is_some_and(|progress| progress.panel_id == panel_id)
        {
            self.state.find_progress = None;
        }
        let panel = self.panel_mut(panel_id);
        panel.cwd = normalized;
        panel.find_view = None;
        panel.search_query.clear();
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        self.reload_panel(panel_id, true)
    }

    fn attach_panel_to_archive(
        &mut self,
        panel_id: PanelId,
        archive_path: PathBuf,
    ) -> Result<bool> {
        if !matches!(self.backend_spec(panel_id), BackendSpec::Local) {
            return Err(anyhow::anyhow!(
                "archive VFS can be opened from local panel only in current build"
            ));
        }
        let normalized_archive = self
            .backend(panel_id)
            .normalize_existing_path("archive_open", archive_path.as_path())?;
        if !is_archive_file_path(normalized_archive.as_path()) {
            return Err(anyhow::anyhow!(
                "not a supported archive: {}",
                normalized_archive.display()
            ));
        }

        self.set_last_local_cwd(panel_id, self.panel(panel_id).cwd.clone());
        let backend_spec = BackendSpec::Archive(ArchiveConnectionInfo {
            archive_path: normalized_archive.clone(),
        });
        self.set_panel_backend(panel_id, backend_spec);
        self.set_active_find_id(panel_id, None);
        if self
            .state
            .find_progress
            .as_ref()
            .is_some_and(|progress| progress.panel_id == panel_id)
        {
            self.state.find_progress = None;
        }
        let panel = self.panel_mut(panel_id);
        panel.cwd = PathBuf::from("/");
        panel.find_view = None;
        panel.search_query.clear();
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        let redraw = self.reload_panel(panel_id, true)?;
        self.push_log(format!("archive opened: {}", normalized_archive.display()));
        Ok(redraw)
    }

    fn detach_archive_panel(&mut self, panel_id: PanelId) -> Result<bool> {
        let BackendSpec::Archive(info) = self.backend_spec(panel_id).clone() else {
            return Ok(false);
        };
        let fallback = info
            .archive_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.last_local_cwd(panel_id));
        let redraw = self.attach_panel_to_local(panel_id, fallback)?;
        self.push_log(format!("archive closed: {}", info.archive_path.display()));
        Ok(redraw)
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
        while self.path_exists_on_backend(self.active_backend(), candidate.as_path()) {
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

        if matches!(self.active_backend_spec(), BackendSpec::Local) {
            if let Some(home) = env::var_os("HOME") {
                let home_path = self
                    .active_backend()
                    .normalize_existing_path("delete", &PathBuf::from(home))?;
                if *target == home_path {
                    return Err(anyhow::anyhow!(
                        "refusing to delete HOME directory: {}",
                        home_path.display()
                    ));
                }
            }
        }

        Ok(())
    }

    fn handle_dialog_input(&mut self, key: &KeyEvent) -> Option<bool> {
        self.state.dialog.as_ref()?;

        if let Some(accel) = accelerator_from_key(key) {
            if let Some(button_idx) = self.find_dialog_button_by_accelerator(accel) {
                return Some(self.activate_dialog_button(button_idx));
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
                let button_idx = self
                    .state
                    .dialog
                    .as_ref()
                    .map(|dialog| dialog.focused_button)
                    .unwrap_or(0);
                Some(self.activate_dialog_button(button_idx))
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

    fn activate_dialog_button(&mut self, button_idx: usize) -> bool {
        if self.pending_conflict.is_some() {
            return self.apply_conflict_dialog_action(button_idx);
        }

        let role = self.dialog_button_role(button_idx);
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

        if self.pending_sftp_connect.is_some() {
            return if role == DialogButtonRole::Primary {
                self.apply_sftp_connect()
            } else {
                self.pending_sftp_connect = None;
                self.state.dialog = None;
                self.push_log("sftp connect canceled");
                true
            };
        }

        if self.pending_find.is_some() {
            return if role == DialogButtonRole::Primary {
                self.apply_find()
            } else {
                self.pending_find = None;
                self.pending_editor_choice = None;
                self.state.dialog = None;
                self.push_log("find canceled");
                true
            };
        }

        if self.pending_editor_choice.is_some() {
            return if role == DialogButtonRole::Primary {
                self.apply_editor_choice()
            } else {
                self.pending_editor_choice = None;
                self.state.dialog = None;
                self.push_log("editor setup canceled");
                true
            };
        }

        if self.pending_viewer_search {
            return if role == DialogButtonRole::Primary {
                self.apply_viewer_search()
            } else {
                self.pending_viewer_search = false;
                self.state.dialog = None;
                self.push_log("viewer search canceled");
                true
            };
        }

        self.state.dialog = None;
        true
    }

    fn cancel_dialog(&mut self) -> bool {
        if self.pending_conflict.is_some() {
            self.pending_conflict = None;
            self.state.dialog = None;
            self.push_log("conflict resolution canceled");
            return true;
        }

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

        if self.pending_sftp_connect.is_some() {
            self.pending_sftp_connect = None;
            self.state.dialog = None;
            self.push_log("sftp connect canceled");
            return true;
        }

        if self.pending_find.is_some() {
            self.pending_find = None;
            self.pending_editor_choice = None;
            self.state.dialog = None;
            self.push_log("find canceled");
            return true;
        }

        if self.pending_editor_choice.is_some() {
            self.pending_editor_choice = None;
            self.state.dialog = None;
            self.push_log("editor setup canceled");
            return true;
        }

        if self.pending_viewer_search {
            self.pending_viewer_search = false;
            self.state.dialog = None;
            self.push_log("viewer search canceled");
            return true;
        }

        self.state.dialog = None;
        true
    }

    fn dialog_button_role(&self, button_idx: usize) -> DialogButtonRole {
        self.state
            .dialog
            .as_ref()
            .and_then(|dialog| dialog.buttons.get(button_idx))
            .map(|button| button.role)
            .unwrap_or(DialogButtonRole::Secondary)
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
        let item = BatchOpItem {
            source: pending.source_path,
            destination: Some(destination),
            name: pending.source_name,
            overwrite_destination: false,
        };

        match self.start_conflict_resolution(pending.kind, None, vec![item]) {
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

    fn apply_sftp_connect(&mut self) -> bool {
        let pending = self.pending_sftp_connect.take();
        let value = self
            .state
            .dialog
            .as_ref()
            .and_then(|dialog| dialog.input_value.as_ref())
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        self.state.dialog = None;

        let Some(mut pending) = pending else {
            return true;
        };
        if value.is_empty() {
            self.show_alert("sftp connect value cannot be empty");
            return true;
        }

        match pending.stage {
            SftpConnectStage::Address => {
                if value.eq_ignore_ascii_case("local") {
                    let result = env::current_dir()
                        .map_err(|err| anyhow::anyhow!("cannot resolve local cwd: {err}"))
                        .and_then(|cwd| self.attach_panel_to_local(pending.panel_id, cwd));
                    return match result {
                        Ok(redraw) => redraw,
                        Err(err) => {
                            self.show_alert(format!("sftp connect failed: {err}"));
                            true
                        }
                    };
                }

                match parse_sftp_address_input(value.as_str()) {
                    Ok((user_from_address, host, port, root_path)) => {
                        pending.draft.host = host;
                        pending.draft.port = port;
                        pending.draft.root_path = root_path;
                        pending.draft.user = user_from_address;
                    }
                    Err(err) => {
                        self.show_alert(format!("sftp connect failed: {err}"));
                        return true;
                    }
                }

                if pending.draft.user.is_none() {
                    return self.prompt_sftp_login(pending);
                }
                self.proceed_sftp_auth_flow(pending)
            }
            SftpConnectStage::Login => {
                pending.draft.user = Some(value.trim().to_string());
                self.proceed_sftp_auth_flow(pending)
            }
            SftpConnectStage::Password => {
                let Some(user) = pending.draft.user.clone() else {
                    self.show_alert("sftp connect failed: login is missing");
                    return true;
                };
                let conn = SftpConnectionInfo {
                    host: pending.draft.host.clone(),
                    user,
                    port: pending.draft.port,
                    root_path: pending.draft.root_path.clone(),
                    auth: SftpAuth::Password(value),
                };
                match self.attach_panel_to_sftp(pending.panel_id, conn) {
                    Ok(redraw) => redraw,
                    Err(err) => {
                        self.show_alert(format!("sftp connect failed: {err}"));
                        true
                    }
                }
            }
            SftpConnectStage::KeyPath => {
                let Some(user) = pending.draft.user.clone() else {
                    self.show_alert("sftp connect failed: login is missing");
                    return true;
                };
                let conn = SftpConnectionInfo {
                    host: pending.draft.host.clone(),
                    user,
                    port: pending.draft.port,
                    root_path: pending.draft.root_path.clone(),
                    auth: SftpAuth::KeyFile {
                        path: expand_tilde_path(value.as_str()),
                        passphrase: None,
                    },
                };
                match self.attach_panel_to_sftp(pending.panel_id, conn) {
                    Ok(redraw) => redraw,
                    Err(err) => {
                        self.show_alert(format!("sftp connect failed: {err}"));
                        true
                    }
                }
            }
        }
    }

    fn apply_find(&mut self) -> bool {
        let pending = self.pending_find.take();
        let value = self
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
        if value.is_empty() {
            self.show_alert("find query cannot be empty");
            return true;
        }

        let parsed = match parse_find_input(value.as_str(), pending.default_hidden) {
            Ok(parsed) => parsed,
            Err(err) => {
                self.show_alert(format!("find parse error: {err}"));
                return true;
            }
        };

        if !is_fd_available() {
            self.show_alert("fd is not installed. Install: brew install fd or apt install fd-find");
            return true;
        }

        let request = FindRequest {
            id: self.next_find_id,
            panel_id: pending.panel_id,
            root: pending.root.clone(),
            query: parsed.query.clone(),
            glob: parsed.glob,
            hidden: parsed.hidden,
            follow_symlinks: parsed.follow_symlinks,
        };
        self.next_find_id = self.next_find_id.saturating_add(1);
        self.set_active_find_id(pending.panel_id, Some(request.id));
        self.state.find_progress = Some(FindProgressState {
            panel_id: pending.panel_id,
            query: request.query.clone(),
            matches: 0,
            running: true,
        });
        self.state.status_line = format!(
            "find running: '{}'{}{}",
            request.query,
            if request.glob { " [glob]" } else { "" },
            if request.follow_symlinks {
                " [follow]"
            } else {
                ""
            }
        );
        spawn_fd_search(request, self.event_tx.clone());
        true
    }

    fn apply_editor_choice(&mut self) -> bool {
        let pending = self.pending_editor_choice.take();
        let value = self
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
        let choice = if value.is_empty() {
            1
        } else {
            value.parse::<usize>().unwrap_or(0)
        };
        if choice == 0 || choice > pending.options.len() {
            self.show_alert(format!(
                "invalid editor choice '{}', expected 1..{}",
                value,
                pending.options.len()
            ));
            return true;
        }

        let selected = pending.options[choice - 1].clone();
        if let Err(err) = save_editor_command(selected.command.as_str()) {
            self.show_alert(format!("cannot save editor config: {err}"));
            return true;
        }

        match pending.context {
            EditorChoiceContext::OpenFile(path) => {
                match self.run_editor_with_command(selected.command.as_str(), path.as_path()) {
                    Ok(redraw) => redraw,
                    Err(err) => {
                        self.show_alert(err.to_string());
                        true
                    }
                }
            }
            EditorChoiceContext::SettingsOnly => {
                self.push_log(format!("editor saved: {}", selected.command));
                if env::var("EDITOR")
                    .ok()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
                {
                    self.push_log("note: $EDITOR overrides saved editor for current session");
                }
                true
            }
        }
    }

    fn apply_viewer_search(&mut self) -> bool {
        let query = self
            .state
            .dialog
            .as_ref()
            .and_then(|dialog| dialog.input_value.as_ref())
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        self.pending_viewer_search = false;
        self.state.dialog = None;

        let Some(viewer) = self.state.viewer.as_mut() else {
            return true;
        };
        viewer.search_query = query;
        refresh_viewer_search(viewer);
        if viewer.search_matches.is_empty() {
            self.state.status_line = "viewer search: no matches".to_string();
        } else {
            self.state.status_line = viewer_match_status(viewer);
        }
        true
    }

    fn proceed_sftp_auth_flow(&mut self, pending: PendingSftpConnect) -> bool {
        let Some(user) = pending.draft.user.clone() else {
            return self.prompt_sftp_login(pending);
        };
        if user.trim().is_empty() {
            return self.prompt_sftp_login(pending);
        }

        let hint = match probe_sftp_auth_hint(
            pending.draft.host.as_str(),
            pending.draft.port,
            user.as_str(),
        ) {
            Ok(hint) => hint,
            Err(_) => SftpAuthHint::Either,
        };

        if hint == SftpAuthHint::KeyOnly {
            return self.prompt_sftp_key_path(pending);
        }

        if hint == SftpAuthHint::PasswordOnly {
            return self.prompt_sftp_password(pending);
        }

        self.prompt_sftp_password(pending)
    }

    fn prompt_sftp_login(&mut self, mut pending: PendingSftpConnect) -> bool {
        pending.stage = SftpConnectStage::Login;
        let default_login = pending
            .draft
            .user
            .clone()
            .unwrap_or_else(|| env::var("USER").unwrap_or_default());
        self.pending_sftp_connect = Some(pending);
        self.state.dialog = Some(input_dialog(
            "SFTP Login",
            "Enter login name",
            default_login,
            DialogTone::Default,
        ));
        self.state.status_line = "sftp connect: enter login".to_string();
        true
    }

    fn prompt_sftp_password(&mut self, mut pending: PendingSftpConnect) -> bool {
        pending.stage = SftpConnectStage::Password;
        self.pending_sftp_connect = Some(pending);
        self.state.dialog = Some(input_dialog_with_mask(
            "SFTP Password",
            "Enter password",
            String::new(),
            DialogTone::Warning,
            true,
        ));
        self.state.status_line = "sftp connect: password required".to_string();
        true
    }

    fn prompt_sftp_key_path(&mut self, mut pending: PendingSftpConnect) -> bool {
        pending.stage = SftpConnectStage::KeyPath;
        self.pending_sftp_connect = Some(pending);
        self.state.dialog = Some(input_dialog(
            "SFTP Key",
            "Enter private key path",
            "~/.ssh/id_rsa".to_string(),
            DialogTone::Warning,
        ));
        self.state.status_line = "sftp connect: key path required".to_string();
        true
    }

    fn edit_dialog_input_backspace(&mut self) -> bool {
        if self.pending_rename.is_none()
            && self.pending_mask.is_none()
            && self.pending_sftp_connect.is_none()
            && self.pending_find.is_none()
            && self.pending_editor_choice.is_none()
            && !self.pending_viewer_search
        {
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
        if self.pending_rename.is_none()
            && self.pending_mask.is_none()
            && self.pending_sftp_connect.is_none()
            && self.pending_find.is_none()
            && self.pending_editor_choice.is_none()
            && !self.pending_viewer_search
        {
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

    fn find_dialog_button_by_accelerator(&self, accelerator: char) -> Option<usize> {
        let normalized = accelerator.to_ascii_lowercase();
        self.state.dialog.as_ref().and_then(|dialog| {
            dialog.buttons.iter().position(|button| {
                button.accelerator.map(|c| c.to_ascii_lowercase()) == Some(normalized)
            })
        })
    }

    fn handle_top_menu_input(&mut self, key: &KeyEvent) -> Option<bool> {
        if self.state.dialog.is_some() || self.state.screen_mode == ScreenMode::Viewer {
            return None;
        }

        if self.state.top_menu.open {
            if let Some(group_index) = top_menu_group_from_key(key) {
                self.state.top_menu.group_index = group_index;
                self.state.top_menu.item_index =
                    first_selectable_menu_item(top_menu_groups()[group_index].items).unwrap_or(0);
                self.state.status_line = format!(
                    "menu: {}",
                    top_menu_groups()[self.state.top_menu.group_index].label
                );
                return Some(true);
            }

            return match key.code {
                KeyCode::Esc | KeyCode::F(9) => Some(self.close_top_menu(true)),
                KeyCode::Left => Some(self.move_top_menu_group(false)),
                KeyCode::Right => Some(self.move_top_menu_group(true)),
                KeyCode::Up => Some(self.move_top_menu_item(false)),
                KeyCode::Down => Some(self.move_top_menu_item(true)),
                KeyCode::Enter => Some(self.activate_top_menu_item()),
                _ => Some(false),
            };
        }

        if self.input_mode.is_some() || self.state.command_line.active {
            return None;
        }

        if key.code == KeyCode::F(9) && key.modifiers.is_empty() {
            return Some(self.open_top_menu().unwrap_or_else(|err| {
                self.show_alert(err.to_string());
                true
            }));
        }

        if let Some(group_index) = top_menu_group_from_key(key) {
            if top_menu_groups().is_empty() {
                return Some(false);
            }
            self.state.top_menu.open = true;
            self.state.top_menu.group_index = group_index;
            self.state.top_menu.item_index =
                first_selectable_menu_item(top_menu_groups()[group_index].items).unwrap_or(0);
            self.state.status_line = format!(
                "menu: {}",
                top_menu_groups()[self.state.top_menu.group_index].label
            );
            return Some(true);
        }

        None
    }

    fn open_top_menu(&mut self) -> Result<bool> {
        if self.state.dialog.is_some() || self.state.screen_mode == ScreenMode::Viewer {
            return Ok(false);
        }
        if self.input_mode.is_some() || self.state.command_line.active {
            return Ok(false);
        }
        if top_menu_groups().is_empty() {
            return Ok(false);
        }

        self.state.top_menu.open = true;
        self.state.top_menu.group_index = self
            .state
            .top_menu
            .group_index
            .min(top_menu_groups().len().saturating_sub(1));
        let items = top_menu_groups()[self.state.top_menu.group_index].items;
        let preferred = self
            .state
            .top_menu
            .item_index
            .min(items.len().saturating_sub(1));
        self.state.top_menu.item_index = preferred;
        if !items
            .get(preferred)
            .is_some_and(|item| item.is_selectable())
        {
            self.state.top_menu.item_index = first_selectable_menu_item(items).unwrap_or(0);
        }
        self.state.status_line = "menu: arrows navigate, Enter run, Esc close".to_string();
        Ok(true)
    }

    fn close_top_menu(&mut self, update_status: bool) -> bool {
        if !self.state.top_menu.open {
            return false;
        }
        self.state.top_menu.open = false;
        if update_status {
            self.state.status_line = "menu closed".to_string();
        }
        true
    }

    fn move_top_menu_group(&mut self, forward: bool) -> bool {
        let groups = top_menu_groups();
        if groups.is_empty() {
            return false;
        }

        let len = groups.len();
        let current = self.state.top_menu.group_index.min(len - 1);
        self.state.top_menu.group_index = if forward {
            (current + 1) % len
        } else if current == 0 {
            len - 1
        } else {
            current - 1
        };
        self.state.top_menu.item_index =
            first_selectable_menu_item(groups[self.state.top_menu.group_index].items).unwrap_or(0);
        self.state.status_line = format!("menu: {}", groups[self.state.top_menu.group_index].label);
        true
    }

    fn move_top_menu_item(&mut self, forward: bool) -> bool {
        let groups = top_menu_groups();
        if groups.is_empty() {
            return false;
        }

        let group_idx = self.state.top_menu.group_index.min(groups.len() - 1);
        let items = groups[group_idx].items;
        if items.is_empty() {
            self.state.top_menu.item_index = 0;
            return false;
        }
        let len = items.len();
        let current = self.state.top_menu.item_index.min(len - 1);
        if let Some(next) = next_selectable_menu_item(items, current, forward) {
            self.state.top_menu.item_index = next;
            true
        } else {
            false
        }
    }

    fn activate_top_menu_item(&mut self) -> bool {
        let groups = top_menu_groups();
        if groups.is_empty() {
            return self.close_top_menu(false);
        }

        let group_idx = self.state.top_menu.group_index.min(groups.len() - 1);
        let group = groups[group_idx];
        if group.items.is_empty() {
            return self.close_top_menu(false);
        }
        let item_idx = self.state.top_menu.item_index.min(group.items.len() - 1);
        let item = group.items[item_idx];
        let Some(action) = item.action else {
            return false;
        };

        self.state.top_menu.open = false;
        self.state.status_line = format!("menu: {} -> {}", group.label, item.label);
        match self.execute_top_menu_action(action) {
            Ok(redraw) => redraw,
            Err(err) => {
                self.show_alert(err.to_string());
                true
            }
        }
    }

    fn execute_top_menu_action(&mut self, action: MenuAction) -> Result<bool> {
        match action {
            MenuAction::ActivatePanel(panel_id) => {
                self.state.active_panel = panel_id;
                self.push_log(format!("active panel: {}", panel_name(panel_id)));
                Ok(true)
            }
            MenuAction::PanelHome(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::go_to_home)
            }
            MenuAction::PanelParent(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::go_to_parent)
            }
            MenuAction::PanelCopy(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::queue_copy)
            }
            MenuAction::PanelMove(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::queue_move)
            }
            MenuAction::PanelDelete(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::queue_delete)
            }
            MenuAction::PanelMkdir(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::queue_mkdir)
            }
            MenuAction::PanelConnectSftp(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::handle_sftp_action)
            }
            MenuAction::PanelOpenShell(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::open_shell_mode)
            }
            MenuAction::PanelOpenCommandLine(panel_id) => {
                self.state.active_panel = panel_id;
                self.state.command_line.active = true;
                self.state.command_line.input.clear();
                self.state.status_line = "command line active".to_string();
                Ok(true)
            }
            MenuAction::PanelFindFd(panel_id) => {
                self.run_with_panel_focus(panel_id, Self::start_find_fd_prompt)
            }
            MenuAction::PanelOpenArchiveVfs(panel_id) => {
                self.state.active_panel = panel_id;
                self.open_selected_archive_from_panel(panel_id)
            }
            MenuAction::ToggleSort => self.toggle_sort(),
            MenuAction::Refresh => self.refresh_all(),
            MenuAction::ViewerModesInfo => {
                self.show_alert(
                    "Viewer controls: F2 toggle text/hex, / search, n/N next/prev match",
                );
                Ok(true)
            }
            MenuAction::EditorSettings => self.open_editor_settings(),
        }
    }

    fn run_with_panel_focus(
        &mut self,
        panel_id: PanelId,
        action: fn(&mut Self) -> Result<bool>,
    ) -> Result<bool> {
        self.state.active_panel = panel_id;
        action(self)
    }

    fn open_selected_archive_from_panel(&mut self, panel_id: PanelId) -> Result<bool> {
        if !matches!(self.backend_spec(panel_id), BackendSpec::Local) {
            self.show_alert("Archive VFS can be opened from local panel only");
            return Ok(true);
        }
        let entry = self
            .panel(panel_id)
            .selected_entry()
            .ok_or_else(|| anyhow::anyhow!("no selected entry"))?
            .clone();
        if entry.entry_type == FsEntryType::Directory {
            return Err(anyhow::anyhow!("select archive file, not directory"));
        }
        if !is_archive_file_path(entry.path.as_path()) {
            return Err(anyhow::anyhow!(
                "selected file is not supported archive: {}",
                entry.name
            ));
        }
        self.attach_panel_to_archive(panel_id, entry.path)
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

    fn handle_command_line_input(&mut self, key: &KeyEvent) -> Option<bool> {
        if self.state.dialog.is_some() || self.state.screen_mode == ScreenMode::Viewer {
            return None;
        }
        if self.input_mode.is_some() {
            return None;
        }

        if !self.state.command_line.active {
            if key.code == KeyCode::Char(':') && key.modifiers.is_empty() {
                self.state.command_line.active = true;
                self.state.status_line = "command line active".to_string();
                return Some(true);
            }
            return None;
        }

        match key.code {
            KeyCode::Esc => {
                self.state.command_line.active = false;
                self.state.command_line.input.clear();
                self.state.status_line = "command line canceled".to_string();
                Some(true)
            }
            KeyCode::Enter => {
                self.state.command_line.active = false;
                let command = std::mem::take(&mut self.state.command_line.input);
                match self.execute_command_line(command.as_str()) {
                    Ok(redraw) => Some(redraw),
                    Err(err) => {
                        self.show_alert(format!("command failed: {err}"));
                        Some(true)
                    }
                }
            }
            KeyCode::Backspace => {
                self.state.command_line.input.pop();
                Some(true)
            }
            KeyCode::Char(c)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.state.command_line.input.push(c);
                Some(true)
            }
            _ => Some(false),
        }
    }

    fn execute_command_line(&mut self, raw: &str) -> Result<bool> {
        let command = raw.trim();
        if command.is_empty() {
            self.state.status_line = "command line: empty".to_string();
            return Ok(true);
        }

        if command == "cd" {
            return self.go_to_home();
        }

        if let Some(path) = command.strip_prefix("cd ") {
            return self.command_line_cd(path.trim());
        }

        if matches!(command, "sh" | "shell") {
            return self.open_shell_mode();
        }

        if looks_like_path_command(command) {
            return self.command_line_cd(command);
        }

        if is_interactive_command(command) {
            return self.run_tty_shell_command(command);
        }

        self.command_line_run_shell(command)
    }

    fn command_line_cd(&mut self, raw_path: &str) -> Result<bool> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return self.go_to_home();
        }

        let current = self.active_panel().cwd.clone();
        let candidate = resolve_command_path(trimmed, &current, self.active_backend_spec());
        let normalized = self
            .active_backend()
            .normalize_existing_path("command_cd", &candidate)?;

        let panel = self.active_panel_mut();
        panel.cwd = normalized.clone();
        panel.find_view = None;
        panel.selected_index = 0;
        panel.clear_selection_anchor();
        self.push_log(format!("cd {}", normalized.display()));
        self.reload_panel(self.state.active_panel, false)
    }

    fn command_line_run_shell(&mut self, command: &str) -> Result<bool> {
        if !matches!(self.active_backend_spec(), BackendSpec::Local) {
            self.show_alert("shell commands are available only on local panel");
            return Ok(true);
        }

        let cwd = self.active_panel().cwd.clone();
        let output = run_shell_command_capture(command, &cwd)?;
        self.reload_panel(PanelId::Left, false)?;
        self.reload_panel(PanelId::Right, false)?;
        self.open_command_output_viewer(command, &output);
        Ok(true)
    }

    fn run_tty_shell_command(&mut self, command: &str) -> Result<bool> {
        if !matches!(self.active_backend_spec(), BackendSpec::Local) {
            self.show_alert("interactive commands are available only on local panel");
            return Ok(true);
        }
        let cwd = self.active_panel().cwd.clone();

        runtime::set_input_poll_paused(true);
        if let Err(err) = terminal::suspend_for_external_process() {
            runtime::set_input_poll_paused(false);
            return Err(err);
        }

        let run_result = run_shell_command_tty(command, &cwd);
        let resume_result = terminal::resume_after_external_process();
        runtime::set_input_poll_paused(false);
        resume_result?;
        self.force_full_redraw = true;

        let status = run_result?;
        if !status.success() {
            self.push_log(format!("interactive command exited with status: {status}"));
        } else {
            self.push_log(format!("interactive command done: {command}"));
        }
        self.reload_panel(PanelId::Left, false)?;
        self.reload_panel(PanelId::Right, false)?;
        Ok(true)
    }

    fn open_shell_mode(&mut self) -> Result<bool> {
        if self.state.dialog.is_some() || self.state.screen_mode == ScreenMode::Viewer {
            return Ok(false);
        }
        if self.input_mode.is_some() || self.state.command_line.active {
            return Ok(false);
        }

        let panel_id = self.state.active_panel;
        let initial_cwd = match self.backend_spec(panel_id) {
            BackendSpec::Local => self.active_panel().cwd.clone(),
            BackendSpec::Sftp(_) | BackendSpec::Archive(_) => self.last_local_cwd(panel_id),
        };
        let cwd = if initial_cwd.exists() {
            initial_cwd
        } else {
            env::current_dir().map_err(|err| anyhow::anyhow!("cannot resolve local cwd: {err}"))?
        };

        runtime::set_input_poll_paused(true);
        if let Err(err) = terminal::suspend_for_external_process() {
            runtime::set_input_poll_paused(false);
            return Err(err);
        }

        let run_result = run_interactive_shell(&cwd);
        let resume_result = terminal::resume_after_external_process();
        runtime::set_input_poll_paused(false);
        resume_result?;
        self.force_full_redraw = true;
        run_result?;

        let _ = self.reload_panel(PanelId::Left, false);
        let _ = self.reload_panel(PanelId::Right, false);
        self.push_log("shell mode closed");
        Ok(true)
    }

    fn open_command_output_viewer(&mut self, command: &str, output: &std::process::Output) {
        let mut lines = Vec::new();
        let mut raw = Vec::new();

        if !output.stdout.is_empty() {
            let stdout_text = String::from_utf8_lossy(&output.stdout);
            lines.extend(stdout_text.lines().map(|line| line.to_string()));
            raw.extend_from_slice(output.stdout.as_slice());
        }

        if !output.stderr.is_empty() {
            if !lines.is_empty() {
                lines.push(String::new());
                raw.push(b'\n');
            }
            lines.push("[stderr]".to_string());
            let stderr_text = String::from_utf8_lossy(&output.stderr);
            lines.extend(stderr_text.lines().map(|line| line.to_string()));
            raw.extend_from_slice(output.stderr.as_slice());
        }

        if lines.is_empty() {
            lines.push("(no output)".to_string());
        }
        lines.push(String::new());
        lines.push(format!("[exit status: {}]", output.status));

        let byte_size = (output.stdout.len() + output.stderr.len()) as u64;
        let mut state = load_viewer_state_from_preview(
            PathBuf::from(format!("<cmd:{command}>")),
            format!("Command Output: {command}"),
            byte_size,
            raw,
            false,
        );
        state.mode = ViewerMode::Text;
        state.text_lines = lines.clone();
        state.lines = lines;
        refresh_viewer_search(&mut state);
        self.state.viewer = Some(state);
        self.state.screen_mode = ScreenMode::Viewer;
        self.pending_viewer_search = false;
        self.state.status_line = format!("command output: {command}");
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
        self.state.top_menu.open = false;
        self.pending_confirmation = None;
        self.pending_rename = None;
        self.pending_mask = None;
        self.pending_sftp_connect = None;
        self.pending_conflict = None;
        self.pending_find = None;
        self.pending_editor_choice = None;
        self.pending_viewer_search = false;
        self.state.dialog = Some(alert_dialog(message.clone()));
        self.push_log(message);
    }
}

fn map_key_to_command(key: &KeyEvent) -> Option<Command> {
    match key.code {
        KeyCode::Char('q') => Some(Command::Quit),
        KeyCode::Tab => Some(Command::SwitchPanel),
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Command::OpenShell)
        }
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => Some(Command::SelectRangeUp),
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(Command::SelectRangeDown)
        }
        KeyCode::Char(c)
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(c, 'a' | 'A' | '\u{1}') =>
        {
            Some(Command::MoveSelectionTop)
        }
        KeyCode::Char(c)
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(c, 'e' | 'E' | '\u{5}') =>
        {
            Some(Command::MoveSelectionBottom)
        }
        KeyCode::Up
            if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT =>
        {
            Some(Command::MoveSelectionTop)
        }
        KeyCode::Down
            if key.modifiers != KeyModifiers::NONE && key.modifiers != KeyModifiers::SHIFT =>
        {
            Some(Command::MoveSelectionBottom)
        }
        KeyCode::Up => Some(Command::MoveSelectionUp),
        KeyCode::Down => Some(Command::MoveSelectionDown),
        KeyCode::Home => Some(Command::MoveSelectionTop),
        KeyCode::End => Some(Command::MoveSelectionBottom),
        KeyCode::PageUp => Some(Command::MoveSelectionTop),
        KeyCode::PageDown => Some(Command::MoveSelectionBottom),
        KeyCode::Enter => Some(Command::OpenSelected),
        KeyCode::Backspace => Some(Command::GoToParent),
        KeyCode::Char(' ') | KeyCode::Insert => Some(Command::ToggleSelectCurrent),
        KeyCode::Char('+') => Some(Command::StartSelectByMask),
        KeyCode::Char('-') => Some(Command::StartDeselectByMask),
        KeyCode::Char('*') => Some(Command::InvertSelection),
        KeyCode::F(3) => Some(Command::OpenViewer),
        KeyCode::F(4) => Some(Command::OpenEditor),
        KeyCode::F(5) => Some(Command::Copy),
        KeyCode::F(6) => Some(Command::Move),
        KeyCode::F(7) => Some(Command::Mkdir),
        KeyCode::F(8) => Some(Command::Delete),
        KeyCode::F(9) => Some(Command::OpenTopMenu),
        KeyCode::F(10) => Some(Command::Quit),
        KeyCode::F(2) => Some(Command::ToggleSort),
        KeyCode::Char('r') => Some(Command::Refresh),
        KeyCode::Char('/') if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Some(Command::StartSearch)
        }
        KeyCode::Char('~') => Some(Command::GoHome),
        _ => None,
    }
}

fn map_viewer_key_to_command(key: &KeyEvent) -> Option<Command> {
    match key.code {
        KeyCode::Esc | KeyCode::F(3) => Some(Command::CloseViewer),
        KeyCode::Char('q') if key.modifiers.is_empty() => Some(Command::CloseViewer),
        KeyCode::F(2) => Some(Command::ViewerToggleMode),
        KeyCode::Char('/') if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            Some(Command::ViewerStartSearch)
        }
        KeyCode::Char('n') if key.modifiers.is_empty() => Some(Command::ViewerSearchNext),
        KeyCode::Char('N') if key.modifiers == KeyModifiers::SHIFT => {
            Some(Command::ViewerSearchPrev)
        }
        KeyCode::Up => Some(Command::ViewerScrollUp),
        KeyCode::Down => Some(Command::ViewerScrollDown),
        KeyCode::PageUp => Some(Command::ViewerPageUp),
        KeyCode::PageDown => Some(Command::ViewerPageDown),
        KeyCode::Home => Some(Command::ViewerTop),
        KeyCode::End => Some(Command::ViewerBottom),
        _ => None,
    }
}

fn run_external_editor_command(editor: &str, path: &Path) -> Result<std::process::ExitStatus> {
    let command_line = format!("{editor} {}", shell_escape_path(path));
    let mut cmd = ProcessCommand::new("sh");
    cmd.arg("-lc").arg(command_line);
    run_status_with_sigint_protection(&mut cmd, "failed to launch EDITOR")
}

fn detect_editor_candidates() -> Vec<EditorCandidate> {
    let definitions = [
        ("Neovim", "nvim"),
        ("Vim", "vim"),
        ("Nano", "nano"),
        ("Helix", "hx"),
        ("Micro", "micro"),
        ("Emacs", "emacs"),
        ("VS Code", "code -w"),
    ];
    let mut candidates = Vec::new();
    for (label, command) in definitions {
        let binary = command.split_whitespace().next().unwrap_or_default();
        if command_in_path(binary) {
            candidates.push(EditorCandidate {
                label: format!("{label} ({command})"),
                command: command.to_string(),
            });
        }
    }
    candidates
}

fn build_editor_choice_body(options: &[EditorCandidate]) -> String {
    let mut lines = Vec::with_capacity(options.len() + 3);
    lines.push("Choose default editor (used when $EDITOR is unset):".to_string());
    for (idx, option) in options.iter().enumerate() {
        lines.push(format!("{}: {}", idx + 1, option.label));
    }
    lines.push(String::new());
    lines.push("Enter number, then Apply.".to_string());
    lines.join("\n")
}

fn load_saved_editor_command() -> Option<String> {
    let path = editor_config_path()?;
    let content = fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "editor" {
            continue;
        }
        let parsed = parse_editor_value(value.trim());
        if parsed.is_some() {
            return parsed;
        }
    }
    None
}

fn save_editor_command(command: &str) -> Result<()> {
    let path = editor_config_path()
        .ok_or_else(|| anyhow::anyhow!("cannot resolve config path (HOME/XDG_CONFIG_HOME)"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = format!(
        "# vcmc config\neditor = \"{}\"\n",
        escape_toml_string(command.trim())
    );
    fs::write(path, content)?;
    Ok(())
}

fn editor_config_path() -> Option<PathBuf> {
    if let Some(base) = env::var_os("XDG_CONFIG_HOME") {
        let base = PathBuf::from(base);
        if !base.as_os_str().is_empty() {
            return Some(base.join("vcmc").join("config.toml"));
        }
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".config").join("vcmc").join("config.toml"))
}

fn parse_editor_value(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2 {
        let inner = &raw[1..raw.len() - 1];
        return Some(unescape_toml_string(inner));
    }
    Some(raw.to_string())
}

fn escape_toml_string(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unescape_toml_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next) = chars.next() {
                match next {
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    other => {
                        out.push('\\');
                        out.push(other);
                    }
                }
            } else {
                out.push('\\');
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn command_in_path(binary: &str) -> bool {
    if binary.trim().is_empty() {
        return false;
    }
    if binary.contains('/') {
        return is_executable_file(Path::new(binary));
    }
    let Some(path_var) = env::var_os("PATH") else {
        return false;
    };
    for dir in env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if is_executable_file(candidate.as_path()) {
            return true;
        }
    }
    false
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = path.metadata() {
            return metadata.permissions().mode() & 0o111 != 0;
        }
    }
    #[cfg(not(unix))]
    {
        return true;
    }
    false
}

fn run_shell_command_capture(command: &str, cwd: &Path) -> Result<std::process::Output> {
    let shell = shell_program();
    ProcessCommand::new(shell.as_str())
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .output()
        .map_err(|err| anyhow::anyhow!("failed to execute shell command: {err}"))
}

fn run_interactive_shell(cwd: &Path) -> Result<std::process::ExitStatus> {
    let shell = shell_program();
    let mut cmd = ProcessCommand::new(shell.as_str());
    cmd.current_dir(cwd);
    run_status_with_sigint_protection(&mut cmd, "failed to launch shell")
}

fn run_shell_command_tty(command: &str, cwd: &Path) -> Result<std::process::ExitStatus> {
    let shell = shell_program();
    let mut cmd = ProcessCommand::new(shell.as_str());
    cmd.arg("-lc").arg(command).current_dir(cwd);
    run_status_with_sigint_protection(&mut cmd, "failed to execute interactive command")
}

fn shell_program() -> String {
    env::var("SHELL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "sh".to_string())
}

fn run_status_with_sigint_protection(
    cmd: &mut ProcessCommand,
    context: &str,
) -> Result<std::process::ExitStatus> {
    let mut child = cmd
        .spawn()
        .map_err(|err| anyhow::anyhow!("{context}: {err}"))?;

    #[cfg(unix)]
    let _sigint_guard = SigintIgnoreGuard::new()?;

    child
        .wait()
        .map_err(|err| anyhow::anyhow!("{context}: {err}"))
}

#[cfg(unix)]
struct SigintIgnoreGuard {
    previous: libc::sighandler_t,
}

#[cfg(unix)]
impl SigintIgnoreGuard {
    fn new() -> Result<Self> {
        let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
        if previous == libc::SIG_ERR {
            return Err(anyhow::anyhow!("failed to ignore SIGINT"));
        }
        Ok(Self { previous })
    }
}

#[cfg(unix)]
impl Drop for SigintIgnoreGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::signal(libc::SIGINT, self.previous);
        }
    }
}

fn is_interactive_command(command: &str) -> bool {
    let head = command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        head.as_str(),
        "top"
            | "htop"
            | "btop"
            | "vim"
            | "vi"
            | "nvim"
            | "nano"
            | "less"
            | "more"
            | "man"
            | "watch"
            | "ssh"
    )
}

fn shell_escape_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    format!("'{}'", raw.replace('\'', r#"'"'"'"#))
}

fn looks_like_path_command(command: &str) -> bool {
    command.starts_with('/')
        || command.starts_with("./")
        || command.starts_with("../")
        || command == ".."
        || command.starts_with("~/")
        || command == "~"
}

fn resolve_command_path(raw_path: &str, current: &Path, backend_spec: &BackendSpec) -> PathBuf {
    if raw_path == "~" || raw_path.starts_with("~/") {
        return match backend_spec {
            BackendSpec::Local => expand_tilde_path(raw_path),
            BackendSpec::Sftp(info) => {
                if raw_path == "~" {
                    info.root_path.clone()
                } else if let Some(rest) = raw_path.strip_prefix("~/") {
                    info.root_path.join(rest)
                } else {
                    info.root_path.clone()
                }
            }
            BackendSpec::Archive(_) => PathBuf::from("/"),
        };
    }

    let candidate = PathBuf::from(raw_path);
    if candidate.is_absolute() {
        candidate
    } else {
        current.join(candidate)
    }
}

fn source_item_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

fn parse_sftp_address_input(input: &str) -> Result<(Option<String>, String, u16, PathBuf)> {
    let mut value = input.trim().to_string();
    if value.is_empty() {
        return Err(anyhow::anyhow!("address cannot be empty"));
    }
    if let Some(rest) = value.strip_prefix("sftp://") {
        value = rest.to_string();
    }

    let (endpoint, path_part) = if let Some((left, right)) = value.split_once('/') {
        (left.to_string(), format!("/{}", right))
    } else {
        (value, "/".to_string())
    };

    let (user, host_port) = if let Some((user, host_part)) = endpoint.split_once('@') {
        (Some(user.to_string()), host_part.to_string())
    } else {
        (None, endpoint)
    };
    let (host, port) = if let Some((host, port_raw)) = host_port.rsplit_once(':') {
        if port_raw.chars().all(|ch| ch.is_ascii_digit()) {
            let port = port_raw
                .parse::<u16>()
                .map_err(|_| anyhow::anyhow!("invalid port: {port_raw}"))?;
            (host.to_string(), port)
        } else {
            (host_port, 22)
        }
    } else {
        (host_port, 22)
    };

    if host.trim().is_empty() {
        return Err(anyhow::anyhow!("host cannot be empty"));
    }
    Ok((user, host, port, PathBuf::from(path_part)))
}

fn probe_sftp_auth_hint(host: &str, port: u16, user: &str) -> Result<SftpAuthHint> {
    let endpoint = format!("{host}:{port}");
    let tcp = TcpStream::connect(endpoint.as_str())?;
    tcp.set_read_timeout(Some(Duration::from_secs(8)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(8)))?;

    let mut session = Session::new()?;
    session.set_tcp_stream(tcp);
    session.handshake()?;
    let methods = session.auth_methods(user).unwrap_or_default();
    let has_publickey = methods
        .split(',')
        .any(|method| method.trim() == "publickey");
    let has_password = methods
        .split(',')
        .any(|method| method.trim() == "password" || method.trim() == "keyboard-interactive");

    let hint = match (has_publickey, has_password) {
        (true, false) => SftpAuthHint::KeyOnly,
        (false, true) => SftpAuthHint::PasswordOnly,
        (true, true) => SftpAuthHint::Either,
        (false, false) => SftpAuthHint::Either,
    };
    Ok(hint)
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    if raw == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
        return PathBuf::from(raw);
    }

    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    PathBuf::from(raw)
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

fn compare_mtime(
    source: Option<std::time::SystemTime>,
    destination: Option<std::time::SystemTime>,
) -> Option<std::cmp::Ordering> {
    match (source, destination) {
        (Some(src), Some(dst)) => Some(src.cmp(&dst)),
        _ => None,
    }
}

fn format_mtime_hint(value: Option<std::time::SystemTime>) -> String {
    match value {
        Some(ts) => format!("{ts:?}"),
        None => "-".to_string(),
    }
}

fn viewer_mode_label(mode: ViewerMode) -> &'static str {
    match mode {
        ViewerMode::Text => "text",
        ViewerMode::Hex => "hex",
    }
}

fn viewer_match_status(viewer: &ViewerState) -> String {
    let total = viewer.search_matches.len();
    if total == 0 {
        return "viewer search: no matches".to_string();
    }
    let current = viewer.search_match_index + 1;
    format!(
        "viewer search [{}/{}]: '{}' ({})",
        current,
        total,
        viewer.search_query,
        viewer_mode_label(viewer.mode)
    )
}

fn split_name_and_extension(name: &str) -> (String, String) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_string()),
        _ => (name.to_string(), String::new()),
    }
}

fn copy_move_item_message(kind: JobKind, item: &BatchOpItem) -> String {
    let destination = item
        .destination
        .as_ref()
        .map(|path| source_item_label(path.as_path()))
        .unwrap_or_else(|| "-".to_string());
    if item.overwrite_destination {
        format!(
            "{} queued (overwrite): {} -> {}",
            operation_name(kind),
            item.name,
            destination
        )
    } else if destination != item.name {
        format!(
            "{} queued: {} -> {}",
            operation_name(kind),
            item.name,
            destination
        )
    } else {
        format!("{} queued: {}", operation_name(kind), item.name)
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

fn top_menu_group_from_key(key: &KeyEvent) -> Option<usize> {
    if !key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    menu_group_index_by_hotkey(c)
}

fn panel_name(panel_id: PanelId) -> &'static str {
    match panel_id {
        PanelId::Left => "left",
        PanelId::Right => "right",
    }
}

fn is_archive_backend(spec: &BackendSpec) -> bool {
    matches!(spec, BackendSpec::Archive(_))
}

fn backend_spec_label(spec: &BackendSpec) -> String {
    match spec {
        BackendSpec::Local => "local".to_string(),
        BackendSpec::Sftp(info) => format!("sftp:{}@{}", info.user, info.host),
        BackendSpec::Archive(info) => {
            let name = info
                .archive_path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| info.archive_path.display().to_string());
            format!("archive:{name}")
        }
    }
}

fn parent_link_entry(parent: PathBuf) -> FsEntry {
    FsEntry {
        name: "..".to_string(),
        path: parent,
        entry_type: FsEntryType::Directory,
        size_bytes: 0,
        modified_at: None,
        is_executable: false,
        is_hidden: false,
        is_virtual: true,
    }
}

fn sort_find_entries(entries: &mut [FsEntry], sort_mode: SortMode) {
    entries.sort_by(|left, right| {
        let type_cmp = find_entry_group(left).cmp(&find_entry_group(right));
        if type_cmp != std::cmp::Ordering::Equal {
            return type_cmp;
        }

        match sort_mode {
            SortMode::Name => left
                .name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.name.cmp(&right.name)),
            SortMode::Size => left
                .size_bytes
                .cmp(&right.size_bytes)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.name.cmp(&right.name)),
            SortMode::ModifiedAt => left
                .modified_at
                .cmp(&right.modified_at)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.name.cmp(&right.name)),
        }
    });
}

fn find_entry_group(entry: &FsEntry) -> u8 {
    if entry.entry_type == FsEntryType::Directory {
        return 0;
    }
    if entry.entry_type == FsEntryType::Symlink {
        return 1;
    }
    if entry.entry_type == FsEntryType::File {
        return 2;
    }
    3
}

fn first_selectable_menu_item(items: &[crate::menu::MenuItemSpec]) -> Option<usize> {
    items
        .iter()
        .position(crate::menu::MenuItemSpec::is_selectable)
}

fn next_selectable_menu_item(
    items: &[crate::menu::MenuItemSpec],
    current: usize,
    forward: bool,
) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let len = items.len();
    for step in 1..=len {
        let idx = if forward {
            (current + step) % len
        } else {
            (current + len - (step % len)) % len
        };
        if items[idx].is_selectable() {
            return Some(idx);
        }
    }
    None
}

fn input_dialog(title: &str, body: &str, value: String, tone: DialogTone) -> DialogState {
    input_dialog_with_mask(title, body, value, tone, false)
}

fn input_dialog_with_mask(
    title: &str,
    body: &str,
    value: String,
    tone: DialogTone,
    mask_input: bool,
) -> DialogState {
    DialogState {
        title: title.to_string(),
        body: body.to_string(),
        input_value: Some(value),
        mask_input,
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
        mask_input: false,
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

fn conflict_dialog(title: String, body: String) -> DialogState {
    DialogState {
        title,
        body,
        input_value: None,
        mask_input: false,
        buttons: vec![
            DialogButton {
                label: "Overwrite".to_string(),
                accelerator: Some('o'),
                role: DialogButtonRole::Primary,
            },
            DialogButton {
                label: "Skip".to_string(),
                accelerator: Some('s'),
                role: DialogButtonRole::Secondary,
            },
            DialogButton {
                label: "Rename".to_string(),
                accelerator: Some('r'),
                role: DialogButtonRole::Secondary,
            },
            DialogButton {
                label: "Newer".to_string(),
                accelerator: Some('n'),
                role: DialogButtonRole::Primary,
            },
            DialogButton {
                label: "OverAll".to_string(),
                accelerator: Some('w'),
                role: DialogButtonRole::Primary,
            },
            DialogButton {
                label: "SkipAll".to_string(),
                accelerator: Some('k'),
                role: DialogButtonRole::Secondary,
            },
            DialogButton {
                label: "NewerAll".to_string(),
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
        tone: DialogTone::Warning,
    }
}

fn alert_dialog(body: String) -> DialogState {
    DialogState {
        title: "Error".to_string(),
        body,
        input_value: None,
        mask_input: false,
        buttons: vec![DialogButton {
            label: "OK".to_string(),
            accelerator: Some('o'),
            role: DialogButtonRole::Primary,
        }],
        focused_button: 0,
        tone: DialogTone::Danger,
    }
}
