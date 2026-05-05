use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    fs,
    path::Path,
};

use anyhow::Context;
use serde::Serialize;

use crate::{
    diagnosis::{ClusterAnchorKind, Diagnosis, FrameDiagnosis, diagnose_cluster, select_anchor},
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{
        FrameEvent, RecordedSpike, SESSION_SCHEMA_VERSION, SessionFile, SessionTask, SpikeEvent,
    },
    session_io::{self, ArtifactLoadOptions},
};

const MIN_CLUSTER_TASKS: usize = 3;
const MAX_INLINE_CLUSTER_POINTS: usize = 8;
const MAX_CLUSTER_CANDIDATES: usize = 4096;

#[derive(Clone, Serialize)]
pub(crate) struct SpikePoint {
    pub(crate) task: u32,
    pub(crate) class: TaskClass,
    pub(crate) process_pid: Option<u32>,
    pub(crate) comm: String,
    pub(crate) cpu: u32,
    pub(crate) wakeup_target_cpu: u32,
    pub(crate) latency_ns: u64,
    pub(crate) wakeup_ns: u64,
    pub(crate) switch_ns: u64,
    pub(crate) target_pending_wakeups: u32,
    pub(crate) elapsed_ms: Option<u128>,
    pub(crate) scx_ops: Option<String>,
    pub(crate) scx_state: Option<String>,
}

#[derive(Clone, Serialize)]
pub(crate) struct SpikeCluster {
    pub(crate) points: Vec<SpikePoint>,
    pub(crate) distinct_tasks: usize,
    pub(crate) min_switch_ns: u64,
    pub(crate) max_switch_ns: u64,
    pub(crate) max_latency_ns: u64,
    pub(crate) diagnosis: Option<Diagnosis>,
    pub(crate) anchor_task: Option<u32>,
    pub(crate) anchor_class: Option<TaskClass>,
    pub(crate) anchor_comm: Option<String>,
    pub(crate) anchor_kind: Option<ClusterAnchorKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum SpikeClusterSource {
    SpikeEvents,
    TopSpikesFallback,
}

#[derive(Serialize)]
pub(crate) struct SpikeClusterAnalysis {
    pub(crate) source: SpikeClusterSource,
    pub(crate) source_count: usize,
    pub(crate) clusters: Vec<SpikeCluster>,
}

#[derive(Serialize)]
pub struct ArtifactsSummary {
    pub irq_event_count: u64,
    pub gpu_sample_count: u64,
    pub frame_event_count: u64,
    pub migration_event_count: u64,
    pub cpu_freq_sample_count: u64,
    pub block_io_event_count: u64,
    pub interval_record_count: u64,
    pub scx_event_count: u64,
}

#[derive(Serialize)]
pub struct ReportAnalysisJson {
    pub session: SessionFile,
    pub cluster_analysis: SpikeClusterAnalysis,
    pub frame_diagnoses: Vec<FrameDiagnosis>,
    pub artifacts_summary: ArtifactsSummary,
}

// Old RunArtifacts removed in favor of session_io::RunArtifacts

#[derive(Clone, Eq, PartialEq)]
struct SpikeClusterCandidate {
    start_idx: usize,
    end_idx: usize,
    distinct_tasks: usize,
    min_switch_ns: u64,
    max_switch_ns: u64,
    max_latency_ns: u64,
}

impl Ord for SpikeClusterCandidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.distinct_tasks,
            self.max_latency_ns,
            self.end_idx.saturating_sub(self.start_idx),
            std::cmp::Reverse(self.min_switch_ns),
        )
            .cmp(&(
                other.distinct_tasks,
                other.max_latency_ns,
                other.end_idx.saturating_sub(other.start_idx),
                std::cmp::Reverse(other.min_switch_ns),
            ))
    }
}

