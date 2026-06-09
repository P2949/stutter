use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{BarChart, Block, Borders},
};

use super::model::TuiCpuHeatBar;

pub(super) fn render_cpu_heat(f: &mut Frame, cpu_heat: &[TuiCpuHeatBar], area: Rect) {
    let bar_data: Vec<(&str, u64)> = cpu_heat
        .iter()
        .map(|bar| (bar.label.as_str(), bar.max_latency_ms))
        .collect();

    let barchart = BarChart::default()
        .block(
            Block::default()
                .title(" Max Latency per CPU (ms) ")
                .borders(Borders::ALL),
        )
        .data(&bar_data)
        .bar_width(3)
        .bar_gap(1)
        .bar_style(Style::default().fg(Color::Red))
        .value_style(Style::default().fg(Color::White).bg(Color::Red));

    f.render_widget(barchart, area);
}
