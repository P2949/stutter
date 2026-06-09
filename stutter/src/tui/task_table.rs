use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table},
};

use super::model::TuiTaskRow;

pub(super) fn render_task_table(f: &mut Frame, task_rows: &[TuiTaskRow], area: Rect) {
    let rows: Vec<Row> = task_rows
        .iter()
        .map(|row| {
            Row::new(vec![
                Cell::from(row.tid.clone()),
                Cell::from(row.comm.clone()),
                Cell::from(row.class.clone()),
                Cell::from(row.samples.clone()),
                Cell::from(row.max_latency.clone()).style(row.max_latency_severity.style()),
                Cell::from(row.avg_latency.clone()),
                Cell::from(row.over_1ms.clone()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Percentage(25),
        Constraint::Length(14),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
    ];

    let header = Row::new(vec![
        "TID", "Comm", "Class", "Samples", "Max", "Avg", ">1ms",
    ])
    .style(
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(Color::Cyan),
    );

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Tasks "))
        .column_spacing(1);

    f.render_widget(table, area);
}
