use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::model::{AppState, FsEntry, FsEntryType, JobStatus, PanelId, PanelState, SortMode};

pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_header(frame, chunks[0], state);

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    render_panel(
        frame,
        panel_chunks[0],
        "Left",
        &state.left_panel,
        state.active_panel == PanelId::Left,
    );
    render_panel(
        frame,
        panel_chunks[1],
        "Right",
        &state.right_panel,
        state.active_panel == PanelId::Right,
    );

    render_status(frame, chunks[2], state);
    render_help(frame, chunks[3]);
}

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    let (queued, running, failed) = job_counters(state);
    let active = match state.active_panel {
        PanelId::Left => "LEFT",
        PanelId::Right => "RIGHT",
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "VCMC ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "active:{active}  size:{}x{}  jobs q/r/f:{queued}/{running}/{failed}",
            state.terminal_size.width, state.terminal_size.height
        )),
    ]));
    frame.render_widget(header, area);
}

fn render_panel(frame: &mut Frame, area: Rect, name: &str, panel: &PanelState, active: bool) {
    let border_style = if active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = format!(
        "{name} [{}] {}",
        sort_label(panel.sort_mode),
        panel.cwd.display()
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    if let Some(error) = &panel.error_message {
        let content = Paragraph::new(Line::from(vec![
            Span::styled(
                "ERROR: ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(error.as_str()),
        ]))
        .block(block);
        frame.render_widget(content, area);
        return;
    }

    if panel.entries.is_empty() {
        let content = Paragraph::new("Empty directory")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(content, area);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = build_entry_lines(panel, active, inner.height as usize);
    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);
}

fn build_entry_lines(
    panel: &PanelState,
    panel_active: bool,
    capacity: usize,
) -> Vec<Line<'static>> {
    if capacity == 0 {
        return Vec::new();
    }

    let selected = panel
        .selected_index
        .min(panel.entries.len().saturating_sub(1));
    let start = visible_window_start(selected, panel.entries.len(), capacity);
    let end = (start + capacity).min(panel.entries.len());

    panel.entries[start..end]
        .iter()
        .enumerate()
        .map(|(offset, entry)| {
            let idx = start + offset;
            let selected_style = if idx == selected {
                if panel_active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().bg(Color::DarkGray)
                }
            } else {
                Style::default()
            };

            let marker = match entry.entry_type {
                FsEntryType::Directory => "D",
                FsEntryType::File => "F",
                FsEntryType::Symlink => "L",
                FsEntryType::Other => "?",
            };
            let name_style = selected_style.patch(type_style(entry));
            let nav = if entry.is_virtual { ">" } else { " " };
            let hidden = if entry.is_hidden { "." } else { " " };
            let line = format!(
                "{marker} {:>7} {nav}{hidden} {}",
                human_size(entry.size_bytes),
                entry.name
            );
            Line::styled(line, name_style)
        })
        .collect()
}

fn render_status(frame: &mut Frame, area: Rect, state: &AppState) {
    let status = Paragraph::new(state.status_line.clone())
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(status, area);
}

fn render_help(frame: &mut Frame, area: Rect) {
    let help = Paragraph::new(
        "Tab switch  Arrows move  Enter open  Backspace up  Home/~ home  F2 sort  F5/F6/F7/F8 ops  q quit",
    );
    frame.render_widget(help, area);
}

fn type_style(entry: &FsEntry) -> Style {
    if entry.is_virtual {
        return Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
    }

    match entry.entry_type {
        FsEntryType::Directory => Style::default().fg(Color::Blue),
        FsEntryType::Symlink => Style::default().fg(Color::Magenta),
        _ => Style::default(),
    }
}

fn visible_window_start(selected: usize, total: usize, capacity: usize) -> usize {
    if total <= capacity {
        return 0;
    }
    let half = capacity / 2;
    let mut start = selected.saturating_sub(half);
    let max_start = total.saturating_sub(capacity);
    if start > max_start {
        start = max_start;
    }
    start
}

fn sort_label(mode: SortMode) -> &'static str {
    match mode {
        SortMode::Name => "name",
        SortMode::Size => "size",
        SortMode::ModifiedAt => "mtime",
    }
}

fn job_counters(state: &AppState) -> (usize, usize, usize) {
    let mut queued = 0;
    let mut running = 0;
    let mut failed = 0;

    for job in &state.jobs {
        match job.status {
            JobStatus::Queued => queued += 1,
            JobStatus::Running => running += 1,
            JobStatus::Failed => failed += 1,
            JobStatus::Done => {}
        }
    }

    (queued, running, failed)
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut size = bytes as f64;
    let mut unit_idx = 0usize;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{bytes}{}", UNITS[unit_idx])
    } else {
        format!("{size:.1}{}", UNITS[unit_idx])
    }
}
