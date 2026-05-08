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
    widgets::{BarChart, Block, Borders, Cell, Paragraph, Row, Sparkline, Table, Wrap},
};

use crate::{
    autotune::tui_panel::{AutotuneTuiPanelSnapshot, load_default_autotune_tui_panel_snapshot},
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::ResolvedFocus,
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

#[derive(Clone)]
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
            TaskClass::GameRenderThread,
            TaskClass::GameWorkerThread,
            TaskClass::GameHelper,
            TaskClass::Launcher,
            TaskClass::WineServer,
            TaskClass::GameScope,
            TaskClass::Compositor,
            TaskClass::AudioRealtime,
            TaskClass::BrowserForeground,
            TaskClass::SteamRuntime,
            TaskClass::Helper,
            TaskClass::Service,
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

#[allow(clippy::too_many_arguments)]
pub fn render_tui(
    f: &mut Frame,
    state: &TuiState,
    active_targets: &BTreeMap<u32, TaskInfo>,
    stats_by_task: &BTreeMap<u32, TaskStats>,
    interval_records: &[IntervalRecord],
    recent_diagnoses: &std::collections::VecDeque<LiveDiagnosisEntry>,
    elapsed_ms: u128,
    drop_counters: &DropCountersSnapshot,
    current_focus: Option<&ResolvedFocus>,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // status bar + focus line
            Constraint::Min(10),   // task table
            Constraint::Length(8), // sparkline / heat
            Constraint::Length(8), // recent diagnoses
        ])
        .split(f.area());

    render_status_bar(
        f,
        state,
        active_targets,
        stats_by_task,
        elapsed_ms,
        drop_counters,
        current_focus,
        chunks[0],
    );

    render_task_table(f, state, stats_by_task, chunks[1]);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[2]);

    render_sparkline(f, interval_records, bottom[0]);
    render_cpu_heat(f, stats_by_task, bottom[1]);

    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[3]);

    let autotune_snapshot = load_default_autotune_tui_panel_snapshot();
    render_autotune_panel(f, Some(&autotune_snapshot), lower[0]);
    render_diagnoses(f, recent_diagnoses, lower[1]);
}

// ---------------------------------------------------------------------------
// Status bar
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn render_status_bar(
    f: &mut Frame,
    state: &TuiState,
    active_targets: &BTreeMap<u32, TaskInfo>,
    stats_by_task: &BTreeMap<u32, TaskStats>,
    elapsed_ms: u128,
    drop_counters: &DropCountersSnapshot,
    current_focus: Option<&ResolvedFocus>,
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
    let focus_line = focus_status_line(current_focus);
    let paragraph = Paragraph::new(vec![Line::from(parts), focus_line]).block(block);
    f.render_widget(paragraph, area);
}

fn focus_status_line(current_focus: Option<&ResolvedFocus>) -> Line<'static> {
    let Some(focus) = current_focus else {
        return Line::from(vec![Span::raw(" Focus: none")]);
    };

    let confidence_percent = (focus.group.confidence * 100.0).round() as u32;
    let roots = if focus.group.root_pids.is_empty() {
        "-".to_owned()
    } else {
        focus
            .group
            .root_pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };

    let display_name = if focus.group.display_name.is_empty() {
        format!("{:?}", focus.group.kind)
    } else {
        focus.group.display_name.clone()
    };

    Line::from(vec![Span::styled(
        format!(
            " Focus: {:?} {} | confidence {}% | roots: {} | situation: {:?}",
            focus.group.kind, display_name, confidence_percent, roots, focus.situation
        ),
        Style::default().fg(Color::Cyan),
    )])
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
    let mut by_tick: BTreeMap<u64, u64> = BTreeMap::new();
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

// ---------------------------------------------------------------------------
// Autotune panel
// ---------------------------------------------------------------------------

fn render_autotune_panel(f: &mut Frame, snapshot: Option<&AutotuneTuiPanelSnapshot>, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Autotune ");
    let paragraph = Paragraph::new(autotune_panel_lines(snapshot))
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(paragraph, area);
}

fn autotune_panel_lines(snapshot: Option<&AutotuneTuiPanelSnapshot>) -> Vec<Line<'static>> {
    let Some(snapshot) = snapshot else {
        return vec![Line::from(vec![Span::raw(" no autotune status")])];
    };

    let rollback_style = if snapshot.rollback_available {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let mut lines = vec![
        label_value_line("mode", &snapshot.mode, Style::default().fg(Color::White)),
        label_value_line("phase", &snapshot.phase, Style::default().fg(Color::Cyan)),
        label_value_line(
            "current profile",
            snapshot.current_profile.as_deref().unwrap_or("none"),
            Style::default().fg(Color::White),
        ),
        label_value_line(
            "baseline score",
            &format_optional_u64(snapshot.baseline_score),
            Style::default().fg(Color::White),
        ),
        label_value_line(
            "candidate score",
            &format_optional_u64(snapshot.candidate_score),
            Style::default().fg(Color::White),
        ),
        label_value_line(
            "decision in",
            &format_decision_in(snapshot.decision_in_seconds),
            Style::default().fg(Color::Yellow),
        ),
        label_value_line(
            "rollback available",
            if snapshot.rollback_available {
                "yes"
            } else {
                "no"
            },
            rollback_style,
        ),
    ];

    if let Some(warning) = snapshot.warning.as_ref()
        && !warning.trim().is_empty()
    {
        lines.push(Line::from(vec![
            Span::styled("warning: ", Style::default().fg(Color::Yellow)),
            Span::raw(warning.clone()),
        ]));
    }

    lines
}

