use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::menu::top_menu_groups;
use crate::model::{
    AppState, BatchProgressState, CommandLineState, DialogButtonRole, DialogState, DialogTone,
    FindProgressState, FsEntry, FsEntryType, JobKind, PanelId, PanelState, ScreenMode, SortMode,
    ViewerMode, ViewerState,
};
use crate::theme::{DirColorsTheme, ThemeColor, ThemeStyle};

const COL_SEP: &str = "│";

pub fn render(frame: &mut Frame, state: &AppState, theme: &DirColorsTheme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_top_menu(frame, chunks[0], state);

    if state.screen_mode == ScreenMode::Viewer {
        render_viewer(frame, chunks[1], state.viewer.as_ref());
    } else {
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
            theme,
        );
        render_panel(
            frame,
            panel_chunks[1],
            "Right",
            &state.right_panel,
            state.active_panel == PanelId::Right,
            theme,
        );
    }

    render_footer(frame, chunks[2], state);
    render_command_line(frame, chunks[3], &state.command_line);

    if state.top_menu.open {
        render_top_menu_popup(frame, chunks[0], chunks[1], state);
    }

    if let Some(progress) = state.batch_progress.as_ref() {
        render_batch_progress_overlay(frame, progress);
    }

    if let Some(dialog) = &state.dialog {
        render_dialog(frame, dialog);
    }
}

