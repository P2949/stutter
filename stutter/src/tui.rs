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
    foreground::{ForegroundProviderStatus, ForegroundSource, ForegroundWindowSnapshot},
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
    current_foreground: Option<&ForegroundWindowSnapshot>,
    focus_switch_count: u64,
    foreground_include_title: bool,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // status bar + foreground line + focus line
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
        current_foreground,
        focus_switch_count,
        foreground_include_title,
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
    current_foreground: Option<&ForegroundWindowSnapshot>,
    focus_switch_count: u64,
    foreground_include_title: bool,
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
    let foreground_line = foreground_status_line(current_foreground, foreground_include_title);
    let focus_line = focus_status_line(current_focus, focus_switch_count);
    let paragraph =
        Paragraph::new(vec![Line::from(parts), foreground_line, focus_line]).block(block);
    f.render_widget(paragraph, area);
}

fn foreground_status_line(
    current_foreground: Option<&ForegroundWindowSnapshot>,
    include_title: bool,
) -> Line<'static> {
    let Some(foreground) = current_foreground else {
        return Line::from(vec![Span::raw(" foreground: none")]);
    };

    let source = foreground
        .source
        .map(foreground_source_label)
        .unwrap_or("unknown");
    let pid = foreground
        .pid
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let class = foreground
        .class
        .as_deref()
        .or(foreground.app_id.as_deref())
        .unwrap_or("-");
    let status = foreground_status_label(foreground.status);

    let mut text = format!(
        " foreground: {source} pid={pid} class={class} conf={:.2}",
        foreground.confidence
    );

    if foreground.status != ForegroundProviderStatus::Available {
        text.push_str(&format!(" status={status}"));
    }

    if include_title
        && let Some(title) = foreground.title.as_deref()
        && !title.trim().is_empty()
    {
        text.push_str(&foreground_title_fragment(title));
    }

    let style = match foreground.status {
        ForegroundProviderStatus::Available => Style::default().fg(Color::Green),
        ForegroundProviderStatus::Unavailable | ForegroundProviderStatus::Unsupported => {
            Style::default().fg(Color::Gray)
        }
        ForegroundProviderStatus::Error => Style::default().fg(Color::Yellow),
    };

    Line::from(vec![Span::styled(text, style)])
}

fn focus_status_line(
    current_focus: Option<&ResolvedFocus>,
    focus_switch_count: u64,
) -> Line<'static> {
    let Some(focus) = current_focus else {
        return Line::from(vec![Span::raw(format!(
            " focus: none switches={focus_switch_count}"
        ))]);
    };

    let roots = format!("{:?}", focus.group.root_pids);

    Line::from(vec![Span::styled(
        format!(
            " focus: {:?} roots={} switches={}",
            focus.group.kind, roots, focus_switch_count
        ),
        Style::default().fg(Color::Cyan),
    )])
}

fn foreground_source_label(source: ForegroundSource) -> &'static str {
    match source {
        ForegroundSource::Auto => "auto",
        ForegroundSource::Sway => "sway",
        ForegroundSource::Hyprland => "hyprland",
        ForegroundSource::X11 => "x11",
        ForegroundSource::Unsupported => "unsupported",
    }
}

fn foreground_status_label(status: ForegroundProviderStatus) -> &'static str {
    match status {
        ForegroundProviderStatus::Available => "available",
        ForegroundProviderStatus::Unavailable => "unavailable",
        ForegroundProviderStatus::Error => "error",
        ForegroundProviderStatus::Unsupported => "unsupported",
    }
}

fn foreground_title_fragment(title: &str) -> String {
    let mut sanitized = title
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();

    let char_count = sanitized.chars().count();
    if char_count > 48 {
        sanitized = sanitized.chars().take(48).collect::<String>();
        sanitized.push('…');
    }

    let escaped = sanitized.replace('\\', "\\\\").replace('"', "\\\"");
    format!(" title=\"{escaped}\"")
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
            planner_selected: None,
            planner_eligible: Vec::new(),
            planner_top_denied: Vec::new(),
            planner_grouped_denials: Vec::new(),
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
    fn foreground_status_line_formats_active_foreground_without_title_by_default() {
        let foreground = ForegroundWindowSnapshot {
            elapsed_ms: 1_000,
            source: Some(ForegroundSource::Sway),
            status: ForegroundProviderStatus::Available,
            pid: Some(12345),
            app_id: Some("steam_app_379430".to_owned()),
            class: Some("steam_app_379430".to_owned()),
            title: Some("Kingdom Come: Deliverance private title".to_owned()),
            window_id: Some("7".to_owned()),
            workspace: Some("gaming".to_owned()),
            confidence: 0.95,
            stale_ms: None,
            reason: "focused Sway node from swaymsg get_tree".to_owned(),
        };

        let line = foreground_status_line(Some(&foreground), false);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(
            rendered,
            " foreground: sway pid=12345 class=steam_app_379430 conf=0.95"
        );
        assert!(!rendered.contains("Kingdom Come"));
        assert!(!rendered.contains("title="));
    }

    #[test]
    fn foreground_status_line_shows_title_only_when_enabled() {
        let foreground = ForegroundWindowSnapshot {
            elapsed_ms: 1_000,
            source: Some(ForegroundSource::X11),
            status: ForegroundProviderStatus::Available,
            pid: Some(12345),
            app_id: Some("steam_app_379430".to_owned()),
            class: Some("steam_app_379430".to_owned()),
            title: Some("Kingdom Come: Deliverance".to_owned()),
            window_id: Some("0x4600007".to_owned()),
            workspace: None,
            confidence: 0.90,
            stale_ms: None,
            reason: "active X11 window from xprop".to_owned(),
        };

        let line = foreground_status_line(Some(&foreground), true);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("foreground: x11 pid=12345 class=steam_app_379430 conf=0.90"));
        assert!(rendered.contains("title=\"Kingdom Come: Deliverance\""));
    }

    #[test]
    fn foreground_status_line_formats_missing_foreground() {
        let line = foreground_status_line(None, false);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, " foreground: none");
    }

    #[test]
    fn foreground_status_line_formats_provider_error_status() {
        let foreground = ForegroundWindowSnapshot {
            elapsed_ms: 1_000,
            source: Some(ForegroundSource::Sway),
            status: ForegroundProviderStatus::Error,
            pid: None,
            app_id: None,
            class: None,
            title: None,
            window_id: None,
            workspace: None,
            confidence: 0.0,
            stale_ms: None,
            reason: "swaymsg failed".to_owned(),
        };

        let line = foreground_status_line(Some(&foreground), false);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(
            rendered,
            " foreground: sway pid=- class=- conf=0.00 status=error"
        );
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

        let line = focus_status_line(Some(&focus), 2);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("focus: Compile"));
        assert!(rendered.contains("roots=[1234]"));
        assert!(rendered.contains("switches=2"));
    }

    #[test]
    fn focus_status_line_formats_empty_focus() {
        let line = focus_status_line(None, 0);
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, " focus: none switches=0");
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