fn label_value_line(label: &str, value: &str, value_style: Style) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!(" {label}: ")),
        Span::styled(value.to_owned(), value_style),
    ])
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn format_decision_in(value: Option<u64>) -> String {
    value
        .map(|seconds| format!("{seconds}s"))
        .unwrap_or_else(|| "unknown".to_owned())
}

// ---------------------------------------------------------------------------
// Recent stutter diagnoses
// ---------------------------------------------------------------------------

fn render_diagnoses(
    f: &mut Frame,
    diagnoses: &std::collections::VecDeque<LiveDiagnosisEntry>,
    area: Rect,
) {
    let mut lines = Vec::new();
    for d in diagnoses.iter().rev() {
        let mut parts = vec![
            Span::styled(
                format!("elapsed={}ms ", d.elapsed_ms),
                Style::default().fg(Color::Gray),
            ),
            Span::styled(
                format!("cause={:?} ", d.cause),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("confidence={:?} ", d.confidence),
                Style::default().fg(match d.confidence {
                    crate::diagnosis::Confidence::High => Color::Red,
                    crate::diagnosis::Confidence::Medium => Color::Yellow,
                    crate::diagnosis::Confidence::Low => Color::Gray,
                }),
            ),
            Span::styled(
                format!("anchor={} ({:?}) ", d.anchor_comm, d.anchor_class),
                Style::default().fg(Color::Cyan),
            ),
        ];

        if !d.evidence.is_empty() {
            parts.push(Span::raw(format!("evidence={} ", d.evidence.join("; "))));
        }

        lines.push(Line::from(parts));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Recent stutter diagnoses ");
    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autotune_panel_lines_match_requested_fields() {
        let snapshot = AutotuneTuiPanelSnapshot {
            mode: "ApplyLowRisk".to_owned(),
            phase: "Measuring".to_owned(),
            current_profile: Some("game-main-suggested".to_owned()),
            baseline_score: Some(412),
            candidate_score: Some(330),
            decision_in_seconds: Some(12),
            rollback_available: true,
            history_path: std::path::PathBuf::from("/tmp/history.jsonl"),
            journal_path: std::path::PathBuf::from("/tmp/controller_journal.json"),
            warning: None,
        };

        let rendered = autotune_panel_lines(Some(&snapshot))
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("mode: \nApplyLowRisk"));
        assert!(rendered.contains("phase: \nMeasuring"));
        assert!(rendered.contains("current profile: \ngame-main-suggested"));
        assert!(rendered.contains("baseline score: \n412"));
        assert!(rendered.contains("candidate score: \n330"));
        assert!(rendered.contains("decision in: \n12s"));
        assert!(rendered.contains("rollback available: \nyes"));
    }

    #[test]
    fn autotune_panel_lines_handle_missing_snapshot() {
        let rendered = autotune_panel_lines(None)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, " no autotune status");
    }

    #[test]
    fn focus_status_line_formats_current_focus() {
        let focus = ResolvedFocus {
            group: crate::focus::FocusGroup {
                kind: crate::focus::FocusGroupKind::Compile,
                root_pids: vec![1234],
                member_pids: vec![1234, 1235],
                primary_pid: Some(1234),
                display_name: "cargo build".to_owned(),
                score: 0.91,
                score_breakdown: crate::focus::FocusScoreBreakdown::default(),
                confidence: 0.87,
                priority_band: crate::focus::PriorityBand::Throughput,
                reasons: vec!["cargo root with active compiler descendants".to_owned()],
            },
            selected_at_ms: 1000,
            last_confirmed_ms: 2000,
            situation: crate::autotune::state::SituationKind::CompileLoad,
        };

        let line = focus_status_line(Some(&focus));
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("Focus: Compile cargo build"));
        assert!(rendered.contains("confidence 87%"));
        assert!(rendered.contains("roots: 1234"));
        assert!(rendered.contains("situation: CompileLoad"));
    }

    #[test]
    fn focus_status_line_formats_empty_focus() {
        let line = focus_status_line(None);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, " Focus: none");
    }

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
        // Cycle until it wraps back to None
        let mut count = 0;
        while state.filter_class.is_some() && count < 100 {
            state.next_filter_class();
            count += 1;
        }
        // Should wrap back to None
        assert_eq!(state.filter_class, None);
        assert!(count > 5); // ensure we actually cycled through several classes
    }
}
