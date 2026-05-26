use std::collections::{BTreeMap, VecDeque};

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::{SortField, TuiRenderInput, TuiState};
use crate::{
    autotune::tui_panel::AutotuneTuiPanelSnapshot,
    diagnosis::{Confidence, LiveDiagnosisEntry},
    focus::ResolvedFocus,
    foreground::{ForegroundProviderStatus, ForegroundSource, ForegroundWindowSnapshot},
    metrics::{IntervalRecord, LatencyStats, TaskStats, TaskStatsMap, format_latency},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TuiModel {
    pub(super) status_lines: Vec<TuiLine>,
    pub(super) task_rows: Vec<TuiTaskRow>,
    pub(super) sparkline_ms: Vec<u64>,
    pub(super) cpu_heat: Vec<TuiCpuHeatBar>,
    pub(super) autotune: TuiAutotunePanel,
    pub(super) diagnoses: Vec<TuiDiagnosisLine>,
}

impl TuiModel {
    pub(super) fn from_render_input(
        input: &TuiRenderInput<'_>,
        autotune_snapshot: Option<&AutotuneTuiPanelSnapshot>,
    ) -> Self {
        Self {
            status_lines: status_lines(input),
            task_rows: task_rows(input.state, input.stats_by_task),
            sparkline_ms: sparkline_ms(input.interval_records),
            cpu_heat: cpu_heat(input.stats_by_task),
            autotune: TuiAutotunePanel::from_snapshot(autotune_snapshot),
            diagnoses: diagnosis_lines(input.recent_diagnoses),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TuiTaskRow {
    pub(super) tid: String,
    pub(super) comm: String,
    pub(super) class: String,
    pub(super) samples: String,
    pub(super) max_latency: String,
    pub(super) avg_latency: String,
    pub(super) over_1ms: String,
    pub(super) max_latency_severity: TuiLatencySeverity,
}

impl TuiTaskRow {
    fn from_stats(stats: &TaskStats) -> Self {
        let max_ns = stats.session_latency.max_ns;
        Self {
            tid: stats.task.to_string(),
            comm: stats.comm.clone(),
            class: format!("{:?}", stats.class.clone()),
            samples: stats.session_latency.count.to_string(),
            max_latency: format_latency(max_ns),
            avg_latency: format_latency(avg_ns(&stats.session_latency)),
            over_1ms: stats.session_latency.over_1ms.to_string(),
            max_latency_severity: TuiLatencySeverity::from_max_ns(max_ns),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TuiAutotunePanel {
    pub(super) lines: Vec<TuiLine>,
}

impl TuiAutotunePanel {
    fn from_snapshot(snapshot: Option<&AutotuneTuiPanelSnapshot>) -> Self {
        let Some(snapshot) = snapshot else {
            return Self {
                lines: vec![TuiLine::plain(" no autotune status")],
            };
        };

        let rollback_intent = if snapshot.rollback_available {
            TuiTextIntent::GoodStrong
        } else {
            TuiTextIntent::Muted
        };

        let mut lines = vec![
            TuiLine::label_value("mode", &snapshot.mode, TuiTextIntent::Primary),
            TuiLine::label_value("phase", &snapshot.phase, TuiTextIntent::Info),
            TuiLine::label_value(
                "current profile",
                snapshot.current_profile.as_deref().unwrap_or("none"),
                TuiTextIntent::Primary,
            ),
            TuiLine::label_value(
                "baseline score",
                &format_optional_u64(snapshot.baseline_score),
                TuiTextIntent::Primary,
            ),
            TuiLine::label_value(
                "candidate score",
                &format_optional_u64(snapshot.candidate_score),
                TuiTextIntent::Primary,
            ),
            TuiLine::label_value(
                "decision in",
                &format_decision_in(snapshot.decision_in_seconds),
                TuiTextIntent::Warning,
            ),
            TuiLine::label_value(
                "rollback available",
                if snapshot.rollback_available {
                    "yes"
                } else {
                    "no"
                },
                rollback_intent,
            ),
        ];

        append_autotune_planner_lines(&mut lines, snapshot);

        if let Some(warning) = snapshot.warning.as_ref()
            && !warning.trim().is_empty()
        {
            lines.push(TuiLine::from_spans(vec![
                TuiTextSpan::styled("warning: ", TuiTextIntent::Warning),
                TuiTextSpan::plain(warning.clone()),
            ]));
        }

        Self { lines }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TuiDiagnosisLine {
    pub(super) parts: Vec<TuiTextSpan>,
}

impl TuiDiagnosisLine {
    fn from_entry(entry: &LiveDiagnosisEntry) -> Self {
        let mut parts = vec![
            TuiTextSpan::styled(
                format!("elapsed={}ms ", entry.elapsed_ms),
                TuiTextIntent::Muted,
            ),
            TuiTextSpan::styled(
                format!("cause={:?} ", entry.cause),
                TuiTextIntent::WarningStrong,
            ),
            TuiTextSpan::styled(
                format!("confidence={:?} ", entry.confidence),
                confidence_intent(entry.confidence),
            ),
            TuiTextSpan::styled(
                format!("anchor={} ({:?}) ", entry.anchor_comm, entry.anchor_class),
                TuiTextIntent::Info,
            ),
        ];

        if !entry.evidence.is_empty() {
            parts.push(TuiTextSpan::plain(format!(
                "evidence={} ",
                entry.evidence.join("; ")
            )));
        }

        Self { parts }
    }

    pub(super) fn to_ratatui_line(&self) -> Line<'static> {
        Line::from(
            self.parts
                .iter()
                .map(TuiTextSpan::to_ratatui_span)
                .collect::<Vec<_>>(),
        )
    }

    #[cfg(test)]
    fn plain_text(&self) -> String {
        self.parts.iter().map(|part| part.text.as_str()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TuiCpuHeatBar {
    pub(super) label: String,
    pub(super) max_latency_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TuiLatencySeverity {
    Normal,
    Warning,
    Critical,
}

impl TuiLatencySeverity {
    fn from_max_ns(max_ns: u64) -> Self {
        if max_ns > 5_000_000 {
            Self::Critical
        } else if max_ns > 2_000_000 {
            Self::Warning
        } else {
            Self::Normal
        }
    }

    pub(super) fn style(self) -> Style {
        match self {
            Self::Normal => Style::default().fg(Color::White),
            Self::Warning => Style::default().fg(Color::Yellow),
            Self::Critical => Style::default().fg(Color::Red),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TuiLine {
    pub(super) spans: Vec<TuiTextSpan>,
}

impl TuiLine {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            spans: vec![TuiTextSpan::plain(text)],
        }
    }

    fn from_spans(spans: Vec<TuiTextSpan>) -> Self {
        Self { spans }
    }

    fn label_value(label: &str, value: &str, value_intent: TuiTextIntent) -> Self {
        Self::from_spans(vec![
            TuiTextSpan::plain(format!(" {label}: ")),
            TuiTextSpan::styled(value.to_owned(), value_intent),
        ])
    }

    pub(super) fn to_ratatui_line(&self) -> Line<'static> {
        Line::from(
            self.spans
                .iter()
                .map(TuiTextSpan::to_ratatui_span)
                .collect::<Vec<_>>(),
        )
    }

    #[cfg(test)]
    fn plain_text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TuiTextSpan {
    text: String,
    intent: TuiTextIntent,
}

impl TuiTextSpan {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            intent: TuiTextIntent::Plain,
        }
    }

    fn styled(text: impl Into<String>, intent: TuiTextIntent) -> Self {
        Self {
            text: text.into(),
            intent,
        }
    }

    fn to_ratatui_span(&self) -> Span<'static> {
        match self.intent {
            TuiTextIntent::Plain => Span::raw(self.text.clone()),
            intent => Span::styled(self.text.clone(), intent.style()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TuiTextIntent {
    Plain,
    Primary,
    Good,
    GoodStrong,
    Muted,
    Warning,
    WarningStrong,
    Danger,
    DangerStrong,
    Info,
}

impl TuiTextIntent {
    fn style(self) -> Style {
        match self {
            Self::Plain => Style::default(),
            Self::Primary => Style::default().fg(Color::White),
            Self::Good => Style::default().fg(Color::Green),
            Self::GoodStrong => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Self::Muted => Style::default().fg(Color::Gray),
            Self::Warning => Style::default().fg(Color::Yellow),
            Self::WarningStrong => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Self::Danger => Style::default().fg(Color::Red),
            Self::DangerStrong => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            Self::Info => Style::default().fg(Color::Cyan),
        }
    }
}

fn status_lines(input: &TuiRenderInput<'_>) -> Vec<TuiLine> {
    vec![
        status_summary_line(input),
        foreground_status_line(input.current_foreground, input.foreground_include_title),
        focus_status_line(input.current_focus, input.focus_switch_count),
    ]
}

fn status_summary_line(input: &TuiRenderInput<'_>) -> TuiLine {
    let secs = input.elapsed_ms / 1000;
    let mins = secs / 60;
    let remaining = secs % 60;

    let mut parts = vec![
        TuiTextSpan::plain(format!(" Elapsed: {mins}m{remaining:02}s │ ")),
        TuiTextSpan::plain(format!(
            "Active: {}/{} │ ",
            input.active_targets.len(),
            input.stats_by_task.len()
        )),
    ];

    let drops = input.drop_counters.total();
    if drops > 0 {
        parts.push(TuiTextSpan::styled(
            format!("Drops: {drops} │ "),
            TuiTextIntent::DangerStrong,
        ));
    } else {
        parts.push(TuiTextSpan::styled("No Drops │ ", TuiTextIntent::Good));
    }

    let filter_text = input
        .state
        .filter_class
        .map(|class| format!("{class:?}"))
        .unwrap_or_else(|| "All".to_owned());
    parts.push(TuiTextSpan::plain(format!(
        "[f]Filter: {filter_text} │ [s]Sort: {} │ ",
        input.state.sort_field.label()
    )));

    if input.state.paused {
        parts.push(TuiTextSpan::styled("PAUSED", TuiTextIntent::WarningStrong));
    } else {
        parts.push(TuiTextSpan::plain("Running"));
    }
    parts.push(TuiTextSpan::plain(" │ [q]Quit"));

    TuiLine::from_spans(parts)
}

fn foreground_status_line(
    current_foreground: Option<&ForegroundWindowSnapshot>,
    include_title: bool,
) -> TuiLine {
    let Some(foreground) = current_foreground else {
        return TuiLine::plain(" foreground: none");
    };

    let source = foreground
        .source
        .map(foreground_source_label)
        .unwrap_or("unknown");
    let pid = foreground
        .decision
        .target
        .as_ref()
        .and_then(|t| t.pid)
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let class_name = foreground
        .decision
        .target
        .as_ref()
        .and_then(|t| t.class.as_deref().or(t.app_id.as_deref()))
        .unwrap_or("-");
    let status = foreground_status_label(foreground.status);

    let mut text = format!(
        " foreground: {source} pid={pid} class={class_name} conf={:.2}",
        foreground.decision.confidence
    );

    if foreground.status != ForegroundProviderStatus::Available {
        text.push_str(&format!(" status={status}"));
    }

    if include_title
        && let Some(title) = foreground
            .decision
            .target
            .as_ref()
            .and_then(|t| t.title.clone())
            .as_deref()
        && !title.trim().is_empty()
    {
        text.push_str(&foreground_title_fragment(title));
    }

    TuiLine::from_spans(vec![TuiTextSpan::styled(
        text,
        foreground_status_intent(foreground.status),
    )])
}

fn focus_status_line(current_focus: Option<&ResolvedFocus>, focus_switch_count: u64) -> TuiLine {
    let Some(focus) = current_focus else {
        return TuiLine::plain(format!(" focus: none switches={focus_switch_count}"));
    };

    let roots = format!("{:?}", focus.group.root_pids);
    TuiLine::from_spans(vec![TuiTextSpan::styled(
        format!(
            " focus: {:?} roots={} switches={}",
            focus.group.kind, roots, focus_switch_count
        ),
        TuiTextIntent::Info,
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

fn foreground_status_intent(status: ForegroundProviderStatus) -> TuiTextIntent {
    match status {
        ForegroundProviderStatus::Available => TuiTextIntent::Good,
        ForegroundProviderStatus::Unavailable | ForegroundProviderStatus::Unsupported => {
            TuiTextIntent::Muted
        }
        ForegroundProviderStatus::Error => TuiTextIntent::Warning,
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

fn task_rows(state: &TuiState, stats_by_task: &TaskStatsMap) -> Vec<TuiTaskRow> {
    let mut tasks: Vec<&TaskStats> = stats_by_task.values().collect();

    if let Some(filter) = state.filter_class {
        tasks.retain(|task| task.class == filter);
    }

    tasks.sort_by(|left, right| match state.sort_field {
        SortField::MaxLatency => right
            .session_latency
            .max_ns
            .cmp(&left.session_latency.max_ns),
        SortField::AvgLatency => avg_ns(&right.session_latency).cmp(&avg_ns(&left.session_latency)),
        SortField::Samples => right.session_latency.count.cmp(&left.session_latency.count),
    });

    tasks.into_iter().map(TuiTaskRow::from_stats).collect()
}

fn sparkline_ms(interval_records: &[IntervalRecord]) -> Vec<u64> {
    let mut by_tick: BTreeMap<u64, u64> = BTreeMap::new();
    for record in interval_records {
        let entry = by_tick.entry(record.elapsed_ms).or_insert(0);
        *entry = (*entry).max(record.max_ns);
    }

    by_tick.values().map(|ns| ns / 1_000_000).collect()
}

fn cpu_heat(stats_by_task: &TaskStatsMap) -> Vec<TuiCpuHeatBar> {
    let mut cpu_max: BTreeMap<u32, u64> = BTreeMap::new();
    for task in stats_by_task.values() {
        for (&cpu, stats) in &task.session_cpu.by_cpu {
            let current = cpu_max.entry(cpu.as_u32()).or_insert(0);
            *current = (*current).max(stats.max_ns);
        }
    }

    cpu_max
        .into_iter()
        .map(|(cpu, max_ns)| TuiCpuHeatBar {
            label: cpu.to_string(),
            max_latency_ms: max_ns / 1_000_000,
        })
        .collect()
}

fn diagnosis_lines(diagnoses: &VecDeque<LiveDiagnosisEntry>) -> Vec<TuiDiagnosisLine> {
    diagnoses
        .iter()
        .rev()
        .map(TuiDiagnosisLine::from_entry)
        .collect()
}

fn append_autotune_planner_lines(lines: &mut Vec<TuiLine>, snapshot: &AutotuneTuiPanelSnapshot) {
    if let Some(selected) = snapshot.planner_selected.as_ref() {
        lines.push(TuiLine::label_value(
            "planner selected",
            selected,
            TuiTextIntent::Good,
        ));
    }

    for eligible in snapshot.planner_eligible.iter().take(3) {
        lines.push(TuiLine::label_value(
            "planner eligible",
            eligible,
            TuiTextIntent::Primary,
        ));
    }

    for denied in snapshot.planner_top_denied.iter().take(3) {
        lines.push(TuiLine::label_value(
            "planner denied",
            denied,
            TuiTextIntent::Danger,
        ));
    }

    if !snapshot.planner_grouped_denials.is_empty() {
        lines.push(TuiLine::label_value(
            "planner denials",
            &snapshot.planner_grouped_denials.join(","),
            TuiTextIntent::Warning,
        ));
    }
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

fn confidence_intent(confidence: Confidence) -> TuiTextIntent {
    match confidence {
        Confidence::High => TuiTextIntent::Danger,
        Confidence::Medium => TuiTextIntent::Warning,
        Confidence::Low => TuiTextIntent::Muted,
    }
}

fn avg_ns(stats: &LatencyStats) -> u64 {
    if stats.count == 0 {
        0
    } else {
        (stats.sum_ns / u128::from(stats.count)) as u64
    }
}

#[cfg(test)]
#[path = "tests/model.rs"]
mod tests;
