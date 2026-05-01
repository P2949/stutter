use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    fs,
    io::BufRead,
    path::Path,
};

use anyhow::Context;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{
        FrameEvent, GpuSample, IrqEventRecord, RecordedSpike, SESSION_SCHEMA_VERSION, SessionFile,
        SessionTask, SpikeEvent,
    },
};

const MIN_CLUSTER_TASKS: usize = 3;
const MAX_INLINE_CLUSTER_POINTS: usize = 8;
const MAX_CLUSTER_CANDIDATES: usize = 4096;

#[derive(Clone, Serialize)]
pub(crate) struct SpikePoint {
    task: u32,
    class: TaskClass,
    process_pid: Option<u32>,
    comm: String,
    cpu: u32,
    wakeup_target_cpu: u32,
    latency_ns: u64,
    wakeup_ns: u64,
    switch_ns: u64,
    // Diagnostic-only: see docs in `metrics::SpikeRecord`.
    // This is included in reports and visualizations only and MUST NOT
    // influence scoring or tuning decisions.
    target_pending_wakeups: u32,
    elapsed_ms: Option<u128>,
}

#[derive(Clone, Serialize)]
pub(crate) struct SpikeCluster {
    points: Vec<SpikePoint>,
    distinct_tasks: usize,
    min_switch_ns: u64,
    max_switch_ns: u64,
    max_latency_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) enum SpikeClusterSource {
    SpikeEvents,
    TopSpikesFallback,
}

#[derive(Serialize)]
pub(crate) struct SpikeClusterAnalysis {
    source: SpikeClusterSource,
    source_count: usize,
    clusters: Vec<SpikeCluster>,
}

