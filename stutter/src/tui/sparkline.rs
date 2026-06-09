use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Sparkline},
};

pub(super) fn render_sparkline(f: &mut Frame, sparkline_ms: &[u64], area: Rect) {
    let mut data = sparkline_ms.to_vec();
    let max_len = area.width.saturating_sub(2) as usize;
    if data.len() > max_len {
        let start = data.len() - max_len;
        data = data[start..].to_vec();
    }

    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .title(" Global Max Latency (ms) ")
                .borders(Borders::ALL),
        )
        .data(&data)
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(sparkline, area);
}