fn render_top_menu(frame: &mut Frame, area: Rect, state: &AppState) {
    let groups = top_menu_groups();
    if groups.is_empty() {
        return;
    }

    let mut cells = Vec::with_capacity(groups.len());
    for (idx, group) in groups.iter().enumerate() {
        cells.push(FooterCellSpec {
            text: format!("{}({})", group.label, group.hotkey.to_ascii_uppercase()),
            kind: FooterCellKind::Button,
            enabled: true,
            active: state.top_menu.open && idx == state.top_menu.group_index,
        });
    }

    let spans = build_top_menu_spans(&cells, area.width as usize);
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_top_menu_popup(frame: &mut Frame, menu_area: Rect, body_area: Rect, state: &AppState) {
    let groups = top_menu_groups();
    if groups.is_empty() || body_area.width < 10 || body_area.height < 4 {
        return;
    }

    let group_idx = state
        .top_menu
        .group_index
        .min(groups.len().saturating_sub(1));
    let group = groups[group_idx];
    if group.items.is_empty() {
        return;
    }

    let mut popup_width = group
        .items
        .iter()
        .map(|item| item.label.chars().count())
        .max()
        .unwrap_or(16)
        .saturating_add(6);
    popup_width = popup_width.max(20);
    popup_width = popup_width.min(body_area.width.saturating_sub(2) as usize);

    let popup_height = (group.items.len() as u16).saturating_add(2);
    let popup_height = popup_height.min(body_area.height);
    if popup_height < 3 || popup_width < 8 {
        return;
    }

    let menu_cells = groups.len();
    let cell_widths = distribute_width(menu_area.width as usize, menu_cells);
    let cell_index = group_idx;
    let offset: usize = cell_widths.iter().take(cell_index).sum();
    let mut popup_x = menu_area.x.saturating_add(offset as u16);
    let max_x = body_area
        .x
        .saturating_add(body_area.width.saturating_sub(popup_width as u16));
    if popup_x > max_x {
        popup_x = max_x;
    }
    if popup_x < body_area.x {
        popup_x = body_area.x;
    }

    let popup_area = Rect {
        x: popup_x,
        y: body_area.y,
        width: popup_width as u16,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .title(format!("{} Menu", group.label))
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines = Vec::with_capacity(inner.height as usize);
    let selected = state
        .top_menu
        .item_index
        .min(group.items.len().saturating_sub(1));
    for (idx, item) in group.items.iter().enumerate() {
        let style = if !item.is_selectable() {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else if idx == selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let text = fit_footer_cell_text(item.label, inner.width as usize);
        lines.push(Line::styled(text, style));
        if lines.len() >= inner.height as usize {
            break;
        }
    }
    while lines.len() < inner.height as usize {
        lines.push(Line::from(String::new()));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_command_line(frame: &mut Frame, area: Rect, command_line: &CommandLineState) {
    let prompt = ":";
    let input = if command_line.active {
        format!("{}{}|", prompt, command_line.input)
    } else if command_line.input.is_empty() {
        ": (press : to command)".to_string()
    } else {
        format!("{prompt}{}", command_line.input)
    };

    let style = if command_line.active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::LightGreen)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray).bg(Color::Black)
    };

    let text = fit_footer_cell_text(input.as_str(), area.width as usize);
    frame.render_widget(Paragraph::new(Line::styled(text, style)), area);
}

fn render_viewer(frame: &mut Frame, area: Rect, viewer: Option<&ViewerState>) {
    let block = Block::default()
        .title(viewer_title(viewer))
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let lines = build_viewer_lines(viewer, inner);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_panel(
    frame: &mut Frame,
    area: Rect,
    name: &str,
    panel: &PanelState,
    active: bool,
    theme: &DirColorsTheme,
) {
    let border_style = if active {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = format!(
        "{name} [{}] <{}> {}{}{}",
        sort_label(panel.sort_mode),
        panel.backend_label,
        panel.cwd.display(),
        find_suffix(panel),
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
    let lines = build_entry_lines(panel, active, inner, theme);
    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);
}

fn build_entry_lines(
    panel: &PanelState,
    panel_active: bool,
    inner: Rect,
    theme: &DirColorsTheme,
) -> Vec<Line<'static>> {
    let capacity = inner.height as usize;
    let total_width = inner.width as usize;
    if capacity <= 1 || total_width == 0 {
        return Vec::new();
    }

    let layout = fixed_table_layout(total_width);
    let (selected_count, selected_bytes) = panel.selection_summary();
    let has_selection_badge = panel_active && selected_count > 0;
    let rows_capacity = if has_selection_badge {
        capacity.saturating_sub(2)
    } else {
        capacity.saturating_sub(1)
    };
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
                    base_style.patch(type_style(entry, theme)),
                ));
                spans.push(Span::styled(COL_SEP, base_style));
                spans.push(Span::styled(
                    format!(
                        "{:>size_width$}",
                        truncate_name(&size_text, size_width),
                        size_width = size_width
                    ),
                    base_style,
                ));
                spans.push(Span::styled(COL_SEP, base_style));
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
                    base_style.patch(type_style(entry, theme)),
                ));
                spans.push(Span::styled(COL_SEP, base_style));
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
                    base_style.patch(type_style(entry, theme)),
                ));
            }
        }
        lines.push(Line::from(spans));
    }

    while lines.len() < rows_capacity + 1 {
        lines.push(render_table_empty_line(layout));
    }

    if has_selection_badge {
        lines.push(render_selection_line(
            layout,
            selected_count,
            selected_bytes,
        ));
    }

    lines
}

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let active_panel = match state.active_panel {
        PanelId::Left => &state.left_panel,
        PanelId::Right => &state.right_panel,
    };
    let mode = footer_mode(state, active_panel);
    let buttons = build_footer_buttons(state, active_panel, mode);
    let mut cells = Vec::with_capacity(buttons.len() + 1);
    cells.push(FooterCellSpec {
        text: footer_mode_label(mode).to_string(),
        kind: FooterCellKind::Mode,
        enabled: true,
        active: false,
    });
    for button in buttons {
        cells.push(FooterCellSpec {
            text: footer_button_label(button.key.as_str(), button.label.as_str()),
            kind: FooterCellKind::Button,
            enabled: button.enabled,
            active: button.active,
        });
    }
    if mode == FooterMode::Viewer {
        cells.push(FooterCellSpec {
            text: viewer_status_label(state.viewer.as_ref()),
            kind: FooterCellKind::Status,
            enabled: true,
            active: false,
        });
    } else if let Some(progress) = state.find_progress.as_ref() {
        cells.push(FooterCellSpec {
            text: find_status_label(progress),
            kind: FooterCellKind::Status,
            enabled: true,
            active: false,
        });
    }

    let spans = build_footer_spans(&cells, area.width as usize);
    let footer = Paragraph::new(Line::from(spans));
    frame.render_widget(footer, area);
}