#[derive(Default, Serialize)]
pub(crate) struct RunArtifacts {
    pub(crate) irq_events: Vec<IrqEventRecord>,
    pub(crate) gpu_samples: Vec<GpuSample>,
    pub(crate) frame_events: Vec<FrameEvent>,
    pub(crate) migration_events: Vec<crate::recorder::MigrationEventRecord>,
    pub(crate) cpu_freq_samples: Vec<crate::recorder::CpuFreqRecord>,
    pub(crate) io_events: Vec<crate::recorder::BlockIoRecord>,
}

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
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let session_path = if path.is_dir() {
        path.join("session.json")
    } else {
        path.to_path_buf()
    };

    let file = fs::File::open(&session_path)
        .with_context(|| format!("failed to open {}", session_path.display()))?;
    let reader = std::io::BufReader::new(file);
    let session: SessionFile = serde_json::from_reader(reader)
        .with_context(|| format!("failed to parse {}", session_path.display()))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&session)?);
        return Ok(());
    }

    let spike_events = load_spike_events(&session_path)?;
    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);
    let cluster_analysis =
        spike_cluster_analysis(&session, spike_events.as_deref(), cluster_window_ns, top, filter_class);
    let artifacts = load_run_artifacts(&session_path, &cluster_analysis.clusters, cluster_window_ns)?;

    print!(
        "{}",
        render_report(
            &session_path,
            &session,
            &cluster_analysis,
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
    let load_session = |path: &Path| -> anyhow::Result<SessionFile> {
        let session_path = if path.is_dir() {
            path.join("session.json")
        } else {
            path.to_path_buf()
        };
        let file = fs::File::open(&session_path)
            .with_context(|| format!("failed to open {}", session_path.display()))?;
        let reader = std::io::BufReader::new(file);
        serde_json::from_reader(reader)
            .with_context(|| format!("failed to parse {}", session_path.display()))
    };

    let session_a = load_session(path_a)?;
    let session_b = load_session(path_b)?;

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
    let session_path = if path.is_dir() {
        path.join("session.json")
    } else {
        path.to_path_buf()
    };
    let file = fs::File::open(&session_path)
        .with_context(|| format!("failed to open {}", session_path.display()))?;
    let reader = std::io::BufReader::new(file);
    let session: SessionFile = serde_json::from_reader(reader)
        .with_context(|| format!("failed to parse {}", session_path.display()))?;

    let spike_events = load_spike_events(&session_path)?;
    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);
    let cluster_analysis =
        spike_cluster_analysis(&session, spike_events.as_deref(), cluster_window_ns, top, filter_class);
    let artifacts = load_run_artifacts(&session_path, &cluster_analysis.clusters, cluster_window_ns)?;

    let text_report = render_report(
        &session_path,
        &session,
        &cluster_analysis,
        &artifacts,
        top,
        cluster_window_ms,
        filter_class,
    );
    let html = render_html_report(
        &session,
        spike_events.as_deref(),
        &artifacts,
        &cluster_analysis,
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
    artifacts: &RunArtifacts,
    cluster_analysis: &SpikeClusterAnalysis,
    text_report: &str,
    top: usize,
) -> String {
    let session_json = serde_json::to_string(session).unwrap_or_else(|_| "{}".to_owned());
    let spike_events_json =
        serde_json::to_string(&spike_events).unwrap_or_else(|_| "null".to_owned());
    let artifacts_json = serde_json::to_string(artifacts).unwrap_or_else(|_| "{}".to_owned());
    let cluster_analysis_json =
        serde_json::to_string(&cluster_analysis).unwrap_or_else(|_| "{}".to_owned());

    let template = include_str!("report_template.html");

    template
        .replace("{text_report}", &html_escape(text_report))
        .replace("{session_json}", &session_json)
        .replace("{spike_events_json}", &spike_events_json)
        .replace("{artifacts_json}", &artifacts_json)
        .replace("{cluster_analysis_json}", &cluster_analysis_json)
        .replace("{top}", &top.to_string())
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn render_report(
    session_path: &Path,
    session: &SessionFile,
    cluster_analysis: &SpikeClusterAnalysis,
    artifacts: &RunArtifacts,
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
                if block_io_correlation_basis(session) == "dev+sector" { " correlated; approximate" } else { " correlated" },
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

    render_correlation_sections(
        &mut output,
        &cluster_analysis.clusters,
        artifacts,
        block_io_correlation_basis(session),
        cluster_window_ns,
        top,
        session_path.parent(),
    );

    output
}

fn load_spike_events(session_path: &Path) -> anyhow::Result<Option<Vec<SpikeEvent>>> {
    let Some(run_dir) = session_path.parent() else {
        return Ok(None);
    };
    let spike_events_path = run_dir.join("spike_events.json");
    if !spike_events_path.exists() {
        return Ok(None);
    }

    let file = fs::File::open(&spike_events_path)
        .with_context(|| format!("failed to open {}", spike_events_path.display()))?;
    let reader = std::io::BufReader::new(file);
    let events = serde_json::from_reader(reader)
        .with_context(|| format!("failed to parse {}", spike_events_path.display()))?;
    Ok(Some(events))
}

fn load_run_artifacts(
    session_path: &Path,
    clusters: &[SpikeCluster],
    cluster_window_ns: u64,
) -> anyhow::Result<RunArtifacts> {
    let Some(run_dir) = session_path.parent() else {
        return Ok(RunArtifacts::default());
    };

    if clusters.is_empty() {
        return Ok(RunArtifacts::default());
    }

    let mut artifacts = RunArtifacts::default();

    // IRQ events
    let path = run_dir.join("irq_events.json");
    if path.exists() {
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
        if let Ok(selected) = stream_json_array_select(&path, |e: &IrqEventRecord| {
            e.exit_ns >= min_overall && e.enter_ns <= max_overall
        }) {
            artifacts.irq_events = selected;
        }
    }

    // GPU samples
    let path = run_dir.join("gpu_samples.json");
    if path.exists() {
        let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
        let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
        if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
            let lower = min_overall.saturating_sub(50);
            let upper = max_overall.saturating_add(50);
            if let Ok(selected) = stream_json_array_select(&path, |s: &GpuSample| {
                s.elapsed_ms >= lower && s.elapsed_ms <= upper
            }) {
                artifacts.gpu_samples = selected;
            }
        }
    }

    // Frame events
    let path = run_dir.join("frame_correlation.json");
    if path.exists() {
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
            if let Ok(selected) = stream_json_array_select(&path, |f: &FrameEvent| {
                f.elapsed_ms >= lower && f.elapsed_ms <= upper
            }) {
                artifacts.frame_events = selected;
            }
        }
    }

    // Migration events
    let path = run_dir.join("migration_events.json");
    if path.exists() {
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
        if let Ok(selected) = stream_json_array_select(&path, |e: &crate::recorder::MigrationEventRecord| {
            e.timestamp_ns >= min_overall && e.timestamp_ns <= max_overall
        }) {
            artifacts.migration_events = selected;
        }
    }

    // CPU frequency samples
    let path = run_dir.join("cpu_freq_samples.json");
    if path.exists() {
        let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
        let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
        if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
            let lower = min_overall.saturating_sub(50);
            let upper = max_overall.saturating_add(50);
            if let Ok(selected) = stream_json_array_select(&path, |s: &crate::recorder::CpuFreqRecord| {
                s.elapsed_ms >= lower && s.elapsed_ms <= upper
            }) {
                artifacts.cpu_freq_samples = selected;
            }
        }
    }

    // I/O events
    let path = run_dir.join("io_events.json");
    if path.exists() {
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
        if let Ok(selected) = stream_json_array_select(&path, |e: &crate::recorder::BlockIoRecord| {
            e.timestamp_ns >= min_overall
                && e.timestamp_ns.saturating_sub(e.duration_ns) <= max_overall
        }) {
            artifacts.io_events = selected;
        }
    }

    Ok(artifacts)
}

