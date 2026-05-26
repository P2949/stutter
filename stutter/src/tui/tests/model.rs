use std::collections::{BTreeMap, VecDeque};

use super::*;
use crate::{ebpf_loader::DropCountersSnapshot, metrics::CpuStatsSet, process_tree::TaskClass};

#[test]
fn foreground_status_line_formats_active_foreground_without_title_by_default() {
    let foreground = test_foreground(ForegroundSource::Sway);

    let line = foreground_status_line(Some(&foreground), false);
    let rendered = line.plain_text();

    assert_eq!(
        rendered,
        " foreground: sway pid=12345 class=steam_app_379430 conf=0.95"
    );
    assert!(!rendered.contains("Kingdom Come"));
    assert!(!rendered.contains("title="));
}

#[test]
fn foreground_status_line_shows_title_only_when_enabled() {
    let mut foreground = test_foreground(ForegroundSource::X11);
    foreground.decision.confidence = 0.90;
    foreground.decision.target.as_mut().unwrap().window_id = Some("0x4600007".to_owned());

    let line = foreground_status_line(Some(&foreground), true);
    let rendered = line.plain_text();

    assert!(rendered.contains("foreground: x11 pid=12345 class=steam_app_379430 conf=0.90"));
    assert!(rendered.contains("title=\"Kingdom Come: Deliverance\""));
}

#[test]
fn foreground_status_line_formats_missing_foreground() {
    let line = foreground_status_line(None, false);

    assert_eq!(line.plain_text(), " foreground: none");
}

#[test]
fn foreground_status_line_formats_provider_error_status() {
    let mut foreground = test_foreground(ForegroundSource::Sway);
    foreground.status = ForegroundProviderStatus::Error;
    if let Some(target) = foreground.decision.target.as_mut() {
        target.pid = None;
        target.app_id = None;
        target.class = None;
        target.title = None;
    }
    foreground.decision.confidence = 0.0;
    foreground.decision.reasons = vec![crate::foreground::ForegroundReason::new("swaymsg failed")];

    let line = foreground_status_line(Some(&foreground), false);

    assert_eq!(
        line.plain_text(),
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

    let rendered = focus_status_line(Some(&focus), 2).plain_text();

    assert!(rendered.contains("focus: Compile"));
    assert!(rendered.contains("roots=[1234]"));
    assert!(rendered.contains("switches=2"));
}

#[test]
fn focus_status_line_formats_empty_focus() {
    let line = focus_status_line(None, 0);

    assert_eq!(line.plain_text(), " focus: none switches=0");
}

#[test]
fn task_rows_filter_sort_and_format_latency_without_terminal() {
    let mut stats_by_task = BTreeMap::new();
    stats_by_task.insert(
        1.into(),
        task_stats(1, "worker", TaskClass::Helper, &[1_000_000]),
    );
    stats_by_task.insert(
        2.into(),
        task_stats(
            2,
            "game",
            TaskClass::Game,
            &[1_000_000, 7_000_000, 3_000_000],
        ),
    );

    let state = TuiState {
        sort_field: SortField::AvgLatency,
        filter_class: Some(TaskClass::Game),
        paused: false,
    };

    let rows = task_rows(&state, &stats_by_task);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tid, "2");
    assert_eq!(rows[0].comm, "game");
    assert_eq!(rows[0].samples, "3");
    assert_eq!(rows[0].max_latency, "7.000ms");
    assert_eq!(rows[0].avg_latency, "3.667ms");
    assert_eq!(rows[0].max_latency_severity, TuiLatencySeverity::Critical);
}

#[test]
fn sparkline_groups_max_latency_per_tick_without_terminal() {
    let records = vec![
        interval_record(10, 1_000_000),
        interval_record(10, 3_000_000),
        interval_record(20, 2_000_000),
    ];

    assert_eq!(sparkline_ms(&records), vec![3, 2]);
}

#[test]
fn cpu_heat_groups_max_latency_per_cpu_without_terminal() {
    let mut first = task_stats(1, "first", TaskClass::Game, &[1_000_000]);
    first.session_cpu = cpu_stats_set([(2, 5_000_000), (0, 2_000_000)]);
    let mut second = task_stats(2, "second", TaskClass::Helper, &[1_000_000]);
    second.session_cpu = cpu_stats_set([(0, 4_000_000)]);

    let mut stats_by_task = BTreeMap::new();
    stats_by_task.insert(1.into(), first);
    stats_by_task.insert(2.into(), second);

    let bars = cpu_heat(&stats_by_task);

    assert_eq!(
        bars,
        vec![
            TuiCpuHeatBar {
                label: "0".to_owned(),
                max_latency_ms: 4,
            },
            TuiCpuHeatBar {
                label: "2".to_owned(),
                max_latency_ms: 5,
            },
        ]
    );
}

#[test]
fn autotune_panel_lines_match_requested_fields() {
    let snapshot = autotune_snapshot();

    let rendered = TuiAutotunePanel::from_snapshot(Some(&snapshot))
        .lines
        .iter()
        .map(TuiLine::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("mode: ApplyLowRisk"));
    assert!(rendered.contains("phase: Measuring"));
    assert!(rendered.contains("current profile: game-main-suggested"));
    assert!(rendered.contains("baseline score: 412"));
    assert!(rendered.contains("candidate score: 330"));
    assert!(rendered.contains("decision in: 12s"));
    assert!(rendered.contains("rollback available: yes"));
}

