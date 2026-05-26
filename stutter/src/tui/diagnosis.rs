use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
};

use super::model::TuiDiagnosisLine;

pub(super) fn render_diagnoses(f: &mut Frame, diagnoses: &[TuiDiagnosisLine], area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Recent stutter diagnoses ");
    let paragraph = Paragraph::new(
        diagnoses
            .iter()
            .map(TuiDiagnosisLine::to_ratatui_line)
            .collect::<Vec<_>>(),
    )
    .block(block);
    f.render_widget(paragraph, area);
}