// Stream elements from a JSON array file without materializing the whole
// array. The function parses the array start token and then repeatedly
// deserializes elements using `serde_json::from_reader`, skipping separators
// (commas) until the array end is reached.
fn stream_json_array_select<T, P>(path: &Path, mut predicate: P) -> anyhow::Result<Vec<T>>
where
    T: DeserializeOwned,
    P: FnMut(&T) -> bool,
{
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let mut reader = std::io::BufReader::new(file);

    // Consume until we find the array start '['
    loop {
        let (consume_amount, found) = {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                return Ok(Vec::new());
            }
            let mut amount = buf.len();
            let mut found = false;
            for (i, &b) in buf.iter().enumerate() {
                if !(b as char).is_whitespace() {
                    if b == b'[' {
                        amount = i + 1;
                        found = true;
                    } else {
                        anyhow::bail!("expected JSON array in {}", path.display());
                    }
                    break;
                }
            }
            (amount, found)
        };
        reader.consume(consume_amount);
        if found {
            break;
        }
    }

    let mut matches = Vec::new();

    loop {
        // Skip whitespace and commas, detect end of array
        let (consume_amount, is_end_array, is_comma) = {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            let mut pos = 0;
            while pos < buf.len() && (buf[pos] as char).is_whitespace() {
                pos += 1;
            }
            if pos >= buf.len() {
                (buf.len(), false, false)
            } else {
                let b = buf[pos];
                if b == b']' {
                    (pos + 1, true, false)
                } else if b == b',' {
                    (pos + 1, false, true)
                } else {
                    (0, false, false)
                }
            }
        };

        reader.consume(consume_amount);

        if is_end_array {
            break;
        }
        if is_comma || consume_amount > 0 {
            continue;
        }

        // Parse next element from the reader
        let val: T = serde_json::from_reader(&mut reader)
            .with_context(|| format!("failed to parse element in {}", path.display()))?;
        if predicate(&val) {
            matches.push(val);
        }
    }

    Ok(matches)
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