fn footer_mode(state: &AppState, panel: &PanelState) -> FooterMode {
    if state.screen_mode == ScreenMode::Viewer {
        FooterMode::Viewer
    } else if state.dialog.is_some() {
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
        FooterMode::Viewer => "VIEWER",
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
            let current_entry_viewable = panel.selected_entry().is_some_and(|entry| {
                !entry.is_virtual && entry.entry_type != FsEntryType::Directory
            });
            let current_entry_editable = current_entry_viewable;
            vec![
                FooterButtonSpec::new("F1", "Help", false, false),
                FooterButtonSpec::new("F2", "Sort", true, false),
                FooterButtonSpec::new("F3", "View", current_entry_viewable, false),
                FooterButtonSpec::new("F4", "Edit", current_entry_editable, false),
                FooterButtonSpec::new("F5", "Copy", current_entry_operable, false),
                FooterButtonSpec::new("F6", "Move", current_entry_operable, false),
                FooterButtonSpec::new("F7", "Mkdir", true, false),
                FooterButtonSpec::new("F8", "Delete", current_entry_operable, false),
                FooterButtonSpec::new("F9", "Menu", true, false),
                FooterButtonSpec::new("F10", "Quit", true, false),
            ]
        }
        FooterMode::Selection => {
            let current_entry_viewable = panel.selected_entry().is_some_and(|entry| {
                !entry.is_virtual && entry.entry_type != FsEntryType::Directory
            });
            let current_entry_editable = current_entry_viewable;
            vec![
                FooterButtonSpec::new("F1", "Help", false, false),
                FooterButtonSpec::new("F2", "Sort", true, false),
                FooterButtonSpec::new("F3", "View", current_entry_viewable, false),
                FooterButtonSpec::new("F4", "Edit", current_entry_editable, false),
                FooterButtonSpec::new("F5", "Copy", true, true),
                FooterButtonSpec::new("F6", "Move", true, true),
                FooterButtonSpec::new("F7", "Mkdir", true, false),
                FooterButtonSpec::new("F8", "Delete", true, true),
                FooterButtonSpec::new("F9", "Menu", true, false),
                FooterButtonSpec::new("F10", "Quit", true, false),
            ]
        }
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
        FooterMode::Viewer => vec![
            FooterButtonSpec::new("F2", "Mode", true, false),
            FooterButtonSpec::new("F3", "Close", true, false),
            FooterButtonSpec::new("Esc", "Close", true, false),
            FooterButtonSpec::new("/", "Find", true, false),
            FooterButtonSpec::new("n", "Next", true, false),
            FooterButtonSpec::new("N", "Prev", true, false),
            FooterButtonSpec::new("Up", "Scroll", true, false),
            FooterButtonSpec::new("Down", "Scroll", true, false),
            FooterButtonSpec::new("PgUp", "PageUp", true, false),
            FooterButtonSpec::new("PgDn", "PageDn", true, false),
            FooterButtonSpec::new("Home", "Top", true, false),
            FooterButtonSpec::new("End", "Bottom", true, false),
        ],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FooterMode {
    Normal,
    Selection,
    Dialog,
    Viewer,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum FooterCellKind {
    Mode,
    Button,
    Status,
}

struct FooterCellSpec {
    text: String,
    kind: FooterCellKind,
    enabled: bool,
    active: bool,
}

fn build_footer_spans(cells: &[FooterCellSpec], total_width: usize) -> Vec<Span<'static>> {
    if cells.is_empty() || total_width == 0 {
        return Vec::new();
    }

    let widths = distribute_width(total_width, cells.len());
    let mut spans = Vec::with_capacity(cells.len());
    for (cell, width) in cells.iter().zip(widths.into_iter()) {
        if width == 0 {
            continue;
        }
        let text = fit_footer_cell_text(cell.text.as_str(), width);
        spans.push(Span::styled(text, footer_cell_style(cell)));
    }
    spans
}

fn build_top_menu_spans(cells: &[FooterCellSpec], total_width: usize) -> Vec<Span<'static>> {
    if cells.is_empty() || total_width == 0 {
        return Vec::new();
    }

    let widths = distribute_width(total_width, cells.len());
    let mut spans = Vec::with_capacity(cells.len());
    for (cell, width) in cells.iter().zip(widths.into_iter()) {
        if width == 0 {
            continue;
        }
        let text = fit_footer_cell_text(cell.text.as_str(), width);
        spans.push(Span::styled(text, top_menu_cell_style(cell)));
    }
    spans
}