impl PartialOrd for SpikeClusterCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub fn print_report(
    path: &Path,
    json: bool,
    analysis_json: bool,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let validation = session_io::validate_run_dir(path)?;
    if !validation.is_ok() {
        for err in &validation.errors {
            log::error!("run_dir_validation_error path={} err={err}", path.display());
        }
    }
    for warning in &validation.warnings {
        log::warn!(
            "run_dir_validation_warning path={} warn={warning}",
            path.display()
        );
    }

    let mut artifacts = session_io::load_run_artifacts(path, ArtifactLoadOptions::REPORT)?;
    let session = artifacts.session.clone();
    let _metadata = artifacts.metadata.clone();

    let median_frametime = calculate_median_frametime(&artifacts.frame_events);
    let frame_spikes = identify_frame_spikes(&artifacts.frame_events, median_frametime);

    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);
    let spike_events_ref = if !artifacts.spikes.is_empty() {
        Some(&artifacts.spikes[..])
    } else {
        None
    };

    let cluster_analysis = spike_cluster_analysis(
        &session,
        spike_events_ref,
        cluster_window_ns,
        top,
        filter_class,
    );

    let windows = compute_correlation_windows(
        &session,
        &cluster_analysis.clusters,
        &frame_spikes,
        cluster_window_ns,
    );
    artifacts.load_correlations(windows)?;

    let mut cluster_analysis = cluster_analysis;
    perform_diagnosis(
        &mut cluster_analysis.clusters,
        &artifacts,
        cluster_window_ns,
    );

    let all_spike_points = if !artifacts.spikes.is_empty() {
        flatten_spike_events(&session, &artifacts.spikes)
    } else {
        flatten_top_spikes(&session)
    };

    let frame_diagnoses = perform_frame_diagnosis(
        &session,
        &frame_spikes,
        &all_spike_points,
        &artifacts,
        cluster_window_ns,
    );

    if json {
        println!("{}", serde_json::to_string_pretty(&session)?);
        return Ok(());
    }

    if analysis_json {
        let artifacts_summary = ArtifactsSummary {
            irq_event_count: session.irq_event_count,
            gpu_sample_count: session.gpu_sample_count,
            frame_event_count: session.frame_event_count,
            migration_event_count: session.migration_event_count.unwrap_or(0),
            cpu_freq_sample_count: session.cpu_freq_sample_count.unwrap_or(0),
            block_io_event_count: session.block_io_event_count,
            interval_record_count: session.interval_record_count,
            scx_event_count: session.scx_event_count,
        };
        let report = ReportAnalysisJson {
            session,
            cluster_analysis,
            frame_diagnoses,
            artifacts_summary,
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    print!(
        "{}",
        render_report(
            path,
            &session,
            &cluster_analysis,
            &frame_diagnoses,
            &artifacts,
            top,
            cluster_window_ms,
            filter_class,
        )
    );

    Ok(())
}

pub fn render_diff_report(
    path_a: &Path,
    path_b: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<String> {
    let session_a = crate::session_io::load_session(path_a)?;
    let session_b = crate::session_io::load_session(path_b)?;

    let mut output = String::new();
    pushln(&mut output, "stutter diff report");
    pushln(&mut output, "===================");
    pushln(
        &mut output,
        format!(
            "run_a: {} ({}ms)",
            session_a.run_name.as_deref().unwrap_or("-"),
            session_a.duration_ms,
        ),
    );
    pushln(
        &mut output,
        format!(
            "run_b: {} ({}ms)",
            session_b.run_name.as_deref().unwrap_or("-"),
            session_b.duration_ms,
        ),
    );
    pushln(&mut output, "");

    // Aggregate tasks by stable identity (class, process_comm, comm). Many
    // games spawn multiple worker threads with the same `comm`; collapsing
    // them loses information. Aggregate counts by summing counters and
    // taking conservative maxima for latency metrics.
    #[derive(Clone)]
    struct Agg {
        max_ns: u64,
        p99_ns: u64,
        over_1ms: u64,
    }

    let mut tasks_a: BTreeMap<(TaskClass, String, String), Agg> = BTreeMap::new();
    for t in session_a
        .tasks
        .iter()
        .filter(|t| t.latency.samples > 0)
        .filter(|t| filter_class.is_none_or(|c| t.class == c))
    {
        let key = (t.class, t.process_comm.to_string(), t.comm.clone());
        let entry = tasks_a.entry(key.clone()).or_insert(Agg {
            max_ns: 0,
            p99_ns: 0,
            over_1ms: 0,
        });
        entry.max_ns = entry.max_ns.max(t.latency.max_ns);
        entry.p99_ns = entry.p99_ns.max(t.latency.p99_ns);
        entry.over_1ms = entry.over_1ms.saturating_add(t.latency.over_1ms);
    }

    let mut tasks_b: BTreeMap<(TaskClass, String, String), Agg> = BTreeMap::new();
    for t in session_b
        .tasks
        .iter()
        .filter(|t| t.latency.samples > 0)
        .filter(|t| filter_class.is_none_or(|c| t.class == c))
    {
        let key = (t.class, t.process_comm.to_string(), t.comm.clone());
        let entry = tasks_b.entry(key.clone()).or_insert(Agg {
            max_ns: 0,
            p99_ns: 0,
            over_1ms: 0,
        });
        entry.max_ns = entry.max_ns.max(t.latency.max_ns);
        entry.p99_ns = entry.p99_ns.max(t.latency.p99_ns);
        entry.over_1ms = entry.over_1ms.saturating_add(t.latency.over_1ms);
    }

    struct TaskDelta {
        comm: String,
        process_comm: String,
        class: TaskClass,
        delta_max_ns: i64,
        delta_p99_ns: i64,
        delta_over_1ms: i64,
        max_a: u64,
        max_b: u64,
    }

    let mut regressions = Vec::new();
    let mut improvements = Vec::new();

    for (key, ta) in &tasks_a {
        if let Some(tb) = tasks_b.get(key) {
            let delta_max = tb.max_ns as i64 - ta.max_ns as i64;
            let delta_p99 = tb.p99_ns as i64 - ta.p99_ns as i64;
            let delta_over = tb.over_1ms as i64 - ta.over_1ms as i64;
            let d = TaskDelta {
                comm: key.2.clone(),
                process_comm: key.1.clone(),
                class: key.0,
                delta_max_ns: delta_max,
                delta_p99_ns: delta_p99,
                delta_over_1ms: delta_over,
                max_a: ta.max_ns,
                max_b: tb.max_ns,
            };
            if delta_max > 0 {
                regressions.push(d);
            } else if delta_max < 0 {
                improvements.push(d);
            }
        }
    }

    regressions.sort_by_key(|d| std::cmp::Reverse(d.delta_max_ns));
    improvements.sort_by_key(|d| d.delta_max_ns);

    pushln(&mut output, "summary highlights");
    pushln(&mut output, "------------------");
    if let Some(worst) = regressions.first() {
        let pct = if worst.max_a > 0 {
            format!(
                " (+{:.1}%)",
                (worst.delta_max_ns as f64 / worst.max_a as f64) * 100.0
            )
        } else {
            String::new()
        };
        pushln(
            &mut output,
            format!(
                "biggest regression:  {} on comm={} process={}{}",
                format_latency_signed(worst.delta_max_ns),
                worst.comm,
                worst.process_comm,
                pct
            ),
        );
    }
    if let Some(best) = improvements.first() {
        let pct = if best.max_a > 0 {
            format!(
                " ({:.1}%)",
                (best.delta_max_ns as f64 / best.max_a as f64) * 100.0
            )
        } else {
            String::new()
        };
        pushln(
            &mut output,
            format!(
                "biggest improvement: {} on comm={} process={}{}",
                format_latency_signed(best.delta_max_ns),
                best.comm,
                best.process_comm,
                pct
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "regressions (worse in run_b)");
    pushln(&mut output, "---------------------------");
    if regressions.is_empty() {
        pushln(&mut output, "none");
    }
    for d in regressions.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "class={:?} comm={} process={} max: {} -> {} (delta={}) p99_delta={} over_1ms_delta={}",
                d.class,
                d.comm,
                d.process_comm,
                format_latency(d.max_a),
                format_latency(d.max_b),
                format_latency_signed(d.delta_max_ns),
                format_latency_signed(d.delta_p99_ns),
                if d.delta_over_1ms >= 0 {
                    format!("+{}", d.delta_over_1ms)
                } else {
                    d.delta_over_1ms.to_string()
                },
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "improvements (better in run_b)");
    pushln(&mut output, "-----------------------------");
    if improvements.is_empty() {
        pushln(&mut output, "none");
    }
    for d in improvements.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "class={:?} comm={} process={} max: {} -> {} (delta={}) p99_delta={} over_1ms_delta={}",
                d.class,
                d.comm,
                d.process_comm,
                format_latency(d.max_a),
                format_latency(d.max_b),
                format_latency_signed(d.delta_max_ns),
                format_latency_signed(d.delta_p99_ns),
                if d.delta_over_1ms >= 0 {
                    format!("+{}", d.delta_over_1ms)
                } else {
                    d.delta_over_1ms.to_string()
                },
            ),
        );
    }
    pushln(&mut output, "");

    // Tasks only in one run
    let new_tasks: Vec<_> = tasks_b
        .keys()
        .filter(|k| !tasks_a.contains_key(k))
        .collect();
    let removed_tasks: Vec<_> = tasks_a
        .keys()
        .filter(|k| !tasks_b.contains_key(k))
        .collect();

    if !new_tasks.is_empty() {
        pushln(&mut output, "new tasks (only in run_b)");
        pushln(&mut output, "------------------------");
        for key in new_tasks.iter().take(top) {
            let (class, process_comm, comm) = key;
            pushln(
                &mut output,
                format!("comm={} process={} class={:?}", comm, process_comm, class),
            );
        }
        pushln(&mut output, "");
    }

    if !removed_tasks.is_empty() {
        pushln(&mut output, "removed tasks (only in run_a)");
        pushln(&mut output, "----------------------------");
        for key in removed_tasks.iter().take(top) {
            let (class, process_comm, comm) = key;
            pushln(
                &mut output,
                format!("comm={} process={} class={:?}", comm, process_comm, class),
            );
        }
        pushln(&mut output, "");
    }

    Ok(output)
}