fn render_correlation_sections(
    output: &mut String,
    clusters: &[SpikeCluster],
    artifacts: &RunArtifacts,
    block_io_correlation_basis: &str,
    cluster_window_ns: u64,
    top: usize,
    run_dir: Option<&Path>,
) {
    // Try to use in-memory artifacts if present; otherwise stream from
    // the run directory artifact files constrained to the union of cluster
    // windows to avoid loading entire files.
    let mut irq_pool: Option<Vec<IrqEventRecord>> = None;
    if !artifacts.irq_events.is_empty() {
        irq_pool = Some(artifacts.irq_events.clone());
    } else if let Some(run_dir) = run_dir {
        let path = run_dir.join("irq_events.json");
        if path.exists() && !clusters.is_empty() {
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
            if let Ok(selected) = stream_json_array_select(&path, |e: &IrqEventRecord| {
                e.exit_ns >= min_overall && e.enter_ns <= max_overall
            }) {
                irq_pool = Some(selected);
            }
        }
    }

    if irq_pool.as_ref().is_some_and(|v| !v.is_empty()) {
        pushln(output, "irq overlap");
        pushln(output, "-----------");
        for (rank, cluster) in clusters.iter().take(top).enumerate() {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            let matches = irq_pool
                .as_ref()
                .unwrap()
                .iter()
                .filter(|event| event.exit_ns >= min_ns && event.enter_ns <= max_ns)
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            let max_duration = matches
                .iter()
                .map(|event| event.duration_ns)
                .max()
                .unwrap_or(0);
            let irq_list = matches
                .iter()
                .map(|event| event.irq)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|irq| irq.to_string())
                .collect::<Vec<_>>()
                .join(",");
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
        }
        pushln(output, "");
    }

    // GPU samples: either use in-memory samples or stream a narrow window
    // around cluster elapsed times.
    let mut gpu_pool: Option<Vec<GpuSample>> = None;
    if !artifacts.gpu_samples.is_empty() {
        gpu_pool = Some(artifacts.gpu_samples.clone());
    } else if let Some(run_dir) = run_dir {
        let path = run_dir.join("gpu_samples.json");
        if path.exists() && !clusters.is_empty() {
            // compute overall elapsed window (add 50ms tolerance)
            let min_overall_opt = clusters
                .iter()
                .filter_map(cluster_elapsed)
                .min();
            let max_overall_opt = clusters
                .iter()
                .filter_map(cluster_elapsed)
                .max();
            if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
                let lower = min_overall.saturating_sub(50);
                let upper = max_overall.saturating_add(50);
                if let Ok(selected) = stream_json_array_select(&path, |s: &GpuSample| {
                    s.elapsed_ms >= lower && s.elapsed_ms <= upper
                }) {
                    gpu_pool = Some(selected);
                }
            }
        }
    }

    if gpu_pool.as_ref().is_some_and(|v| !v.is_empty()) {
        pushln(output, "gpu near clusters");
        pushln(output, "-----------------");
        for (rank, cluster) in clusters.iter().take(top).enumerate() {
            let Some(elapsed) = cluster_elapsed(cluster) else { continue; };
            let Some(sample) = nearest_gpu_sample(elapsed, gpu_pool.as_ref().unwrap()) else { continue; };
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
        }
        pushln(output, "");
    }

    // Frame events: stream a bounded elapsed window if needed.
    let mut frame_pool: Option<Vec<FrameEvent>> = None;
    if !artifacts.frame_events.is_empty() {
        frame_pool = Some(artifacts.frame_events.clone());
    } else if let Some(run_dir) = run_dir {
        let path = run_dir.join("frame_correlation.json");
        if path.exists() && !clusters.is_empty() {
            // compute overall min/max elapsed with padding
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
                if let Ok(selected) = stream_json_array_select(&path, |f: &FrameEvent| {
                    f.elapsed_ms >= lower && f.elapsed_ms <= upper
                }) {
                    frame_pool = Some(selected);
                }
            }
        }
    }

    if frame_pool.as_ref().is_some_and(|v| !v.is_empty()) {
        pushln(output, "frame overlap");
        pushln(output, "-------------");
        for (rank, cluster) in clusters.iter().take(top).enumerate() {
            let Some((min_elapsed, max_elapsed)) = cluster_elapsed_range(cluster) else { continue; };
            let padding_ms = u128::from(cluster_window_ns / 1_000_000).max(1);
            let min_elapsed = min_elapsed.saturating_sub(padding_ms);
            let max_elapsed = max_elapsed.saturating_add(padding_ms);
            let matches = frame_pool
                .as_ref()
                .map(|pool| {
                    pool.iter()
                        .filter(|frame| frame.elapsed_ms >= min_elapsed && frame.elapsed_ms <= max_elapsed)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if matches.is_empty() {
                continue;
            }
            let max_frame = matches
                .iter()
                .map(|frame| frame.frametime_ms)
                .fold(0.0_f64, f64::max);
            pushln(
                output,
                format!(
                    "cluster=#{} frames={} max_frametime_ms={:.3} elapsed={}..{}",
                    rank + 1,
                    matches.len(),
                    max_frame,
                    min_elapsed,
                    max_elapsed
                ),
            );
        }
        pushln(output, "");
    }

    let mut migration_pool: Option<Vec<crate::recorder::MigrationEventRecord>> = None;
    if !artifacts.migration_events.is_empty() {
        migration_pool = Some(artifacts.migration_events.clone());
    } else if let Some(run_dir) = run_dir {
        let path = run_dir.join("migration_events.json");
        if path.exists() && !clusters.is_empty() {
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
            if let Ok(selected) = stream_json_array_select(&path, |e: &crate::recorder::MigrationEventRecord| {
                e.timestamp_ns >= min_overall && e.timestamp_ns <= max_overall
            }) {
                migration_pool = Some(selected);
            }
        }
    }

    if migration_pool.as_ref().is_some_and(|v| !v.is_empty()) {
        pushln(output, "migration overlap");
        pushln(output, "-----------------");
        for (rank, cluster) in clusters.iter().take(top).enumerate() {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            let matches = migration_pool
                .as_ref()
                .unwrap()
                .iter()
                .filter(|event| event.timestamp_ns >= min_ns && event.timestamp_ns <= max_ns)
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            let tids = matches
                .iter()
                .map(|event| event.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
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
        }
        pushln(output, "");
    }

    let mut cpu_freq_pool: Option<Vec<crate::recorder::CpuFreqRecord>> = None;
    if !artifacts.cpu_freq_samples.is_empty() {
        cpu_freq_pool = Some(artifacts.cpu_freq_samples.clone());
    } else if let Some(run_dir) = run_dir {
        let path = run_dir.join("cpu_freq_samples.json");
        if path.exists() && !clusters.is_empty() {
            let min_overall_opt = clusters
                .iter()
                .filter_map(cluster_elapsed)
                .min();
            let max_overall_opt = clusters
                .iter()
                .filter_map(cluster_elapsed)
                .max();
            if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
                let lower = min_overall.saturating_sub(50);
                let upper = max_overall.saturating_add(50);
                if let Ok(selected) = stream_json_array_select(&path, |s: &crate::recorder::CpuFreqRecord| {
                    s.elapsed_ms >= lower && s.elapsed_ms <= upper
                }) {
                    cpu_freq_pool = Some(selected);
                }
            }
        }
    }

    if cpu_freq_pool.as_ref().is_some_and(|v| !v.is_empty()) {
        pushln(output, "cpu freq near clusters");
        pushln(output, "----------------------");
        for (rank, cluster) in clusters.iter().take(top).enumerate() {
            let Some(elapsed) = cluster_elapsed(cluster) else {
                continue;
            };
            let matches = cpu_freq_pool
                .as_ref()
                .unwrap()
                .iter()
                .filter(|sample| sample.elapsed_ms.abs_diff(elapsed) <= 50)
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            let max_freq = matches
                .iter()
                .map(|sample| sample.freq_khz)
                .max()
                .unwrap_or(0);
            pushln(
                output,
                format!(
                    "cluster=#{} cpu_freq_samples={} max_freq_khz={}",
                    rank + 1,
                    matches.len(),
                    max_freq
                ),
            );
        }
        pushln(output, "");
    }

    let mut io_pool: Option<Vec<crate::recorder::BlockIoRecord>> = None;
    if !artifacts.io_events.is_empty() {
        io_pool = Some(artifacts.io_events.clone());
    } else if let Some(run_dir) = run_dir {
        let path = run_dir.join("io_events.json");
        if path.exists() && !clusters.is_empty() {
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
            if let Ok(selected) = stream_json_array_select(&path, |e: &crate::recorder::BlockIoRecord| {
                e.timestamp_ns >= min_overall
                    && e.timestamp_ns.saturating_sub(e.duration_ns) <= max_overall
            }) {
                io_pool = Some(selected);
            }
        }
    }

    if io_pool.as_ref().is_some_and(|v| !v.is_empty()) {
        let heading = if block_io_correlation_basis == "dev+sector" {
            "block i/o overlap (advisory, approximate; correlated by dev+sector)"
        } else {
            "block i/o overlap (correlated by request-pointer)"
        };
        pushln(output, heading);
        pushln(output, "-------------------------------");
        for (rank, cluster) in clusters.iter().take(top).enumerate() {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            let matches = io_pool
                .as_ref()
                .unwrap()
                .iter()
                .filter(|event| {
                    event.timestamp_ns >= min_ns
                        && event.timestamp_ns.saturating_sub(event.duration_ns) <= max_ns
                })
                .collect::<Vec<_>>();
            if matches.is_empty() {
                continue;
            }
            let max_duration = matches
                .iter()
                .map(|event| event.duration_ns)
                .max()
                .unwrap_or(0);
            let tids = matches
                .iter()
                .map(|event| event.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            pushln(
                output,
                format!(
                    "cluster=#{} matches={} tids={} max_duration={} window_ns={}..{}",
                    rank + 1,
                    matches.len(),
                    tids,
                    format_latency(max_duration),
                    min_ns,
                    max_ns
                ),
            );
        }
        pushln(output, "");
    }
}

fn nearest_gpu_sample(elapsed_ms: u128, samples: &[GpuSample]) -> Option<&GpuSample> {
    samples
        .iter()
        .min_by_key(|sample| sample.elapsed_ms.abs_diff(elapsed_ms))
        .filter(|sample| sample.elapsed_ms.abs_diff(elapsed_ms) <= 50)
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

    format!(
        "#{rank} elapsed={} span={} tasks={} spikes={} total_spikes={} shown_points={} omitted_points={} cpus={} labels={} max={} switch_ns={}..{} points={}",
        format_elapsed(elapsed),
        format_latency(span_ns),
        cluster.distinct_tasks,
        cluster.points.len(),
        cluster.points.len(),
        shown_points,
        omitted_points,
        cpu_list,
        labels,
        format_latency(cluster.max_latency_ns),
        cluster.min_switch_ns,
        cluster.max_switch_ns,
        points
    )
}

fn render_cluster_point(point: &SpikePoint) -> String {
    format!(
        "{}({:?}:{} cpu={} wakeup_target_cpu={} latency={} switch_ns={} process_pid={} wakeup_ns={} target_pending_on_switch_cpu={})",
        point.task,
        point.class,
        point.comm,
        point.cpu,
        point.wakeup_target_cpu,
        format_latency(point.latency_ns),
        point.switch_ns,
        format_process_pid(point.process_pid),
        point.wakeup_ns,
        point.target_pending_wakeups
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