fn footer_button_label(key: &str, label: &str) -> String {
    format!("{key} {}", abbreviate_footer_label(label))
}

fn abbreviate_footer_label(label: &str) -> String {
    match label {
        "Selection" => "Select".to_string(),
        "PageUp" => "PgUp".to_string(),
        "PageDn" => "PgDn".to_string(),
        "Left/Right" => "L/R".to_string(),
        "S-Tab" => "ShiftTab".to_string(),
        _ => label.to_string(),
    }
}

fn fit_footer_cell_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut value = truncate_name(text, width);
    let len = value.chars().count();
    if len >= width {
        return value;
    }
    let right_padding = width - len;
    value.push_str(&" ".repeat(right_padding));
    value
}

fn distribute_width(total_width: usize, cells: usize) -> Vec<usize> {
    if cells == 0 {
        return Vec::new();
    }
    let base = total_width / cells;
    let rem = total_width % cells;
    let mut widths = Vec::with_capacity(cells);
    for idx in 0..cells {
        widths.push(base + usize::from(idx < rem));
    }
    widths
}

fn footer_cell_style(cell: &FooterCellSpec) -> Style {
    match cell.kind {
        FooterCellKind::Mode => Style::default()
            .fg(Color::Black)
            .bg(Color::White)
            .add_modifier(Modifier::BOLD),
        FooterCellKind::Status => Style::default()
            .fg(Color::Cyan)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        FooterCellKind::Button => {
            if !cell.enabled {
                Style::default().fg(Color::DarkGray).bg(Color::Blue)
            } else if cell.active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            }
        }
    }
}