pub fn check_percentile_regression(
    path_baseline: &Path,
    path_current: &Path,
    max_regression_p99_ms: f64,
) -> anyhow::Result<()> {
    let session_baseline = crate::session_io::load_session(path_baseline)?;
    let session_current = crate::session_io::load_session(path_current)?;

    let max_regression_ns = (max_regression_p99_ms * 1_000_000.0) as i64;

    #[derive(Clone)]
    struct Agg {
        p99_ns: u64,
    }

    let mut tasks_baseline: BTreeMap<(TaskClass, String, String), Agg> = BTreeMap::new();
    for t in session_baseline
        .tasks
        .iter()
        .filter(|t| t.latency.samples > 0)
    {
        let key = (t.class, t.process_comm.to_string(), t.comm.clone());
        let entry = tasks_baseline
            .entry(key.clone())
            .or_insert(Agg { p99_ns: 0 });
        entry.p99_ns = entry.p99_ns.max(t.latency.p99_ns);
    }

    let mut tasks_current: BTreeMap<(TaskClass, String, String), Agg> = BTreeMap::new();
    for t in session_current
        .tasks
        .iter()
        .filter(|t| t.latency.samples > 0)
    {
        let key = (t.class, t.process_comm.to_string(), t.comm.clone());
        let entry = tasks_current
            .entry(key.clone())
            .or_insert(Agg { p99_ns: 0 });
        entry.p99_ns = entry.p99_ns.max(t.latency.p99_ns);
    }

    let mut worst_regression: Option<((TaskClass, String, String), i64)> = None;

    for (key, tb) in &tasks_current {
        let delta_p99 = if let Some(ta) = tasks_baseline.get(key) {
            tb.p99_ns as i64 - ta.p99_ns as i64
        } else if crate::scorer::class_contributes_to_score(key.0) {
            // New task in current run. If it belongs to a scored class,
            // treat its entire p99 as a regression relative to 0.
            tb.p99_ns as i64
        } else {
            0
        };

        if delta_p99 > 0 {
            if let Some((_, current_worst)) = worst_regression {
                if delta_p99 > current_worst {
                    worst_regression = Some((key.clone(), delta_p99));
                }
            } else {
                worst_regression = Some((key.clone(), delta_p99));
            }
        }
    }

    match &worst_regression {
        Some((key, delta_p99)) if *delta_p99 > max_regression_ns => {
            anyhow::bail!(
                "percentile_regression_check_failed: p99 regressed by {} on comm={} process={} (max_allowed={})",
                format_latency_signed(*delta_p99),
                key.2,
                key.1,
                format_latency(max_regression_ns as u64)
            );
        }
        _ => {}
    }

    println!(
        "percentile_regression_check_passed: baseline={} current={} max_regression_p99_ms={}",
        path_baseline.display(),
        path_current.display(),
        max_regression_p99_ms
    );
    if let Some((key, delta_p99)) = &worst_regression {
        println!(
            "worst_p99_regression: {} on comm={} process={}",
            format_latency_signed(*delta_p99),
            key.2,
            key.1
        );
    } else {
        println!("worst_p99_regression: none");
    }

    Ok(())
}

pub fn print_diff_report(
    path_a: &Path,
    path_b: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    print!("{}", render_diff_report(path_a, path_b, top, filter_class)?);
    Ok(())
}

pub fn write_html_report(
    path: &Path,
    html_path: &Path,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let mut artifacts = session_io::load_run_artifacts(path, ArtifactLoadOptions::REPORT)?;
    let session = artifacts.session.clone();

    let median_frametime = calculate_median_frametime(&artifacts.frame_events);
    let frame_spikes = identify_frame_spikes(&artifacts.frame_events, median_frametime);

    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);
    let spike_events_ref = if !artifacts.spikes.is_empty() {
        Some(&artifacts.spikes[..])
    } else {
        None
    };

    let cluster_analysis = spike_cluster_analysis(
        &session,
        spike_events_ref,
        cluster_window_ns,
        top,
        filter_class,
    );

    let windows = compute_correlation_windows(
        &session,
        &cluster_analysis.clusters,
        &frame_spikes,
        cluster_window_ns,
    );
    artifacts.load_correlations(windows)?;

    let mut cluster_analysis = cluster_analysis;
    perform_diagnosis(
        &mut cluster_analysis.clusters,
        &artifacts,
        cluster_window_ns,
    );

    let all_spike_points = if !artifacts.spikes.is_empty() {
        flatten_spike_events(&session, &artifacts.spikes)
    } else {
        flatten_top_spikes(&session)
    };

    let frame_diagnoses = perform_frame_diagnosis(
        &session,
        &frame_spikes,
        &all_spike_points,
        &artifacts,
        cluster_window_ns,
    );

    let text_report = render_report(
        path,
        &session,
        &cluster_analysis,
        &frame_diagnoses,
        &artifacts,
        top,
        cluster_window_ms,
        filter_class,
    );
    let spike_events_opt = if artifacts.spikes.is_empty() {
        None
    } else {
        Some(&artifacts.spikes[..])
    };
    let html = render_html_report(
        &session,
        spike_events_opt,
        &artifacts,
        &cluster_analysis,
        &frame_diagnoses,
        &text_report,
        top,
    );
    if let Some(parent) = html_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(html_path, html)
        .with_context(|| format!("failed to write HTML report {}", html_path.display()))?;
    Ok(())
}
fn render_html_report(
    session: &SessionFile,
    spike_events: Option<&[SpikeEvent]>,
    artifacts: &session_io::RunArtifacts,
    cluster_analysis: &SpikeClusterAnalysis,
    frame_diagnoses: &[FrameDiagnosis],
    text_report: &str,
    top: usize,
) -> String {
    let session_json =
        json_escape(&serde_json::to_string(session).unwrap_or_else(|_| "{}".to_owned()));
    let spike_events_json =
        json_escape(&serde_json::to_string(&spike_events).unwrap_or_else(|_| "null".to_owned()));
    let artifacts_json =
        json_escape(&serde_json::to_string(artifacts).unwrap_or_else(|_| "{}".to_owned()));
    let cluster_analysis_json =
        json_escape(&serde_json::to_string(&cluster_analysis).unwrap_or_else(|_| "{}".to_owned()));
    let frame_diagnoses_json =
        json_escape(&serde_json::to_string(&frame_diagnoses).unwrap_or_else(|_| "[]".to_owned()));

    let template = include_str!("report_template.html");

    template
        .replace("{text_report}", &html_escape(text_report))
        .replace("{session_json}", &session_json)
        .replace("{spike_events_json}", &spike_events_json)
        .replace("{artifacts_json}", &artifacts_json)
        .replace("{cluster_analysis_json}", &cluster_analysis_json)
        .replace("{frame_diagnoses_json}", &frame_diagnoses_json)
        .replace("{top}", &top.to_string())
}

