use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::model::{
    AppState, DialogButton, DialogButtonRole, DialogState, DialogTone, FsEntry, FsEntryType,
    PanelId, PanelState, SortMode,
};

pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

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

    render_status(frame, chunks[1], state);
    render_log(frame, chunks[2], state);
    render_footer(frame, chunks[3], state);

    if let Some(dialog) = &state.dialog {
        render_dialog(frame, dialog);
    }
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
                spans.push(Span::styled("|", base_style));
                spans.push(Span::styled(
                    format!(
                        "{:>size_width$}",
                        truncate_name(&size_text, size_width),
                        size_width = size_width
                    ),
                    base_style,
                ));
                spans.push(Span::styled("|", base_style));
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
                spans.push(Span::styled("|", base_style));
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

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let active_panel = match state.active_panel {
        PanelId::Left => &state.left_panel,
        PanelId::Right => &state.right_panel,
    };
    let mode = footer_mode(state, active_panel);

    let mut spans = vec![Span::styled(
        format!("[{}] ", footer_mode_label(mode)),
        Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
    )];

    for button in build_footer_buttons(state, active_panel, mode) {
        let style = if !button.enabled {
            Style::default().fg(Color::DarkGray)
        } else if button.active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        };
        spans.push(Span::styled(
            format!("[{} {}] ", button.key, button.label),
            style,
        ));
    }

    let footer = Paragraph::new(Line::from(spans));
    frame.render_widget(footer, area);
}

fn footer_mode(state: &AppState, panel: &PanelState) -> FooterMode {
    if state.dialog.is_some() {
        FooterMode::Dialog
    } else if !panel.selected_paths.is_empty() {
        FooterMode::Selection
    } else {
        FooterMode::Normal
    }
}

fn footer_mode_label(mode: FooterMode) -> &'static str {
    match mode {
        FooterMode::Normal => "NORMAL",
        FooterMode::Selection => "SELECTION",
        FooterMode::Dialog => "DIALOG",
    }
}

fn build_footer_buttons(
    state: &AppState,
    panel: &PanelState,
    mode: FooterMode,
) -> Vec<FooterButtonSpec> {
    match mode {
        FooterMode::Normal => {
            let current_entry_operable = panel
                .selected_entry()
                .is_some_and(|entry| !entry.is_virtual);
            vec![
                FooterButtonSpec::new("F1", "Help", false, false),
                FooterButtonSpec::new("F2", "Sort", true, false),
                FooterButtonSpec::new("F3", "View", false, false),
                FooterButtonSpec::new("F4", "Edit", false, false),
                FooterButtonSpec::new("F5", "Copy", current_entry_operable, false),
                FooterButtonSpec::new("F6", "Move", current_entry_operable, false),
                FooterButtonSpec::new("F7", "Mkdir", true, false),
                FooterButtonSpec::new("F8", "Delete", current_entry_operable, false),
                FooterButtonSpec::new("F9", "Menu", false, false),
                FooterButtonSpec::new("F10", "Quit", true, false),
            ]
        }
        FooterMode::Selection => vec![
            FooterButtonSpec::new("F1", "Help", false, false),
            FooterButtonSpec::new("F2", "Sort", true, false),
            FooterButtonSpec::new("F3", "View", false, false),
            FooterButtonSpec::new("F4", "Edit", false, false),
            FooterButtonSpec::new("F5", "Copy", true, true),
            FooterButtonSpec::new("F6", "Move", true, true),
            FooterButtonSpec::new("F7", "Mkdir", true, false),
            FooterButtonSpec::new("F8", "Delete", true, true),
            FooterButtonSpec::new("F9", "Menu", false, false),
            FooterButtonSpec::new("F10", "Quit", true, false),
        ],
        FooterMode::Dialog => {
            let mut buttons = vec![
                FooterButtonSpec::new("Tab", "Next", true, false),
                FooterButtonSpec::new("S-Tab", "Prev", true, false),
                FooterButtonSpec::new("Left/Right", "Focus", true, false),
            ];

            if let Some(dialog) = state.dialog.as_ref() {
                if let Some(primary) = dialog.buttons.first() {
                    let key = if let Some(acc) = primary.accelerator {
                        format!("Alt+{}", acc.to_ascii_uppercase())
                    } else {
                        "Enter".to_string()
                    };
                    buttons.push(FooterButtonSpec::new(
                        key.as_str(),
                        primary.label.as_str(),
                        true,
                        dialog.focused_button == 0,
                    ));
                }

                if let Some(secondary) = dialog.buttons.get(1) {
                    let key = if let Some(acc) = secondary.accelerator {
                        format!("Alt+{}", acc.to_ascii_uppercase())
                    } else {
                        "Esc".to_string()
                    };
                    buttons.push(FooterButtonSpec::new(
                        key.as_str(),
                        secondary.label.as_str(),
                        true,
                        dialog.focused_button == 1,
                    ));
                } else {
                    buttons.push(FooterButtonSpec::new("Esc", "Close", true, false));
                }
            }

            buttons
        }
    }
}

#[derive(Clone, Copy)]
enum FooterMode {
    Normal,
    Selection,
    Dialog,
}

struct FooterButtonSpec {
    key: String,
    label: String,
    enabled: bool,
    active: bool,
}

impl FooterButtonSpec {
    fn new(key: &str, label: &str, enabled: bool, active: bool) -> Self {
        Self {
            key: key.to_string(),
            label: label.to_string(),
            enabled,
            active,
        }
    }
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
            Constraint::Length(3),
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
        let label = input_label(dialog);
        let input_block = Block::default()
            .title(label)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let input = Paragraph::new(Line::styled(
            format!("{value}|"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .block(input_block);
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

fn input_label(dialog: &DialogState) -> &'static str {
    let title = dialog.title.to_ascii_lowercase();
    if title.contains("mask") {
        "Mask"
    } else {
        "Name"
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
            "{:<name_width$}|{:>size_width$}|{:>modified_width$}",
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
            "{:<name_width$}|{:>size_width$}",
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
    if width == 0 {
        return String::new();
    }

    let char_count = name.chars().count();
    if char_count <= width {
        return name.to_string();
    }

    if width <= 3 {
        return ".".repeat(width);
    }

    let mut truncated = String::new();
    for c in name.chars().take(width - 3) {
        truncated.push(c);
    }
    truncated.push_str("...");
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

#[cfg(test)]
mod tests {
    use super::{TableLayout, fixed_table_layout};

    #[test]
    fn fixed_table_layout_switches_modes_for_narrow_widths() {
        assert!(matches!(fixed_table_layout(8), TableLayout::Minimal { .. }));
        assert!(matches!(
            fixed_table_layout(20),
            TableLayout::Compact { .. }
        ));
        assert!(matches!(fixed_table_layout(80), TableLayout::Full { .. }));
    }

    #[test]
    fn fixed_table_layout_never_exceeds_panel_width() {
        for width in 1usize..=180 {
            let layout = fixed_table_layout(width);
            assert!(
                layout_total_width(layout) <= width,
                "layout width exceeds panel width: panel={width}, layout={}",
                layout_total_width(layout)
            );
        }
    }

    fn layout_total_width(layout: TableLayout) -> usize {
        match layout {
            TableLayout::Full {
                name_width,
                size_width,
                modified_width,
            } => name_width + size_width + modified_width + 2,
            TableLayout::Compact {
                name_width,
                size_width,
            } => name_width + size_width + 1,
            TableLayout::Minimal { name_width } => name_width,
        }
    }
}