fn top_menu_cell_style(cell: &FooterCellSpec) -> Style {
    if cell.active {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD)
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
        let visible_value = if dialog.mask_input {
            "*".repeat(value.chars().count())
        } else {
            value.clone()
        };
        let input_block = Block::default()
            .title(label)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let input = Paragraph::new(Line::styled(
            format!("{visible_value}|"),
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

fn render_batch_progress_overlay(frame: &mut Frame, progress: &BatchProgressState) {
    let area = centered_rect(62, 8, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Progress #{}", progress.batch_id))
        .border_style(
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let completed = progress.completed.min(progress.total);
    let total = progress.total.max(1);
    let percent = (completed * 100) / total;
    let bar_width = inner.width.saturating_sub(8) as usize;
    let filled = (bar_width * completed) / total;
    let progress_bar = format!(
        "[{}{}] {:>3}%",
        "=".repeat(filled),
        "-".repeat(bar_width.saturating_sub(filled)),
        percent
    );
    let current_file = truncate_name(progress.current_file.as_str(), inner.width as usize);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "Operation: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(operation_label(progress.operation)),
        ]),
        Line::from(vec![
            Span::styled(
                "Current file: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(current_file),
        ]),
        Line::from(vec![
            Span::styled(
                "Files: ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{}/{}  failed: {}",
                completed, progress.total, progress.failed
            )),
        ]),
        Line::styled(progress_bar, Style::default().fg(Color::Green)),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_button_row(dialog: &DialogState) -> Line<'static> {
    let mut spans = Vec::new();

    for (idx, button) in dialog.buttons.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::raw(" "));
        }

        let is_focused = idx == dialog.focused_button;
        let label = button.label.as_str();
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

fn input_label(dialog: &DialogState) -> &'static str {
    let title = dialog.title.to_ascii_lowercase();
    if title.contains("mask") {
        "Mask"
    } else if title.contains("sftp connect") {
        "Address"
    } else if title.contains("sftp") && title.contains("address") {
        "Address"
    } else if title.contains("sftp") && title.contains("login") {
        "Login"
    } else if title.contains("sftp") && title.contains("password") {
        "Password"
    } else if title.contains("sftp") && title.contains("key") {
        "Key Path"
    } else if title.contains("editor") {
        "Choice"
    } else {
        "Name"
    }
}

fn type_style(entry: &FsEntry, theme: &DirColorsTheme) -> Style {
    theme_style_to_ratatui(theme.style_for_entry(entry))
}

fn theme_style_to_ratatui(style: ThemeStyle) -> Style {
    let mut ratatui_style = Style::default();
    if let Some(color) = style.fg {
        ratatui_style = ratatui_style.fg(theme_color_to_ratatui(color));
    }
    if style.bold {
        ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
    }
    ratatui_style
}

fn theme_color_to_ratatui(color: ThemeColor) -> Color {
    match color {
        ThemeColor::Black => Color::Black,
        ThemeColor::Red => Color::Red,
        ThemeColor::Green => Color::Green,
        ThemeColor::Yellow => Color::Yellow,
        ThemeColor::Blue => Color::Blue,
        ThemeColor::Magenta => Color::Magenta,
        ThemeColor::Cyan => Color::Cyan,
        ThemeColor::White => Color::White,
        ThemeColor::BrightBlack => Color::DarkGray,
        ThemeColor::BrightRed => Color::LightRed,
        ThemeColor::BrightGreen => Color::LightGreen,
        ThemeColor::BrightYellow => Color::LightYellow,
        ThemeColor::BrightBlue => Color::LightBlue,
        ThemeColor::BrightMagenta => Color::LightMagenta,
        ThemeColor::BrightCyan => Color::LightCyan,
        ThemeColor::BrightWhite => Color::Gray,
        ThemeColor::Indexed(value) => Color::Indexed(value),
        ThemeColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
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
            "{:<name_width$}{COL_SEP}{:>size_width$}{COL_SEP}{:>modified_width$}",
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
            "{:<name_width$}{COL_SEP}{:>size_width$}",
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

fn render_table_empty_line(layout: TableLayout) -> Line<'static> {
    let text = match layout {
        TableLayout::Full {
            name_width,
            size_width,
            modified_width,
        } => format!(
            "{:<name_width$}{COL_SEP}{:>size_width$}{COL_SEP}{:>modified_width$}",
            "",
            "",
            "",
            name_width = name_width,
            size_width = size_width,
            modified_width = modified_width
        ),
        TableLayout::Compact {
            name_width,
            size_width,
        } => format!(
            "{:<name_width$}{COL_SEP}{:>size_width$}",
            "",
            "",
            name_width = name_width,
            size_width = size_width
        ),
        TableLayout::Minimal { name_width } => {
            format!("{:<name_width$}", "", name_width = name_width)
        }
    };

    Line::styled(text, Style::default().fg(Color::DarkGray))
}

fn render_selection_line(
    layout: TableLayout,
    selected_count: usize,
    selected_bytes: u64,
) -> Line<'static> {
    let badge = format!("sel:{selected_count} ({})", human_size(selected_bytes));

    let text = match layout {
        TableLayout::Full {
            name_width,
            size_width,
            modified_width,
        } => format!(
            "{:<name_width$}{COL_SEP}{:>size_width$}{COL_SEP}{:>modified_width$}",
            truncate_name(badge.as_str(), name_width),
            "",
            "",
            name_width = name_width,
            size_width = size_width,
            modified_width = modified_width
        ),
        TableLayout::Compact {
            name_width,
            size_width,
        } => format!(
            "{:<name_width$}{COL_SEP}{:>size_width$}",
            truncate_name(badge.as_str(), name_width),
            "",
            name_width = name_width,
            size_width = size_width
        ),
        TableLayout::Minimal { name_width } => format!(
            "{:<name_width$}",
            truncate_name(badge.as_str(), name_width),
            name_width = name_width
        ),
    };

    Line::styled(
        text,
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}

fn entry_name(entry: &FsEntry) -> String {
    if entry.is_virtual {
        return entry.name.clone();
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

fn find_suffix(panel: &PanelState) -> String {
    let Some(find) = panel.find_view.as_ref() else {
        return String::new();
    };

    let mut flags = Vec::new();
    if find.glob {
        flags.push("glob");
    }
    if find.hidden {
        flags.push("hidden");
    }
    if find.follow_symlinks {
        flags.push("follow");
    }
    if flags.is_empty() {
        format!(" [fd:{}]", find.query)
    } else {
        format!(" [fd:{}:{}]", find.query, flags.join(","))
    }
}

fn find_status_label(progress: &FindProgressState) -> String {
    let panel = match progress.panel_id {
        PanelId::Left => "L",
        PanelId::Right => "R",
    };
    if progress.running {
        format!("FIND {panel} '{}' {}...", progress.query, progress.matches)
    } else {
        format!(
            "FIND {panel} '{}' {} done",
            progress.query, progress.matches
        )
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

fn viewer_title(viewer: Option<&ViewerState>) -> String {
    match viewer {
        Some(state) => format!(
            "Viewer [{}|{}{}] {}",
            state.title,
            match state.mode {
                ViewerMode::Text => "text",
                ViewerMode::Hex => "hex",
            },
            if state.preview_truncated {
                ",trunc"
            } else {
                ""
            },
            state.path.display()
        ),
        None => "Viewer".to_string(),
    }
}

fn viewer_status_label(viewer: Option<&ViewerState>) -> String {
    match viewer {
        Some(state) => {
            let total_lines = state.lines.len();
            let current_offset = state.scroll_offset.min(total_lines.saturating_sub(1));
            let mode = match state.mode {
                ViewerMode::Text => "text",
                ViewerMode::Hex => "hex",
            };
            let match_status = if state.search_query.is_empty() {
                "m:0/0".to_string()
            } else {
                let total = state.search_matches.len();
                let current = if total == 0 {
                    0
                } else {
                    state.search_match_index + 1
                };
                format!("m:{current}/{total}")
            };
            format!(
                "off:{current_offset} lines:{total_lines} bytes:{} mode:{mode} {match_status}",
                human_size(state.byte_size),
            )
        }
        None => "off:0 lines:0 bytes:0B mode:text".to_string(),
    }
}

fn operation_label(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Copy => "Copy",
        JobKind::Move => "Move",
        JobKind::Delete => "Delete",
        JobKind::Mkdir => "Mkdir",
    }
}

fn build_viewer_lines(viewer: Option<&ViewerState>, area: Rect) -> Vec<Line<'static>> {
    let capacity = area.height as usize;
    if capacity == 0 {
        return Vec::new();
    }

    let Some(viewer) = viewer else {
        return vec![Line::styled(
            "Viewer state is unavailable.",
            Style::default().fg(Color::Red),
        )];
    };

    if viewer.lines.is_empty() {
        return vec![Line::styled(
            "File is empty.",
            Style::default().fg(Color::DarkGray),
        )];
    }

    let start = viewer
        .scroll_offset
        .min(viewer.lines.len().saturating_sub(1));
    let end = (start + capacity).min(viewer.lines.len());
    let mut lines = Vec::with_capacity(capacity);
    let active_match_line = viewer
        .search_matches
        .get(viewer.search_match_index)
        .copied();
    for (offset, line) in viewer.lines[start..end].iter().enumerate() {
        let absolute = start + offset;
        let is_match = viewer.search_matches.contains(&absolute);
        let is_active_match = active_match_line == Some(absolute);
        let style = if is_active_match {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if is_match {
            Style::default()
                .fg(Color::Black)
                .bg(Color::LightYellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::styled(line.clone(), style));
    }
    while lines.len() < capacity {
        lines.push(Line::from(String::new()));
    }

    lines
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
    use super::{TableLayout, distribute_width, fit_footer_cell_text, fixed_table_layout};

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

    #[test]
    fn footer_width_distribution_always_matches_target() {
        for width in 1usize..=220 {
            for cells in 1usize..=16 {
                let widths = distribute_width(width, cells);
                assert_eq!(widths.iter().sum::<usize>(), width);
            }
        }
    }

    #[test]
    fn footer_text_fits_cell_width() {
        for width in 1usize..=24 {
            let text = fit_footer_cell_text("F10 VeryLongLabel", width);
            assert_eq!(text.chars().count(), width);
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