fn json_escape(s: &str) -> String {
    s.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_report(
    session_path: &Path,
    session: &SessionFile,
    cluster_analysis: &SpikeClusterAnalysis,
    frame_diagnoses: &[FrameDiagnosis],
    artifacts: &session_io::RunArtifacts,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> String {
    let mut output = String::new();

    pushln(&mut output, "stutter report");
    pushln(&mut output, "==============");
    pushln(&mut output, format!("file: {}", session_path.display()));
    pushln(&mut output, format!("schema: {}", session.schema_version));
    pushln(
        &mut output,
        format!("expected_schema: {}", SESSION_SCHEMA_VERSION),
    );
    pushln(
        &mut output,
        format!("run: {}", session.run_name.as_deref().unwrap_or("-")),
    );
    pushln(&mut output, format!("duration_ms: {}", session.duration_ms));
    pushln(&mut output, format!("stop_reason: {}", session.stop_reason));
    pushln(
        &mut output,
        format!("manual_pids: {:?}", session.config.manual_pids),
    );
    pushln(
        &mut output,
        format!("tree_roots: {:?}", session.config.tree_roots),
    );
    pushln(
        &mut output,
        format!("include_comm: {:?}", session.config.include_comm),
    );
    pushln(
        &mut output,
        format!("exclude_comm: {:?}", session.config.exclude_comm),
    );

    if session.event_stream_write_errors > 0 {
        pushln(&mut output, "recording integrity warning");
        pushln(&mut output, "---------------------------");
        pushln(
            &mut output,
            format!(
                "event_stream_write_errors={}",
                session.event_stream_write_errors
            ),
        );
        pushln(
            &mut output,
            format!(
                "first_event_stream_write_error={}",
                session
                    .first_event_stream_write_error
                    .as_deref()
                    .unwrap_or("-")
            ),
        );
        pushln(&mut output, "diagnosis may be incomplete");
        pushln(&mut output, "");
    }
    pushln(
        &mut output,
        format!(
            "watch_process: {}",
            session.config.watch_process.as_deref().unwrap_or("-")
        ),
    );
    pushln(
        &mut output,
        format!("persistent: {}", session.config.persistent),
    );
    pushln(
        &mut output,
        format!(
            "csv_path: {}",
            session
                .config
                .csv_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "-".to_owned())
        ),
    );
    pushln(
        &mut output,
        format!("active_tasks_at_end: {}", session.active_target_pids_count),
    );
    pushln(&mut output, "");

    if session.spike_events_truncated {
        pushln(&mut output, "spike event warning");
        pushln(&mut output, "-------------------");
        pushln(
            &mut output,
            format!(
                "spike_events_truncated=true retained_spike_events={} note=spike_events.json is capped; top_spikes and threshold counters remain available",
                session.spike_events_retained_count
            ),
        );
        pushln(&mut output, "");
    }

    if session.scx_event_count > 0 {
        pushln(
            &mut output,
            format!("scx_events: {}", session.scx_event_count),
        );
        pushln(&mut output, "");
    }
    if session.irq_event_count > 0
        || session.gpu_sample_count > 0
        || session.frame_event_count > 0
        || session.block_io_event_count > 0
        || session.migration_event_count.unwrap_or(0) > 0
        || session.cpu_freq_sample_count.unwrap_or(0) > 0
    {
        pushln(&mut output, "correlation artifacts");
        pushln(&mut output, "---------------------");
        pushln(
            &mut output,
            format!("irq_events: {}", session.irq_event_count),
        );
        pushln(
            &mut output,
            format!("gpu_samples: {}", session.gpu_sample_count),
        );
        pushln(
            &mut output,
            format!("frame_events: {}", session.frame_event_count),
        );
        if session.frame_event_count > 0 {
            let alignment = if session.mangohud_first_frame_monotonic_ns.is_some() {
                "monotonic_observed"
            } else {
                "approximate_first_row"
            };
            pushln(
                &mut output,
                format!("frame_timestamp_alignment={}", alignment),
            );
        }
        pushln(
            &mut output,
            format!(
                "migration_events: {}",
                session.migration_event_count.unwrap_or(0)
            ),
        );
        pushln(
            &mut output,
            format!(
                "cpu_freq_samples: {}",
                session.cpu_freq_sample_count.unwrap_or(0)
            ),
        );
        pushln(
            &mut output,
            format!(
                "io_events: {} ({}{})",
                session.block_io_event_count,
                block_io_correlation_basis(session),
                if block_io_correlation_basis(session) == "dev+sector" {
                    " correlated (advisory, approximate)"
                } else {
                    " correlated"
                },
            ),
        );
        pushln(&mut output, "");

        if session.block_io_event_count > 0 && block_io_correlation_basis(session) == "dev+sector" {
            pushln(&mut output, "block i/o correlation warning");
            pushln(&mut output, "----------------------------");
            pushln(
                &mut output,
                "note: Block I/O correlation uses dev+sector hashing. Attribution to specific",
            );
            pushln(
                &mut output,
                "      tasks is best-effort and may collide if multiple concurrent requests",
            );
            pushln(
                &mut output,
                "      target the same device and sector. Exact attribution is not guaranteed.",
            );
            pushln(&mut output, "");
        }
    }

    let truncated = session
        .tasks
        .iter()
        .filter(|task| task.latency.truncated_samples > 0)
        .collect::<Vec<_>>();

    if !truncated.is_empty() {
        pushln(&mut output, "percentile warnings");
        pushln(&mut output, "-------------------");
        for task in truncated.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "task={} comm={} truncated_samples={} percentile_scope={} note={}",
                    task.task,
                    task.comm,
                    task.latency.truncated_samples,
                    task.latency.percentile_scope,
                    percentile_warning_note(&task.latency.percentile_scope)
                ),
            );
        }
        pushln(&mut output, "");
    }

    let mut tasks = session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|c| task.class == c))
        .collect::<Vec<_>>();

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));

    pushln(&mut output, "top tasks by max latency");
    pushln(&mut output, "------------------------");
    let duration_secs = session.duration_ms as f64 / 1000.0;
    for task in tasks.iter().take(top) {
        let spike_rate = if duration_secs > 0.0 {
            task.latency.over_1ms as f64 / duration_secs
        } else {
            0.0
        };
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} process_pid={:?} samples={} max={} over_1ms={} over_2ms={} over_5ms={} spike_rate_per_s={:.1} percentile_scope={}",
                task.task,
                task.active,
                task.class,
                task.comm,
                task.process_pid,
                task.latency.samples,
                format_latency(task.latency.max_ns),
                task.latency.over_1ms,
                task.latency.over_2ms,
                task.latency.over_5ms,
                spike_rate,
                task.latency.percentile_scope,
            ),
        );
    }
    pushln(&mut output, "");

    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.over_5ms),
            std::cmp::Reverse(task.latency.over_2ms),
            std::cmp::Reverse(task.latency.over_1ms),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });

    pushln(&mut output, "top tasks by threshold counters");
    pushln(&mut output, "-------------------------------");
    for task in tasks.iter().take(top) {
        let spike_rate = if duration_secs > 0.0 {
            task.latency.over_1ms as f64 / duration_secs
        } else {
            0.0
        };
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} over_5ms={} over_2ms={} over_1ms={} spike_rate_per_s={:.1} max={}",
                task.task,
                task.active,
                task.class,
                task.comm,
                task.latency.over_5ms,
                task.latency.over_2ms,
                task.latency.over_1ms,
                spike_rate,
                format_latency(task.latency.max_ns),
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "top spikes");
    pushln(&mut output, "----------");
    for spike in session.top_spikes.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} cpu={} wakeup_target_cpu={} latency={} wakeup_ns={} switch_ns={} target_pending_on_switch_cpu={}",
                spike.task,
                spike.active,
                spike.class,
                spike.comm,
                spike.cpu,
                spike.wakeup_target_cpu,
                format_latency(spike.latency_ns),
                spike.wakeup_ns,
                spike.switch_ns,
                spike.target_pending_wakeups,
            ),
        );
    }
    pushln(&mut output, "");

    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);

    pushln(&mut output, "spike clusters");
    pushln(&mut output, "--------------");
    pushln(
        &mut output,
        render_cluster_source(cluster_analysis, cluster_window_ms),
    );
    pushln(
        &mut output,
        "target_pending_on_switch_cpu is a rough advisory-only diagnostic: it counts other monitored",
    );
    pushln(
        &mut output,
        "wakeup records still pending on the CPU that actually ran the task.",
    );
    pushln(
        &mut output,
        "It is not kernel runqueue depth and must not be used for scoring or tuning decisions.",
    );
    pushln(&mut output, "");
    if cluster_analysis.clusters.is_empty() {
        pushln(
            &mut output,
            format!(
                "none min_tasks={} window_ms={}",
                MIN_CLUSTER_TASKS, cluster_window_ms
            ),
        );
    } else {
        for (rank, cluster) in cluster_analysis.clusters.iter().take(top).enumerate() {
            pushln(&mut output, render_cluster(rank + 1, cluster));
        }
    }
    pushln(&mut output, "");

    if !frame_diagnoses.is_empty() {
        pushln(&mut output, "frame spike diagnoses");
        pushln(&mut output, "---------------------");
        for (rank, diag) in frame_diagnoses.iter().take(top).enumerate() {
            pushln(&mut output, render_frame_diagnosis(rank + 1, diag));
        }
        pushln(&mut output, "");
    }

    render_correlation_sections(
        &mut output,
        &cluster_analysis.clusters,
        artifacts,
        block_io_correlation_basis(session),
        cluster_window_ns,
        top,
    );

    output
}

fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

fn block_io_correlation_basis(session: &SessionFile) -> &str {
    if session.block_io_correlation_basis.is_empty() {
        "dev+sector"
    } else {
        &session.block_io_correlation_basis
    }
}

fn format_latency_signed(ns: i64) -> String {
    let abs_ns = ns.unsigned_abs();
    let sign = if ns >= 0 { "+" } else { "-" };
    format!("{sign}{}", format_latency(abs_ns))
}

pub(crate) fn spike_cluster_analysis(
    session: &SessionFile,
    spike_events: Option<&[SpikeEvent]>,
    cluster_window_ns: u64,
    top: usize,
    filter_class: Option<TaskClass>,
) -> SpikeClusterAnalysis {
    let (source, mut points) = match spike_events {
        Some(spike_events) => (
            SpikeClusterSource::SpikeEvents,
            flatten_spike_events(session, spike_events),
        ),
        None => (
            SpikeClusterSource::TopSpikesFallback,
            flatten_top_spikes(session),
        ),
    };

    if let Some(class) = filter_class {
        points.retain(|p| p.class == class);
    }

    let source_count = points.len();

    SpikeClusterAnalysis {
        source,
        source_count,
        clusters: spike_clusters_from_points(points, cluster_window_ns, top),
    }
}

fn spike_clusters_from_points(
    mut points: Vec<SpikePoint>,
    cluster_window_ns: u64,
    top: usize,
) -> Vec<SpikeCluster> {
    points.sort_by_key(|point| point.switch_ns);

    let mut candidates = BinaryHeap::new();
    let mut task_counts = BTreeMap::<u32, usize>::new();
    let mut max_latency_candidates = std::collections::VecDeque::<usize>::new();
    let mut left_idx = 0;

    for right_idx in 0..points.len() {
        *task_counts.entry(points[right_idx].task).or_default() += 1;

        while max_latency_candidates
            .back()
            .is_some_and(|idx| points[*idx].latency_ns <= points[right_idx].latency_ns)
        {
            max_latency_candidates.pop_back();
        }
        max_latency_candidates.push_back(right_idx);

        while left_idx <= right_idx
            && points[right_idx]
                .switch_ns
                .saturating_sub(points[left_idx].switch_ns)
                > cluster_window_ns
        {
            decrement_task_count(&mut task_counts, points[left_idx].task);
            if max_latency_candidates.front() == Some(&left_idx) {
                max_latency_candidates.pop_front();
            }
            left_idx += 1;
        }

        if task_counts.len() >= MIN_CLUSTER_TASKS {
            let max_latency_ns = max_latency_candidates
                .front()
                .map(|idx| points[*idx].latency_ns)
                .unwrap_or(0);

            let candidate = SpikeClusterCandidate {
                start_idx: left_idx,
                end_idx: right_idx + 1,
                distinct_tasks: task_counts.len(),
                min_switch_ns: points[left_idx].switch_ns,
                max_switch_ns: points[right_idx].switch_ns,
                max_latency_ns,
            };

            if candidates.len() < MAX_CLUSTER_CANDIDATES {
                candidates.push(std::cmp::Reverse(candidate));
            } else if let Some(mut worst) = candidates.peek_mut()
                && candidate > worst.0
            {
                *worst = std::cmp::Reverse(candidate);
            }
        }
    }

    let mut candidates_vec: Vec<_> = candidates.into_iter().map(|r| r.0).collect();
    candidates_vec.sort_by(|a, b| b.cmp(a));

    let mut selected_candidates = Vec::new();
    let max_selected = top.saturating_mul(4).min(MAX_CLUSTER_CANDIDATES);

    // Sweep-line: track selected intervals by max_switch_ns in a BTreeSet
    // for O(log n) overlap checking instead of O(n) per candidate.
    let mut selected_intervals: BTreeSet<(u64, u64)> = BTreeSet::new(); // (max_switch_ns, min_switch_ns)

    for candidate in candidates_vec {
        // Check for overlap: we need intervals where
        //   existing.min_switch_ns <= candidate.max_switch_ns AND
        //   existing.max_switch_ns >= candidate.min_switch_ns
        //
        // Since intervals are stored as (max_switch_ns, min_switch_ns),
        // we look for entries whose max_switch_ns >= candidate.min_switch_ns.
        let overlaps = selected_intervals
            .range((candidate.min_switch_ns, 0)..)
            .any(|(_, min_ns)| *min_ns <= candidate.max_switch_ns);

        if !overlaps {
            selected_intervals.insert((candidate.max_switch_ns, candidate.min_switch_ns));
            selected_candidates.push(candidate);
            if selected_candidates.len() >= max_selected {
                break;
            }
        }
    }

    selected_candidates
        .into_iter()
        .map(|candidate| {
            cluster_from_points(
                points[candidate.start_idx..candidate.end_idx].to_vec(),
                candidate.distinct_tasks,
            )
        })
        .collect()
}

fn flatten_spike_events(session: &SessionFile, spike_events: &[SpikeEvent]) -> Vec<SpikePoint> {
    spike_events
        .iter()
        .map(|spike| SpikePoint {
            task: spike.task,
            class: spike.class,
            process_pid: spike.process_pid,
            comm: spike.comm.clone(),
            cpu: spike.cpu,
            wakeup_target_cpu: spike.wakeup_target_cpu,
            latency_ns: spike.latency_ns,
            wakeup_ns: spike.wakeup_ns,
            switch_ns: spike.switch_ns,
            target_pending_wakeups: spike.target_pending_wakeups,
            elapsed_ms: elapsed_ms(session.monotonic_start_ns, spike.switch_ns)
                .or(spike.elapsed_ms),
            scx_ops: spike.scx_ops.clone(),
            scx_state: spike.scx_state.clone(),
        })
        .collect()
}

fn flatten_top_spikes(session: &SessionFile) -> Vec<SpikePoint> {
    let mut points = Vec::new();

    for task in &session.tasks {
        for spike in &task.top_spikes {
            points.push(spike_point_from_task(
                task,
                spike,
                elapsed_ms(session.monotonic_start_ns, spike.switch_ns),
            ));
        }
    }

    points
}

