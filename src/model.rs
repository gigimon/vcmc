#![allow(dead_code)]

use std::path::PathBuf;
use std::time::SystemTime;

use crossterm::event::KeyEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelId {
    Left,
    Right,
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
    pub is_hidden: bool,
}

#[derive(Debug, Clone)]
pub struct PanelState {
    pub cwd: PathBuf,
    pub entries: Vec<FsEntry>,
    pub selected_index: usize,
    pub sort_mode: SortMode,
    pub show_hidden: bool,
    pub error_message: Option<String>,
}

impl PanelState {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            entries: Vec::new(),
            selected_index: 0,
            sort_mode: SortMode::Name,
            show_hidden: false,
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
}

#[derive(Debug, Clone)]
pub struct TerminalSize {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub active_panel: PanelId,
    pub left_panel: PanelState,
    pub right_panel: PanelState,
    pub status_line: String,
    pub jobs: Vec<Job>,
    pub terminal_size: TerminalSize,
}

impl AppState {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            active_panel: PanelId::Left,
            left_panel: PanelState::new(cwd.clone()),
            right_panel: PanelState::new(cwd),
            status_line: "Ready".to_string(),
            jobs: Vec::new(),
            terminal_size: TerminalSize {
                width: 0,
                height: 0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
    SwitchPanel,
    MoveSelectionUp,
    MoveSelectionDown,
    OpenSelected,
    GoToParent,
    GoHome,
    Refresh,
    Copy,
    Move,
    Delete,
    Mkdir,
    ToggleSort,
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
    pub kind: JobKind,
    pub status: JobStatus,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JobRequest {
    pub id: u64,
    pub kind: JobKind,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct JobUpdate {
    pub id: u64,
    pub kind: JobKind,
    pub status: JobStatus,
    pub source: PathBuf,
    pub destination: Option<PathBuf>,
    pub message: Option<String>,
}

impl JobUpdate {
    pub fn into_job(self) -> Job {
        Job {
            id: self.id,
            kind: self.kind,
            status: self.status,
            source: self.source,
            destination: self.destination,
            message: self.message,
        }
    }
}
