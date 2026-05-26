use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::model::TuiAutotunePanel;

pub(super) fn render_autotune_panel(f: &mut Frame, panel: &TuiAutotunePanel, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Autotune ");
    let paragraph = Paragraph::new(
        panel
            .lines
            .iter()
            .map(|line| line.to_ratatui_line())
            .collect::<Vec<_>>(),
    )
    .block(block)
    .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}