#[test]
fn autotune_panel_lines_render_planner_summary_fields() {
    let snapshot = AutotuneTuiPanelSnapshot {
        mode: "Suggest".to_owned(),
        phase: "Observing".to_owned(),
        current_profile: None,
        baseline_score: None,
        candidate_score: None,
        decision_in_seconds: None,
        rollback_available: false,
        history_path: std::path::PathBuf::from("/tmp/history.jsonl"),
        journal_path: std::path::PathBuf::from("/tmp/controller_journal.json"),
        warning: None,
        planner_selected: Some(
            "cpu_affinity_profile game-main objective=StutterScore confidence=0.920 evidence=situation=GameFocused weight=0.95"
                .to_owned(),
        ),
        planner_eligible: vec![
            "cpu_affinity_profile game-main objective=StutterScore confidence=0.920 evidence=situation=GameFocused weight=0.95"
                .to_owned(),
        ],
        planner_top_denied: vec![
            "nice nice-denied objective=DesktopInteractivity confidence=0.610 reasons=capability_missing evidence=capability=nice weight=1.00"
                .to_owned(),
        ],
        planner_grouped_denials: vec!["capability_missing=1".to_owned()],
    };

    let rendered = TuiAutotunePanel::from_snapshot(Some(&snapshot))
        .lines
        .iter()
        .map(TuiLine::plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("planner selected:"));
    assert!(rendered.contains("planner eligible:"));
    assert!(rendered.contains("planner denied:"));
    assert!(rendered.contains("planner denials:"));
    assert!(rendered.contains("objective=StutterScore"));
    assert!(rendered.contains("confidence=0.920"));
    assert!(rendered.contains("evidence=situation=GameFocused weight=0.95"));
    assert!(rendered.contains("reasons=capability_missing"));
    assert!(rendered.contains("evidence=capability=nice weight=1.00"));
    assert!(rendered.contains("capability_missing=1"));
}

#[test]
fn autotune_panel_lines_handle_missing_snapshot() {
    let panel = TuiAutotunePanel::from_snapshot(None);

    assert_eq!(panel.lines[0].plain_text(), " no autotune status");
}

#[test]
fn diagnosis_line_formats_evidence_without_terminal() {
    let entry = LiveDiagnosisEntry {
        elapsed_ms: 42,
        cause: crate::diagnosis::StutterCause::GameThreadSchedulerDelay,
        confidence: Confidence::High,
        anchor_class: TaskClass::Game,
        anchor_comm: "game-thread".to_owned(),
        evidence: vec!["p99=7ms".to_owned(), "cpu=2".to_owned()],
    };

    let line = TuiDiagnosisLine::from_entry(&entry);

    assert_eq!(
        line.plain_text(),
        "elapsed=42ms cause=GameThreadSchedulerDelay confidence=High anchor=game-thread (Game) evidence=p99=7ms; cpu=2 "
    );
}

#[test]
fn full_model_builds_owned_formatting_without_terminal() {
    let mut active_targets = BTreeMap::new();
    active_targets.insert(
        9.into(),
        task_stats(9, "game", TaskClass::Game, &[2_000_000]).task_info(),
    );
    let mut stats_by_task = BTreeMap::new();
    stats_by_task.insert(
        9.into(),
        task_stats(9, "game", TaskClass::Game, &[2_000_000]),
    );
    let diagnoses = VecDeque::new();
    let state = TuiState::default();
    let drops = DropCountersSnapshot::default();
    let records = vec![interval_record(10, 2_000_000)];
    let input = TuiRenderInput {
        state: &state,
        active_targets: &active_targets,
        stats_by_task: &stats_by_task,
        interval_records: &records,
        recent_diagnoses: &diagnoses,
        elapsed_ms: 65_000,
        drop_counters: &drops,
        current_focus: None,
        current_foreground: None,
        focus_switch_count: 0,
        foreground_include_title: false,
    };

    let model = TuiModel::from_render_input(&input, None);

    assert_eq!(
        model.status_lines[0].plain_text(),
        " Elapsed: 1m05s │ Active: 1/1 │ No Drops │ [f]Filter: All │ [s]Sort: Max Latency │ Running │ [q]Quit"
    );
    assert_eq!(model.task_rows[0].tid, "9");
    assert_eq!(model.sparkline_ms, vec![2]);
    assert_eq!(model.autotune.lines[0].plain_text(), " no autotune status");
}

fn test_foreground(source: ForegroundSource) -> ForegroundWindowSnapshot {
    ForegroundWindowSnapshot::available(crate::foreground::ForegroundAvailableInput {
        elapsed_ms: 1_000,
        source,
        pid: Some(12345),
        app_id: Some("steam_app_379430".to_owned()),
        class: Some("steam_app_379430".to_owned()),
        title: Some("Kingdom Come: Deliverance".to_owned()),
        include_title: true,
        window_id: Some("7".to_owned()),
        workspace: Some("gaming".to_owned()),
        confidence: 0.95,
        reason: "focused Sway node from swaymsg get_tree".to_owned(),
    })
}

fn task_stats(task: u32, comm: &str, class: TaskClass, latencies: &[u64]) -> TaskStats {
    let mut stats = TaskStats::new(task, comm.to_owned(), 0);
    stats.class = class;
    for &latency in latencies {
        stats.session_latency.record(latency);
    }
    stats
}

fn interval_record(elapsed_ms: u64, max_ns: u64) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        max_ns,
        ..IntervalRecord::default()
    }
}

fn cpu_stats_set<const N: usize>(entries: [(u32, u64); N]) -> CpuStatsSet {
    let mut set = CpuStatsSet::new();
    for (cpu, max_ns) in entries {
        set.record(cpu.into(), max_ns, u64::MAX);
    }
    set
}

fn autotune_snapshot() -> AutotuneTuiPanelSnapshot {
    AutotuneTuiPanelSnapshot {
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
    }
}
