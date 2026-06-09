use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
};

use super::model::TuiModel;

pub(super) fn render_status_bar(f: &mut Frame, area: Rect, model: &TuiModel) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" stutter profiler ");
    let paragraph = Paragraph::new(
        model
            .status_lines
            .iter()
            .map(|line| line.to_ratatui_line())
            .collect::<Vec<_>>(),
    )
    .block(block);
    f.render_widget(paragraph, area);
}
