use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::model::{
    AppState, DialogButton, DialogButtonRole, DialogState, DialogTone, FsEntry, FsEntryType,
    JobStatus, PanelId, PanelState, SortMode,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
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
    render_log(frame, chunks[3], state);
    render_help(frame, chunks[4]);

    if let Some(dialog) = &state.dialog {
        render_dialog(frame, dialog);
    }
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
        "{name} [{}] {}{}",
        sort_label(panel.sort_mode),
        panel.cwd.display(),
        search_suffix(panel),
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
    let lines = build_entry_lines(panel, active, inner);
    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);
}

fn build_entry_lines(panel: &PanelState, panel_active: bool, inner: Rect) -> Vec<Line<'static>> {
    let capacity = inner.height as usize;
    let total_width = inner.width as usize;
    if capacity <= 1 || total_width == 0 {
        return Vec::new();
    }

    let layout = fixed_table_layout(total_width);
    let rows_capacity = capacity.saturating_sub(1);
    let selected = panel
        .selected_index
        .min(panel.entries.len().saturating_sub(1));
    let start = visible_window_start(selected, panel.entries.len(), rows_capacity);
    let end = (start + rows_capacity).min(panel.entries.len());

    let mut lines = Vec::with_capacity(rows_capacity + 1);
    lines.push(render_table_header(layout));

    for (offset, entry) in panel.entries[start..end].iter().enumerate() {
        let idx = start + offset;
        let is_current = idx == selected;
        let is_marked = panel.is_selected(entry);

        let base_style = if is_current {
            if panel_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(Color::DarkGray)
            }
        } else if is_marked {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        let name = entry_name(entry);
        let size_text = if entry.is_virtual {
            "-".to_string()
        } else {
            human_size(entry.size_bytes)
        };
        let mtime_text = format_modified_at(entry);

        let mut spans = Vec::new();
        match layout {
            TableLayout::Full {
                name_width,
                size_width,
                modified_width,
            } => {
                spans.push(Span::styled(
                    format!(
                        "{:<name_width$}",
                        truncate_name(&name, name_width),
                        name_width = name_width
                    ),
                    base_style.patch(type_style(entry)),
                ));
                spans.push(Span::styled(" ", base_style));
                spans.push(Span::styled(
                    format!(
                        "{:>size_width$}",
                        truncate_name(&size_text, size_width),
                        size_width = size_width
                    ),
                    base_style,
                ));
                spans.push(Span::styled(" ", base_style));
                spans.push(Span::styled(
                    format!(
                        "{:>modified_width$}",
                        truncate_name(&mtime_text, modified_width),
                        modified_width = modified_width
                    ),
                    base_style,
                ));
            }
            TableLayout::Compact {
                name_width,
                size_width,
            } => {
                spans.push(Span::styled(
                    format!(
                        "{:<name_width$}",
                        truncate_name(&name, name_width),
                        name_width = name_width
                    ),
                    base_style.patch(type_style(entry)),
                ));
                spans.push(Span::styled(" ", base_style));
                spans.push(Span::styled(
                    format!(
                        "{:>size_width$}",
                        truncate_name(&size_text, size_width),
                        size_width = size_width
                    ),
                    base_style,
                ));
            }
            TableLayout::Minimal { name_width } => {
                spans.push(Span::styled(
                    format!(
                        "{:<name_width$}",
                        truncate_name(&name, name_width),
                        name_width = name_width
                    ),
                    base_style.patch(type_style(entry)),
                ));
            }
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn render_status(frame: &mut Frame, area: Rect, state: &AppState) {
    let panel = match state.active_panel {
        PanelId::Left => &state.left_panel,
        PanelId::Right => &state.right_panel,
    };
    let (selected_count, selected_bytes) = panel.selection_summary();
    let suffix = if selected_count == 0 {
        " | sel:0".to_string()
    } else {
        format!(" | sel:{selected_count} ({})", human_size(selected_bytes))
    };
    let status = Paragraph::new(format!("{}{}", state.status_line, suffix))
        .style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(status, area);
}

fn render_log(frame: &mut Frame, area: Rect, state: &AppState) {
    let latest = state
        .activity_log
        .last()
        .map(String::as_str)
        .unwrap_or("log: -");
    let log = Paragraph::new(format!("log: {latest}")).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(log, area);
}

fn render_help(frame: &mut Frame, area: Rect) {
    let help = Paragraph::new(
        "Tab switch  Arrows move  Shift+Arrows range  Space/Ins mark  +/-/* mask ops  / search  F5/F6 rename-op  F7 mkdir  F8 delete  Dialog: Tab/Shift+Tab, Left/Right, Enter, Esc, Alt+hotkey",
    );
    frame.render_widget(help, area);
}

fn render_dialog(frame: &mut Frame, dialog: &DialogState) {
    let area = centered_rect(
        78,
        if dialog.input_value.is_some() { 9 } else { 7 },
        frame.area(),
    );
    frame.render_widget(Clear, area);

    let border_color = match dialog.tone {
        DialogTone::Default => Color::Cyan,
        DialogTone::Warning => Color::Yellow,
        DialogTone::Danger => Color::Red,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(dialog.title.as_str())
        .border_style(
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let constraints = if dialog.input_value.is_some() {
        vec![
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]
    } else {
        vec![Constraint::Min(1), Constraint::Length(1)]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let body = Paragraph::new(dialog.body.as_str()).alignment(Alignment::Center);
    frame.render_widget(body, chunks[0]);

    let button_row_idx = if let Some(value) = dialog.input_value.as_ref() {
        let input = Paragraph::new(Line::styled(
            format!("> {value}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::UNDERLINED),
        ));
        frame.render_widget(input, chunks[1]);
        2
    } else {
        1
    };

    let buttons = Paragraph::new(render_button_row(dialog)).alignment(Alignment::Center);
    frame.render_widget(buttons, chunks[button_row_idx]);
}

fn render_button_row(dialog: &DialogState) -> Line<'static> {
    let mut spans = Vec::new();

    for (idx, button) in dialog.buttons.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw(" "));
        }

        let is_focused = idx == dialog.focused_button;
        let label = button_label(button);
        let style = if is_focused {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if button.role == DialogButtonRole::Primary {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(format!("[ {label} ]"), style));
    }

    Line::from(spans)
}

fn button_label(button: &DialogButton) -> String {
    match button.accelerator {
        Some(accel) => format!("Alt+{} {}", accel.to_ascii_uppercase(), button.label),
        None => button.label.clone(),
    }
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

#[derive(Clone, Copy)]
enum TableLayout {
    Full {
        name_width: usize,
        size_width: usize,
        modified_width: usize,
    },
    Compact {
        name_width: usize,
        size_width: usize,
    },
    Minimal {
        name_width: usize,
    },
}

fn fixed_table_layout(total_width: usize) -> TableLayout {
    if total_width < 14 {
        return TableLayout::Minimal {
            name_width: total_width.max(1),
        };
    }

    if total_width < 44 {
        let size_width = 8usize;
        let name_width = total_width.saturating_sub(size_width + 1).max(4);
        return TableLayout::Compact {
            name_width,
            size_width,
        };
    }

    let spaces = 2usize;
    let available = total_width.saturating_sub(spaces);
    let mut name_width = available * 64 / 100;
    let mut size_width = available * 14 / 100;
    let mut modified_width = available.saturating_sub(name_width + size_width);

    if size_width < 8 {
        let delta = 8 - size_width;
        size_width = 8;
        name_width = name_width.saturating_sub(delta);
    }
    if modified_width < 14 {
        let delta = 14 - modified_width;
        modified_width = 14;
        name_width = name_width.saturating_sub(delta);
    }
    name_width = name_width.max(8);

    TableLayout::Full {
        name_width,
        size_width,
        modified_width,
    }
}

fn render_table_header(layout: TableLayout) -> Line<'static> {
    let text = match layout {
        TableLayout::Full {
            name_width,
            size_width,
            modified_width,
        } => format!(
            "{:<name_width$} {:>size_width$} {:>modified_width$}",
            "Name",
            "Size",
            "Modified",
            name_width = name_width,
            size_width = size_width,
            modified_width = modified_width
        ),
        TableLayout::Compact {
            name_width,
            size_width,
        } => format!(
            "{:<name_width$} {:>size_width$}",
            "Name",
            "Size",
            name_width = name_width,
            size_width = size_width
        ),
        TableLayout::Minimal { name_width } => {
            format!("{:<name_width$}", "Name", name_width = name_width)
        }
    };

    Line::styled(
        text,
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
}

fn entry_name(entry: &FsEntry) -> String {
    if entry.is_virtual {
        return "..".to_string();
    }

    match entry.entry_type {
        FsEntryType::Directory => format!("{}/", entry.name),
        FsEntryType::Symlink => format!("{}@", entry.name),
        _ => entry.name.clone(),
    }
}

fn truncate_name(name: &str, width: usize) -> String {
    if width <= 1 {
        return "…".to_string();
    }
    let mut truncated = String::new();
    for c in name.chars().take(width - 1) {
        truncated.push(c);
    }
    truncated.push('…');
    truncated
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

fn search_suffix(panel: &PanelState) -> String {
    if panel.search_query.is_empty() {
        String::new()
    } else {
        format!("  /{}", panel.search_query)
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

fn format_modified_at(entry: &FsEntry) -> String {
    if entry.is_virtual {
        return "-".to_string();
    }
    let Some(ts) = entry.modified_at else {
        return "-".to_string();
    };

    let dt: DateTime<Local> = DateTime::<Local>::from(ts);
    dt.format("%Y-%m-%d %H:%M").to_string()
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(height),
            Constraint::Min(1),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}