fn spike_point_from_task(
    task: &SessionTask,
    spike: &RecordedSpike,
    elapsed_ms: Option<u128>,
) -> SpikePoint {
    SpikePoint {
        task: task.task,
        class: spike.class,
        process_pid: spike.process_pid,
        comm: task.comm.clone(),
        cpu: spike.cpu,
        wakeup_target_cpu: spike.wakeup_target_cpu,
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        target_pending_wakeups: spike.target_pending_wakeups,
        elapsed_ms,
        scx_ops: spike.scx_ops.clone(),
        scx_state: spike.scx_state.clone(),
    }
}

fn elapsed_ms(monotonic_start_ns: Option<u64>, switch_ns: u64) -> Option<u128> {
    let start_ns = monotonic_start_ns?;
    switch_ns
        .checked_sub(start_ns)
        .map(|elapsed_ns| u128::from(elapsed_ns / 1_000_000))
}

fn decrement_task_count(task_counts: &mut BTreeMap<u32, usize>, task: u32) {
    let Some(count) = task_counts.get_mut(&task) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        task_counts.remove(&task);
    }
}

// retain_cluster_candidate removed as it's replaced by BinaryHeap logic above

// compare_cluster_candidates removed as it's replaced by Ord implementation

fn cluster_from_points(mut points: Vec<SpikePoint>, distinct_tasks: usize) -> SpikeCluster {
    points.sort_by_key(|point| (point.switch_ns, std::cmp::Reverse(point.latency_ns)));

    let min_switch_ns = points.first().map(|point| point.switch_ns).unwrap_or(0);
    let max_switch_ns = points.last().map(|point| point.switch_ns).unwrap_or(0);
    let max_latency_ns = points
        .iter()
        .map(|point| point.latency_ns)
        .max()
        .unwrap_or(0);

    SpikeCluster {
        points,
        distinct_tasks,
        min_switch_ns,
        max_switch_ns,
        max_latency_ns,
        diagnosis: None,
        anchor_task: None,
        anchor_class: None,
        anchor_comm: None,
        anchor_kind: None,
    }
}

fn perform_diagnosis(
    clusters: &mut [SpikeCluster],
    artifacts: &session_io::RunArtifacts,
    cluster_window_ns: u64,
) {
    for cluster in clusters {
        let diagnosis = diagnose_cluster(cluster, artifacts, cluster_window_ns);
        let anchor = select_anchor(cluster);
        cluster.anchor_task = Some(anchor.task);
        cluster.anchor_class = Some(anchor.class);
        cluster.anchor_comm = Some(anchor.comm);
        cluster.anchor_kind = Some(anchor.kind);
        cluster.diagnosis = Some(diagnosis);
    }
}

fn render_cluster_source(analysis: &SpikeClusterAnalysis, cluster_window_ms: u64) -> String {
    let source = match analysis.source {
        SpikeClusterSource::SpikeEvents => "source=spike_events",
        SpikeClusterSource::TopSpikesFallback => "source=top_spikes fallback",
    };
    format!(
        "{source} count={} window_ms={} min_tasks={}",
        analysis.source_count, cluster_window_ms, MIN_CLUSTER_TASKS
    )
}

struct CorrelationCtx<'a> {
    output: &'a mut String,
    clusters: &'a [SpikeCluster],
    top: usize,
}

fn render_correlation<T, LP, MP, R>(
    ctx: &mut CorrelationCtx,
    title: &str,
    in_memory: &[T],
    mut load_predicate: LP,
    mut match_predicate: MP,
    mut render_closure: R,
) where
    T: Clone,
    LP: FnMut(&T) -> bool,
    MP: FnMut(&SpikeCluster, &T) -> bool,
    R: FnMut(&mut String, usize, &SpikeCluster, &[&T]),
{
    let pool: Vec<_> = in_memory
        .iter()
        .filter(|item| load_predicate(item))
        .collect();

    if pool.is_empty() {
        return;
    }

    pushln(ctx.output, title);
    pushln(ctx.output, "-".repeat(title.len()));

    for (rank, cluster) in ctx.clusters.iter().take(ctx.top).enumerate() {
        let matches: Vec<&T> = pool
            .iter()
            .copied()
            .filter(|item| match_predicate(cluster, item))
            .collect();

        if !matches.is_empty() {
            render_closure(ctx.output, rank, cluster, &matches);
        }
    }
    pushln(ctx.output, "");
}

