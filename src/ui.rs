use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::model::{AppState, PanelId};

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

    let header = Paragraph::new(Line::from(vec![
        Span::raw("VCMC  "),
        Span::raw("Tab: switch panel  "),
        Span::raw("q: quit"),
    ]));
    frame.render_widget(header, chunks[0]);

    let panel_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    let left_title = panel_title("Left", state.active_panel == PanelId::Left);
    let right_title = panel_title("Right", state.active_panel == PanelId::Right);

    let left = Paragraph::new(format!(
        "cwd: {}\nentries: {}\nsort: {:?}",
        state.left_panel.cwd.display(),
        state.left_panel.entries.len(),
        state.left_panel.sort_mode
    ))
    .block(Block::default().title(left_title).borders(Borders::ALL));
    frame.render_widget(left, panel_chunks[0]);

    let right = Paragraph::new(format!(
        "cwd: {}\nentries: {}\nsort: {:?}",
        state.right_panel.cwd.display(),
        state.right_panel.entries.len(),
        state.right_panel.sort_mode
    ))
    .block(Block::default().title(right_title).borders(Borders::ALL));
    frame.render_widget(right, panel_chunks[1]);

    let status = Paragraph::new(state.status_line.clone());
    frame.render_widget(status, chunks[2]);

    let help = Paragraph::new("F5 Copy  F6 Move  F7 Mkdir  F8 Delete  Backspace Parent");
    frame.render_widget(help, chunks[3]);
}

fn panel_title(name: &str, active: bool) -> String {
    if active {
        format!("{name} *")
    } else {
        name.to_string()
    }
}
