#![allow(dead_code)]

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

use crossterm::event::KeyEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelId {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SftpAuth {
    Agent,
    Password(String),
    KeyFile {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpConnectionInfo {
    pub host: String,
    pub user: String,
    pub port: u16,
    pub root_path: PathBuf,
    pub auth: SftpAuth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendSpec {
    Local,
    Sftp(SftpConnectionInfo),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Name,
    Size,
    ModifiedAt,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::Size,
            Self::Size => Self::ModifiedAt,
            Self::ModifiedAt => Self::Name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsEntryType {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub entry_type: FsEntryType,
    pub size_bytes: u64,
    pub modified_at: Option<SystemTime>,
    pub is_executable: bool,
    pub is_hidden: bool,
    pub is_virtual: bool,
}

#[derive(Debug, Clone)]
pub struct PanelState {
    pub cwd: PathBuf,
    pub all_entries: Vec<FsEntry>,
    pub entries: Vec<FsEntry>,
    pub selected_index: usize,
    pub sort_mode: SortMode,
    pub show_hidden: bool,
    pub search_query: String,
    pub selected_paths: HashSet<PathBuf>,
    pub selection_anchor: Option<usize>,
    pub error_message: Option<String>,
}

impl PanelState {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            all_entries: Vec::new(),
            entries: Vec::new(),
            selected_index: 0,
            sort_mode: SortMode::Name,
            show_hidden: false,
            search_query: String::new(),
            selected_paths: HashSet::new(),
            selection_anchor: None,
            error_message: None,
        }
    }

    pub fn move_selection_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    pub fn move_selection_down(&mut self) {
        if self.entries.is_empty() {
            self.selected_index = 0;
            return;
        }

        let last = self.entries.len().saturating_sub(1);
        self.selected_index = (self.selected_index + 1).min(last);
    }

    pub fn normalize_selection(&mut self) {
        if self.entries.is_empty() {
            self.selected_index = 0;
            return;
        }

        let last = self.entries.len().saturating_sub(1);
        self.selected_index = self.selected_index.min(last);
    }

    pub fn selected_entry(&self) -> Option<&FsEntry> {
        self.entries.get(self.selected_index)
    }

    pub fn set_entries(&mut self, entries: Vec<FsEntry>) {
        let current_paths: HashSet<PathBuf> =
            entries.iter().map(|entry| entry.path.clone()).collect();
        self.selected_paths
            .retain(|path| current_paths.contains(path));
        self.all_entries = entries;
        self.apply_search_filter();
        self.selection_anchor = None;
    }

    pub fn clear_search(&mut self) {
        self.search_query.clear();
        self.apply_search_filter();
    }

    pub fn apply_search_filter(&mut self) {
        if self.search_query.trim().is_empty() {
            self.entries = self.all_entries.clone();
            self.normalize_selection();
            return;
        }

        let needle = self.search_query.to_lowercase();
        self.entries = self
            .all_entries
            .iter()
            .filter(|entry| entry.is_virtual || entry.name.to_lowercase().contains(needle.as_str()))
            .cloned()
            .collect();
        self.normalize_selection();
    }

    pub fn clear_selection_anchor(&mut self) {
        self.selection_anchor = None;
    }

    pub fn is_selected(&self, entry: &FsEntry) -> bool {
        self.selected_paths.contains(&entry.path)
    }

    pub fn toggle_current_selection(&mut self) -> bool {
        let Some(entry) = self.selected_entry().cloned() else {
            return false;
        };
        if entry.is_virtual {
            return false;
        }

        if self.selected_paths.contains(&entry.path) {
            self.selected_paths.remove(&entry.path);
            true
        } else {
            self.selected_paths.insert(entry.path);
            true
        }
    }

    pub fn select_range_from_anchor(&mut self, previous_index: usize, new_index: usize) -> usize {
        if self.entries.is_empty() {
            return 0;
        }

        let max_idx = self.entries.len().saturating_sub(1);
        let previous_index = previous_index.min(max_idx);
        let new_index = new_index.min(max_idx);
        let anchor = *self.selection_anchor.get_or_insert(previous_index);

        let start = anchor.min(new_index);
        let end = anchor.max(new_index);
        let mut changed = 0usize;
        for idx in start..=end {
            if let Some(entry) = self.entries.get(idx) {
                if entry.is_virtual {
                    continue;
                }
                if self.selected_paths.insert(entry.path.clone()) {
                    changed += 1;
                }
            }
        }

        changed
    }

    pub fn select_by_mask(&mut self, mask: &str) -> usize {
        let mask = normalize_mask(mask);
        let mut changed = 0usize;
        for entry in &self.all_entries {
            if entry.is_virtual {
                continue;
            }
            if wildcard_match(mask.as_str(), entry.name.as_str())
                && self.selected_paths.insert(entry.path.clone())
            {
                changed += 1;
            }
        }
        changed
    }

    pub fn deselect_by_mask(&mut self, mask: &str) -> usize {
        let mask = normalize_mask(mask);
        let mut changed = 0usize;
        for entry in &self.all_entries {
            if entry.is_virtual {
                continue;
            }
            if wildcard_match(mask.as_str(), entry.name.as_str())
                && self.selected_paths.remove(&entry.path)
            {
                changed += 1;
            }
        }
        changed
    }

    pub fn invert_selection(&mut self) -> usize {
        let mut changed = 0usize;
        for entry in &self.all_entries {
            if entry.is_virtual {
                continue;
            }
            if self.selected_paths.contains(&entry.path) {
                self.selected_paths.remove(&entry.path);
                changed += 1;
            } else {
                self.selected_paths.insert(entry.path.clone());
                changed += 1;
            }
        }
        changed
    }

    pub fn selection_summary(&self) -> (usize, u64) {
        let mut count = 0usize;
        let mut bytes = 0u64;
        for entry in &self.all_entries {
            if self.selected_paths.contains(&entry.path) {
                count += 1;
                bytes = bytes.saturating_add(entry.size_bytes);
            }
        }
        (count, bytes)
    }
}

#[derive(Debug, Clone)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenMode {
    Normal,
    Viewer,
}

#[derive(Debug, Clone)]
pub struct ViewerState {
    pub path: PathBuf,
    pub title: String,
    pub lines: Vec<String>,
    pub scroll_offset: usize,
    pub is_binary_like: bool,
    pub byte_size: u64,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub screen_mode: ScreenMode,
    pub active_panel: PanelId,
    pub top_menu: TopMenuState,
    pub left_panel: PanelState,
    pub right_panel: PanelState,
    pub status_line: String,
    pub activity_log: Vec<String>,
    pub dialog: Option<DialogState>,
    pub viewer: Option<ViewerState>,
    pub batch_progress: Option<BatchProgressState>,
    pub command_line: CommandLineState,
    pub jobs: Vec<Job>,
    pub terminal_size: TerminalSize,
}

impl AppState {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            screen_mode: ScreenMode::Normal,
            active_panel: PanelId::Left,
            top_menu: TopMenuState::default(),
            left_panel: PanelState::new(cwd.clone()),
            right_panel: PanelState::new(cwd),
            status_line: "Ready".to_string(),
            activity_log: Vec::new(),
            dialog: None,
            viewer: None,
            batch_progress: None,
            command_line: CommandLineState::default(),
            jobs: Vec::new(),
            terminal_size: TerminalSize {
                width: 0,
                height: 0,
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TopMenuState {
    pub open: bool,
    pub group_index: usize,
    pub item_index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct CommandLineState {
    pub input: String,
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct DialogState {
    pub title: String,
    pub body: String,
    pub input_value: Option<String>,
    pub mask_input: bool,
    pub buttons: Vec<DialogButton>,
    pub focused_button: usize,
    pub tone: DialogTone,
}

impl DialogState {
    pub fn focus_next(&mut self) {
        if self.buttons.is_empty() {
            self.focused_button = 0;
            return;
        }
        self.focused_button = (self.focused_button + 1) % self.buttons.len();
    }

    pub fn focus_prev(&mut self) {
        if self.buttons.is_empty() {
            self.focused_button = 0;
            return;
        }
        self.focused_button = if self.focused_button == 0 {
            self.buttons.len().saturating_sub(1)
        } else {
            self.focused_button.saturating_sub(1)
        };
    }

    pub fn focused_button(&self) -> Option<&DialogButton> {
        self.buttons.get(self.focused_button)
    }
}

#[derive(Debug, Clone)]
pub struct DialogButton {
    pub label: String,
    pub accelerator: Option<char>,
    pub role: DialogButtonRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogButtonRole {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogTone {
    Default,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
    SwitchPanel,
    OpenTopMenu,
    ConnectSftp,
    OpenShell,
    MoveSelectionUp,
    MoveSelectionDown,
    MoveSelectionTop,
    MoveSelectionBottom,
    OpenViewer,
    CloseViewer,
    ViewerScrollUp,
    ViewerScrollDown,
    ViewerPageUp,
    ViewerPageDown,
    ViewerTop,
    ViewerBottom,
    OpenEditor,
    OpenSelected,
    GoToParent,
    GoHome,
    Refresh,
    Copy,
    Move,
    Delete,
    Mkdir,
    ToggleSort,
    StartSearch,
    ToggleSelectCurrent,
    StartSelectByMask,
    StartDeselectByMask,
    InvertSelection,
    SelectRangeUp,
    SelectRangeDown,
}

#[derive(Debug, Clone)]
pub enum Event {
    Input(KeyEvent),
    Tick,
    Resize { width: u16, height: u16 },
    Job(JobUpdate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Copy,
    Move,
    Delete,
    Mkdir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub batch_id: Option<u64>,
    pub kind: JobKind,
    pub status: JobStatus,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub current_item: Option<String>,
    pub batch_completed: Option<usize>,
    pub batch_total: Option<usize>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JobRequest {
    pub id: u64,
    pub batch_id: Option<u64>,
    pub kind: JobKind,
    pub source_backend: BackendSpec,
    pub destination_backend: Option<BackendSpec>,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct JobUpdate {
    pub id: u64,
    pub batch_id: Option<u64>,
    pub kind: JobKind,
    pub status: JobStatus,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub current_item: Option<String>,
    pub batch_completed: Option<usize>,
    pub batch_total: Option<usize>,
    pub message: Option<String>,
}

impl JobUpdate {
    pub fn into_job(self) -> Job {
        Job {
            id: self.id,
            batch_id: self.batch_id,
            kind: self.kind,
            status: self.status,
            source: self.source,
            destination: self.destination,
            current_item: self.current_item,
            batch_completed: self.batch_completed,
            batch_total: self.batch_total,
            message: self.message,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BatchProgressState {
    pub batch_id: u64,
    pub operation: JobKind,
    pub current_file: String,
    pub completed: usize,
    pub total: usize,
    pub failed: usize,
}

fn normalize_mask(mask: &str) -> String {
    let trimmed = mask.trim();
    if trimmed.is_empty() {
        "*".to_string()
    } else {
        trimmed.to_string()
    }
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let p_len = pattern.len();
    let t_len = text.len();

    let mut dp = vec![vec![false; t_len + 1]; p_len + 1];
    dp[0][0] = true;
    for i in 1..=p_len {
        if pattern[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }

    for i in 1..=p_len {
        for j in 1..=t_len {
            let p = pattern[i - 1];
            let t = text[j - 1];
            dp[i][j] = match p {
                '*' => dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i - 1][j - 1],
                _ => p == t && dp[i - 1][j - 1],
            };
        }
    }

    dp[p_len][t_len]
}
