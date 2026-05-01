use std::{
    collections::BTreeMap,
    io::{self, Stdout},
};

use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{BarChart, Block, Borders, Cell, Paragraph, Row, Sparkline, Table},
};

use crate::{
    ebpf_loader::DropCountersSnapshot,
    metrics::{IntervalRecord, TaskStats, format_latency},
    process_tree::{TaskClass, TaskInfo},
};

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortField {
    MaxLatency,
    AvgLatency,
    Samples,
}

impl SortField {
    pub fn next(self) -> Self {
        match self {
            Self::MaxLatency => Self::AvgLatency,
            Self::AvgLatency => Self::Samples,
            Self::Samples => Self::MaxLatency,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::MaxLatency => "Max Latency",
            Self::AvgLatency => "Avg Latency",
            Self::Samples => "Samples",
        }
    }
}

pub struct TuiState {
    pub sort_field: SortField,
    pub filter_class: Option<TaskClass>,
    pub paused: bool,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            sort_field: SortField::MaxLatency,
            filter_class: None,
            paused: false,
        }
    }
}

impl TuiState {
    /// Cycle through `None -> Unknown -> Game -> … -> None`.
    pub fn next_filter_class(&mut self) {
        const CLASSES: &[TaskClass] = &[
            TaskClass::Unknown,
            TaskClass::Game,
            TaskClass::GameHelper,
            TaskClass::Launcher,
            TaskClass::WineServer,
            TaskClass::GameScope,
            TaskClass::Compositor,
            TaskClass::SteamRuntime,
            TaskClass::Helper,
        ];
        if let Some(current) = self.filter_class {
            let pos = CLASSES.iter().position(|c| *c == current).unwrap_or(0);
            if pos + 1 < CLASSES.len() {
                self.filter_class = Some(CLASSES[pos + 1]);
            } else {
                self.filter_class = None; // wrap around
            }
        } else {
            self.filter_class = Some(CLASSES[0]);
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal lifecycle
// ---------------------------------------------------------------------------

pub fn init_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

pub fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Render entry point
// ---------------------------------------------------------------------------

pub fn render_tui(
    f: &mut Frame,
    state: &TuiState,
    active_targets: &BTreeMap<u32, TaskInfo>,
    stats_by_task: &BTreeMap<u32, TaskStats>,
    interval_records: &[IntervalRecord],
    elapsed_ms: u128,
    drop_counters: &DropCountersSnapshot,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // status bar
            Constraint::Min(10),   // task table
            Constraint::Length(8), // bottom panels
        ])
        .split(f.area());

    render_status_bar(
        f,
        state,
        active_targets,
        stats_by_task,
        elapsed_ms,
        drop_counters,
        chunks[0],
    );

    render_task_table(f, state, stats_by_task, chunks[1]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    render_sparkline(f, interval_records, bottom[0]);
    render_cpu_heat(f, stats_by_task, bottom[1]);
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

fn render_status_bar(
    f: &mut Frame,
    state: &TuiState,
    active_targets: &BTreeMap<u32, TaskInfo>,
    stats_by_task: &BTreeMap<u32, TaskStats>,
    elapsed_ms: u128,
    drop_counters: &DropCountersSnapshot,
    area: Rect,
) {
    let secs = elapsed_ms / 1000;
    let mins = secs / 60;
    let remaining = secs % 60;

    let mut parts = vec![
        Span::raw(format!(" Elapsed: {mins}m{remaining:02}s │ ")),
        Span::raw(format!(
            "Active: {}/{} │ ",
            active_targets.len(),
            stats_by_task.len()
        )),
    ];

    let drops = drop_counters.total();
    if drops > 0 {
        parts.push(Span::styled(
            format!("Drops: {drops} │ "),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    } else {
        parts.push(Span::styled(
            "No Drops │ ",
            Style::default().fg(Color::Green),
        ));
    }

    let filter_text = state
        .filter_class
        .map(|c| format!("{c:?}"))
        .unwrap_or_else(|| "All".to_owned());
    parts.push(Span::raw(format!(
        "[f]Filter: {filter_text} │ [s]Sort: {} │ ",
        state.sort_field.label()
    )));

    if state.paused {
        parts.push(Span::styled(
            "PAUSED",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        parts.push(Span::raw("Running"));
    }
    parts.push(Span::raw(" │ [q]Quit"));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" stutter profiler ");
    let paragraph = Paragraph::new(Line::from(parts)).block(block);
    f.render_widget(paragraph, area);
}

// ---------------------------------------------------------------------------
// Task table
// ---------------------------------------------------------------------------

fn render_task_table(
    f: &mut Frame,
    state: &TuiState,
    stats_by_task: &BTreeMap<u32, TaskStats>,
    area: Rect,
) {
    let mut tasks: Vec<&TaskStats> = stats_by_task.values().collect();

    if let Some(filter) = state.filter_class {
        tasks.retain(|t| t.class == filter);
    }

    tasks.sort_by(|a, b| match state.sort_field {
        SortField::MaxLatency => b.session_latency.max_ns.cmp(&a.session_latency.max_ns),
        SortField::AvgLatency => {
            let avg_a = avg_ns(&a.session_latency);
            let avg_b = avg_ns(&b.session_latency);
            avg_b.cmp(&avg_a)
        }
        SortField::Samples => b.session_latency.count.cmp(&a.session_latency.count),
    });

    let rows: Vec<Row> = tasks
        .iter()
        .map(|t| {
            let max_color = if t.session_latency.max_ns > 5_000_000 {
                Color::Red
            } else if t.session_latency.max_ns > 2_000_000 {
                Color::Yellow
            } else {
                Color::White
            };

            Row::new(vec![
                Cell::from(t.task.to_string()),
                Cell::from(t.comm.clone()),
                Cell::from(format!("{:?}", t.class)),
                Cell::from(t.session_latency.count.to_string()),
                Cell::from(format_latency(t.session_latency.max_ns))
                    .style(Style::default().fg(max_color)),
                Cell::from(format_latency(avg_ns(&t.session_latency))),
                Cell::from(t.session_latency.over_1ms.to_string()),
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

/// Compute average latency from session stats (avoiding divide-by-zero).
fn avg_ns(stats: &crate::metrics::LatencyStats) -> u64 {
    if stats.count == 0 {
        0
    } else {
        (stats.sum_ns / u128::from(stats.count)) as u64
    }
}

// ---------------------------------------------------------------------------
// Sparkline — global max latency per interval
// ---------------------------------------------------------------------------

fn render_sparkline(f: &mut Frame, interval_records: &[IntervalRecord], area: Rect) {
    // Each interval can contain multiple IntervalRecords (one per task).
    // Group by `elapsed_ms` and take the max `max_ns` across all tasks
    // within each interval tick.
    let mut by_tick: BTreeMap<u128, u64> = BTreeMap::new();
    for r in interval_records {
        let entry = by_tick.entry(r.elapsed_ms).or_insert(0);
        *entry = (*entry).max(r.max_ns);
    }

    let mut data: Vec<u64> = by_tick.values().map(|ns| ns / 1_000_000).collect();

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

// ---------------------------------------------------------------------------
// Per-CPU heat bar
// ---------------------------------------------------------------------------

fn render_cpu_heat(f: &mut Frame, stats_by_task: &BTreeMap<u32, TaskStats>, area: Rect) {
    // Aggregate the session-level max latency per CPU across all tasks.
    let mut cpu_max: BTreeMap<u32, u64> = BTreeMap::new();
    for task in stats_by_task.values() {
        for (&cpu, stats) in &task.session_cpu.by_cpu {
            let current = cpu_max.entry(cpu).or_insert(0);
            *current = (*current).max(stats.max_ns);
        }
    }

    let mut cpus: Vec<u32> = cpu_max.keys().copied().collect();
    cpus.sort();

    // BarChart requires &[(&str, u64)] with string refs that outlive the call.
    let labels: Vec<String> = cpus.iter().map(|c| c.to_string()).collect();
    let bar_data: Vec<(&str, u64)> = cpus
        .iter()
        .enumerate()
        .map(|(i, cpu)| (labels[i].as_str(), cpu_max[cpu] / 1_000_000))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_field_cycles() {
        let mut s = SortField::MaxLatency;
        s = s.next();
        assert_eq!(s, SortField::AvgLatency);
        s = s.next();
        assert_eq!(s, SortField::Samples);
        s = s.next();
        assert_eq!(s, SortField::MaxLatency);
    }

    #[test]
    fn filter_class_cycles_through_all_and_wraps() {
        let mut state = TuiState::default();
        assert_eq!(state.filter_class, None);
        state.next_filter_class();
        assert_eq!(state.filter_class, Some(TaskClass::Unknown));
        // Cycle through remaining 8 variants + 1 wrap = 9 calls
        for _ in 0..9 {
            state.next_filter_class();
        }
        // Should wrap back to None
        assert_eq!(state.filter_class, None);
    }
}