fn render_correlation_sections(
    output: &mut String,
    clusters: &[SpikeCluster],
    artifacts: &session_io::RunArtifacts,
    block_io_correlation_basis: &str,
    cluster_window_ns: u64,
    top: usize,
) {
    let mut ctx = CorrelationCtx {
        output,
        clusters,
        top,
    };

    // 1. IRQ events
    let min_overall = clusters
        .iter()
        .map(|c| c.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|c| c.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    render_correlation(
        &mut ctx,
        "irq overlap",
        &artifacts.irq_events,
        |e| e.exit_ns >= min_overall && e.enter_ns <= max_overall,
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.exit_ns >= min_ns && event.enter_ns <= max_ns
        },
        |output, rank, cluster, matches| {
            let max_duration = matches.iter().map(|e| e.duration_ns).max().unwrap_or(0);
            let irq_list = matches
                .iter()
                .map(|e| e.irq)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|irq| irq.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            pushln(
                output,
                format!(
                    "cluster=#{} matches={} irqs={} max_duration={} window_ns={}..{}",
                    rank + 1,
                    matches.len(),
                    irq_list,
                    format_latency(max_duration),
                    min_ns,
                    max_ns
                ),
            );
        },
    );

    // 2. GPU samples
    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(50);
        let upper = max_overall.saturating_add(50);
        render_correlation(
            &mut ctx,
            "gpu near clusters",
            &artifacts.gpu_samples,
            |s| s.elapsed_ms >= lower && s.elapsed_ms <= upper,
            |cluster, sample| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| sample.elapsed_ms.abs_diff(elapsed) <= 50)
            },
            |output, rank, cluster, matches| {
                let elapsed = cluster_elapsed(cluster).unwrap();
                let sample = matches
                    .iter()
                    .min_by_key(|s| s.elapsed_ms.abs_diff(elapsed))
                    .unwrap();
                pushln(
                    output,
                    format!(
                        "cluster=#{} sample_elapsed={} gpu_busy={} gpu_clock_mhz={} mem_clock_mhz={} temp_mC={} power_uW={}",
                        rank + 1,
                        format_elapsed(Some(sample.elapsed_ms)),
                        format_option(sample.gpu_busy_percent),
                        format_option(sample.gpu_clock_mhz),
                        format_option(sample.mem_clock_mhz),
                        format_option(sample.temp_millidegrees),
                        format_option(sample.power_microwatts),
                    ),
                );
            },
        );
    }

    // 3. Frame events
    let padding_ms = u128::from(cluster_window_ns / 1_000_000).max(1);
    let min_overall_opt = clusters
        .iter()
        .filter_map(|c| cluster_elapsed_range(c).map(|(min, _)| min))
        .min();
    let max_overall_opt = clusters
        .iter()
        .filter_map(|c| cluster_elapsed_range(c).map(|(_, max)| max))
        .max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(padding_ms);
        let upper = max_overall.saturating_add(padding_ms);
        render_correlation(
            &mut ctx,
            "frame overlap",
            &artifacts.frame_events,
            |f| f.elapsed_ms >= lower && f.elapsed_ms <= upper,
            |cluster, frame| {
                cluster_elapsed_range(cluster).is_some_and(|(min_e, max_e)| {
                    let min_e = min_e.saturating_sub(padding_ms);
                    let max_e = max_e.saturating_add(padding_ms);
                    frame.elapsed_ms >= min_e && frame.elapsed_ms <= max_e
                })
            },
            |output, rank, cluster, matches| {
                let (min_e, max_e) = cluster_elapsed_range(cluster).unwrap();
                let min_e = min_e.saturating_sub(padding_ms);
                let max_e = max_e.saturating_add(padding_ms);
                let max_frame = matches
                    .iter()
                    .map(|f| f.frametime_ms)
                    .fold(0.0_f64, f64::max);
                pushln(
                    output,
                    format!(
                        "cluster=#{} frames={} max_frametime_ms={:.3} elapsed={}..{}",
                        rank + 1,
                        matches.len(),
                        max_frame,
                        min_e,
                        max_e
                    ),
                );
            },
        );
    }

    // 4. Migration events
    let min_overall = clusters
        .iter()
        .map(|c| c.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|c| c.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    render_correlation(
        &mut ctx,
        "migration overlap",
        &artifacts.migration_events,
        |e| e.timestamp_ns >= min_overall && e.timestamp_ns <= max_overall,
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.timestamp_ns >= min_ns && event.timestamp_ns <= max_ns
        },
        |output, rank, cluster, matches| {
            let tids = matches
                .iter()
                .map(|e| e.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            pushln(
                output,
                format!(
                    "cluster=#{} matches={} tids={} window_ns={}..{}",
                    rank + 1,
                    matches.len(),
                    tids,
                    min_ns,
                    max_ns
                ),
            );
        },
    );

    // 5. CPU freq samples
    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(50);
        let upper = max_overall.saturating_add(50);
        render_correlation(
            &mut ctx,
            "cpu freq near clusters",
            &artifacts.cpu_freq_events,
            |s| s.elapsed_ms >= lower && s.elapsed_ms <= upper,
            |cluster, sample| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| sample.elapsed_ms.abs_diff(elapsed) <= 50)
            },
            |output, rank, _, matches| {
                let max_freq = matches.iter().map(|s| s.freq_khz).max().unwrap_or(0);
                pushln(
                    output,
                    format!(
                        "cluster=#{} cpu_freq_samples={} max_freq_khz={}",
                        rank + 1,
                        matches.len(),
                        max_freq
                    ),
                );
            },
        );
    }

    // 6. Block I/O events
    let min_overall = clusters
        .iter()
        .map(|c| c.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|c| c.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    let io_title = if block_io_correlation_basis == "dev+sector" {
        "block i/o overlap (advisory, approximate; correlated by dev+sector)"
    } else {
        "block i/o overlap (correlated by request-pointer)"
    };

    render_correlation(
        &mut ctx,
        io_title,
        &artifacts.block_io_events,
        |e| {
            e.timestamp_ns >= min_overall
                && e.timestamp_ns.saturating_sub(e.duration_ns) <= max_overall
        },
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.timestamp_ns >= min_ns
                && event.timestamp_ns.saturating_sub(event.duration_ns) <= max_ns
        },
        |output, rank, cluster, matches| {
            let max_duration = matches.iter().map(|e| e.duration_ns).max().unwrap_or(0);
            let tids = matches
                .iter()
                .map(|e| e.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            pushln(
                output,
                format!(
                    "cluster=#{} matches={} tids={}{} max_duration={} window_ns={}..{}",
                    rank + 1,
                    matches.len(),
                    tids,
                    if block_io_correlation_basis == "dev+sector" {
                        " (approximate)"
                    } else {
                        ""
                    },
                    format_latency(max_duration),
                    min_ns,
                    max_ns
                ),
            );
        },
    );

    // 7. SCX transitions
    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(2000);
        let upper = max_overall.saturating_add(2000);
        render_correlation(
            &mut ctx,
            "scx transitions near clusters",
            &artifacts.scx_events,
            |e| e.elapsed_ms >= lower && e.elapsed_ms <= upper,
            |cluster, event| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| event.elapsed_ms.abs_diff(elapsed) <= 2000)
            },
            |output, rank, cluster, matches| {
                let elapsed = cluster_elapsed(cluster).unwrap();
                for event in matches {
                    pushln(
                        output,
                        format!(
                            "cluster=#{} SCX transition near spike: ops={} state={} at elapsed={}ms (cluster_elapsed={}ms)",
                            rank + 1,
                            event.ops.as_deref().unwrap_or("-"),
                            event.state.as_deref().unwrap_or("-"),
                            event.elapsed_ms,
                            elapsed
                        ),
                    );
                }
            },
        );
    }
}

fn cluster_elapsed_range(cluster: &SpikeCluster) -> Option<(u128, u128)> {
    let mut elapsed = cluster.points.iter().filter_map(|point| point.elapsed_ms);
    let first = elapsed.next()?;
    let mut min_elapsed = first;
    let mut max_elapsed = first;
    for value in elapsed {
        min_elapsed = min_elapsed.min(value);
        max_elapsed = max_elapsed.max(value);
    }
    Some((min_elapsed, max_elapsed))
}

fn format_option<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn render_cluster(rank: usize, cluster: &SpikeCluster) -> String {
    let labels = cluster_labels(cluster);
    let labels = if labels.is_empty() {
        "-".to_owned()
    } else {
        labels.join(",")
    };
    let span_ns = cluster.max_switch_ns.saturating_sub(cluster.min_switch_ns);
    let elapsed = cluster_elapsed(cluster);
    let cpu_list = cluster
        .points
        .iter()
        .map(|point| point.cpu)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|cpu| cpu.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let shown_points = cluster.points.len().min(MAX_INLINE_CLUSTER_POINTS);
    let omitted_points = cluster.points.len().saturating_sub(shown_points);
    let points = cluster
        .points
        .iter()
        .take(MAX_INLINE_CLUSTER_POINTS)
        .map(render_cluster_point)
        .collect::<Vec<_>>()
        .join(" ");

    let diagnosis_line = if let Some(d) = &cluster.diagnosis {
        let evidence = d.evidence.join("; ");
        let secondary = if d.secondary_causes.is_empty() {
            String::new()
        } else {
            format!(" secondary={:?}", d.secondary_causes)
        };
        format!(
            "\n  diagnosis: primary={:?} confidence={:?}{} evidence=[{}]",
            d.cause, d.confidence, secondary, evidence
        )
    } else {
        String::new()
    };

    format!(
        "#{rank} elapsed={} span={} tasks={} total_spikes={} shown_points={} omitted_points={} cpus={} labels={} max={} switch_ns={}..{} points={}{}",
        format_elapsed(elapsed),
        format_latency(span_ns),
        cluster.distinct_tasks,
        cluster.points.len(),
        shown_points,
        omitted_points,
        cpu_list,
        labels,
        format_latency(cluster.max_latency_ns),
        cluster.min_switch_ns,
        cluster.max_switch_ns,
        points,
        diagnosis_line
    )
}

fn render_cluster_point(point: &SpikePoint) -> String {
    let scx = if let Some(ops) = &point.scx_ops {
        format!(" scx_ops={ops}")
    } else {
        String::new()
    };
    format!(
        "{}({:?}:{} cpu={} wakeup_target_cpu={} latency={} switch_ns={} process_pid={} wakeup_ns={} target_pending_on_switch_cpu={}{})",
        point.task,
        point.class,
        point.comm,
        point.cpu,
        point.wakeup_target_cpu,
        format_latency(point.latency_ns),
        point.switch_ns,
        format_process_pid(point.process_pid),
        point.wakeup_ns,
        point.target_pending_wakeups,
        scx
    )
}

fn format_process_pid(process_pid: Option<u32>) -> String {
    match process_pid {
        Some(process_pid) => process_pid.to_string(),
        None => "-".to_owned(),
    }
}

fn cluster_elapsed(cluster: &SpikeCluster) -> Option<u128> {
    cluster
        .points
        .iter()
        .filter_map(|point| point.elapsed_ms)
        .min()
}

fn format_elapsed(elapsed_ms: Option<u128>) -> String {
    match elapsed_ms {
        Some(elapsed_ms) => format!("{elapsed_ms}ms"),
        None => "-".to_owned(),
    }
}

fn cluster_labels(cluster: &SpikeCluster) -> Vec<&'static str> {
    let mut labels = Vec::new();

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "RenderThread" || point.comm == "Main")
    {
        labels.push("render-main");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm.starts_with("dxvk-"))
    {
        labels.push("dxvk");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "wineserver" || point.comm.contains("winedevice"))
    {
        labels.push("wine");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "AudioThread")
    {
        labels.push("audio");
    }

    labels
}

fn percentile_warning_note(percentile_scope: &str) -> &'static str {
    match percentile_scope {
        "histogram" => {
            "p95/p99 are approximate histogram estimates across the full session; max and threshold counters are exact"
        }
        "capped_prefix" | "capped" => {
            "p95/p99 are capped prefix estimates; prefer max and over_1ms/over_2ms/over_5ms"
        }
        _ => {
            "p95/p99 may be capped because this session predates histogram percentiles; prefer max and threshold counters"
        }
    }
}

// load_all_frame_events removed

fn calculate_median_frametime(frames: &[FrameEvent]) -> f64 {
    if frames.is_empty() {
        return 0.0;
    }
    let mut times: Vec<_> = frames.iter().map(|f| f.frametime_ms).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    if times.len() % 2 == 0 {
        (times[mid - 1] + times[mid]) / 2.0
    } else {
        times[mid]
    }
}

fn identify_frame_spikes(frames: &[FrameEvent], median: f64) -> Vec<FrameEvent> {
    let threshold = if median.is_finite() && median > 0.0 {
        (1.5 * median).min(33.3)
    } else {
        33.3
    };

    frames
        .iter()
        .filter(|f| f.frametime_ms.is_finite() && f.frametime_ms > threshold)
        .cloned()
        .collect()
}

fn perform_frame_diagnosis(
    session: &SessionFile,
    frame_spikes: &[FrameEvent],
    all_spike_points: &[SpikePoint],
    artifacts: &session_io::RunArtifacts,
    cluster_window_ns: u64,
) -> Vec<FrameDiagnosis> {
    let mut diagnoses = Vec::new();
    for frame in frame_spikes {
        let frame_monotonic_ns = if let Some(start_ns) = session.monotonic_start_ns {
            start_ns + (frame.elapsed_ms as u64 * 1_000_000)
        } else {
            continue;
        };

        let nearby_points: Vec<_> = all_spike_points
            .iter()
            .filter(|p| p.switch_ns.abs_diff(frame_monotonic_ns) <= cluster_window_ns)
            .cloned()
            .collect();

        let distinct_tasks = nearby_points
            .iter()
            .map(|p| p.task)
            .collect::<BTreeSet<_>>()
            .len();
        let cluster = cluster_from_points(nearby_points, distinct_tasks);

        // Let `diagnose_cluster` handle artifact filtering.

        let diagnosis = diagnose_cluster(&cluster, artifacts, cluster_window_ns);

        diagnoses.push(FrameDiagnosis {
            frame_elapsed_ms: frame.elapsed_ms,
            frametime_ms: frame.frametime_ms,
            diagnosis,
        });
    }

    diagnoses
}

fn render_frame_diagnosis(rank: usize, diag: &FrameDiagnosis) -> String {
    let mut evidence = diag.diagnosis.evidence.join(", ");
    if !diag.diagnosis.secondary_causes.is_empty() {
        let secondary: Vec<_> = diag
            .diagnosis
            .secondary_causes
            .iter()
            .map(|c| format!("{:?}", c))
            .collect();
        evidence.push_str(&format!(" (secondary: {})", secondary.join(", ")));
    }

    format!(
        "{}. elapsed={}ms frametime={:.1}ms cause={:?} confidence={:?} evidence=\"{}\"",
        rank,
        diag.frame_elapsed_ms,
        diag.frametime_ms,
        diag.diagnosis.cause,
        diag.diagnosis.confidence,
        evidence
    )
}

fn compute_correlation_windows(
    session: &SessionFile,
    clusters: &[SpikeCluster],
    frame_spikes: &[FrameEvent],
    cluster_window_ns: u64,
) -> session_io::CorrelationWindows {
    let mut windows = session_io::CorrelationWindows::default();
    let padding_ms = u128::from(cluster_window_ns / 1_000_000).max(1);

    for cluster in clusters {
        let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
        let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
        windows.windows_ns.push((min_ns, max_ns));

        if let Some((min_e, max_e)) = cluster_elapsed_range(cluster) {
            // Padding for SCX (2000ms), CPU freq (50ms), GPU (50ms), intervals (1000ms)
            windows.windows_ms.push((
                min_e.saturating_sub(2000).max(padding_ms),
                max_e.saturating_add(2000).max(padding_ms),
            ));
        }
    }

    for frame in frame_spikes {
        if let Some(start_ns) = session.monotonic_start_ns {
            let frame_ns = start_ns + (frame.elapsed_ms as u64 * 1_000_000);
            windows.windows_ns.push((
                frame_ns.saturating_sub(cluster_window_ns),
                frame_ns.saturating_add(cluster_window_ns),
            ));
        }
        windows.windows_ms.push((
            frame.elapsed_ms.saturating_sub(2000).max(padding_ms),
            frame.elapsed_ms.saturating_add(2000).max(padding_ms),
        ));
    }

    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identify_frame_spikes() {
        let frames = vec![
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 16.0,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 24.1,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 30.0,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: f64::NAN,
            },
        ];

        // median 16.0 => threshold 24.0 (1.5 * 16 = 24.0, which is < 33.3)
        let spikes = identify_frame_spikes(&frames, 16.0);
        assert_eq!(spikes.len(), 2);
        assert_eq!(spikes[0].frametime_ms, 24.1);
        assert_eq!(spikes[1].frametime_ms, 30.0);

        // median 30.0 => threshold 33.3 (1.5 * 30 = 45.0, but capped at 33.3)
        let spikes = identify_frame_spikes(&frames, 30.0);
        assert!(spikes.is_empty());

        // median 0.0 => threshold 33.3
        let spikes = identify_frame_spikes(&frames, 0.0);
        assert!(spikes.is_empty());

        let frames_with_long = vec![FrameEvent {
            elapsed_ms: 0,
            frametime_ms: 33.4,
        }];
        let spikes = identify_frame_spikes(&frames_with_long, 0.0);
        assert_eq!(spikes.len(), 1);
        assert_eq!(spikes[0].frametime_ms, 33.4);
    }
}
