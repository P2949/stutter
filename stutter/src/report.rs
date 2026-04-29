use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::Context;

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{RecordedSpike, SESSION_SCHEMA_VERSION, SessionFile, SessionTask, SpikeEvent},
};

const MIN_CLUSTER_TASKS: usize = 3;
const MAX_INLINE_CLUSTER_POINTS: usize = 8;
const MAX_CLUSTER_CANDIDATES: usize = 4096;

#[derive(Clone)]
struct SpikePoint {
    task: u32,
    class: TaskClass,
    process_pid: Option<u32>,
    comm: String,
    cpu: u32,
    latency_ns: u64,
    wakeup_ns: u64,
    switch_ns: u64,
    elapsed_ms: Option<u128>,
}

#[derive(Clone)]
struct SpikeCluster {
    points: Vec<SpikePoint>,
    distinct_tasks: usize,
    min_switch_ns: u64,
    max_switch_ns: u64,
    max_latency_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpikeClusterSource {
    SpikeEvents,
    TopSpikesFallback,
}

struct SpikeClusterAnalysis {
    source: SpikeClusterSource,
    source_count: usize,
    clusters: Vec<SpikeCluster>,
}

#[derive(Clone)]
struct SpikeClusterCandidate {
    start_idx: usize,
    end_idx: usize,
    distinct_tasks: usize,
    min_switch_ns: u64,
    max_switch_ns: u64,
    max_latency_ns: u64,
}

pub fn print_report(
    path: &Path,
    json: bool,
    top: usize,
    cluster_window_ms: u64,
) -> anyhow::Result<()> {
    let session_path = if path.is_dir() {
        path.join("session.json")
    } else {
        path.to_path_buf()
    };

    let data = fs::read_to_string(&session_path)?;
    let session: SessionFile = serde_json::from_str(&data)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&session)?);
        return Ok(());
    }

    let spike_events = load_spike_events(&session_path)?;

    print!(
        "{}",
        render_report(
            &session_path,
            &session,
            spike_events.as_deref(),
            top,
            cluster_window_ms
        )
    );

    Ok(())
}

pub(crate) fn render_report(
    session_path: &Path,
    session: &SessionFile,
    spike_events: Option<&[SpikeEvent]>,
    top: usize,
    cluster_window_ms: u64,
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
                session.spike_event_count
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
        .collect::<Vec<_>>();

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));

    pushln(&mut output, "top tasks by max latency");
    pushln(&mut output, "------------------------");
    for task in tasks.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} process_pid={:?} samples={} max={} over_1ms={} over_2ms={} over_5ms={} percentile_scope={}",
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
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} over_5ms={} over_2ms={} over_1ms={} max={}",
                task.task,
                task.active,
                task.class,
                task.comm,
                task.latency.over_5ms,
                task.latency.over_2ms,
                task.latency.over_1ms,
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
                "task={} active={} class={:?} comm={} cpu={} latency={} wakeup_ns={} switch_ns={}",
                spike.task,
                spike.active,
                spike.class,
                spike.comm,
                spike.cpu,
                format_latency(spike.latency_ns),
                spike.wakeup_ns,
                spike.switch_ns,
            ),
        );
    }
    pushln(&mut output, "");

    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);
    let cluster_analysis = spike_cluster_analysis(session, spike_events, cluster_window_ns, top);

    pushln(&mut output, "spike clusters");
    pushln(&mut output, "--------------");
    pushln(
        &mut output,
        render_cluster_source(&cluster_analysis, cluster_window_ms),
    );
    if cluster_analysis.clusters.is_empty() {
        pushln(
            &mut output,
            format!(
                "none min_tasks={} window_ms={}",
                MIN_CLUSTER_TASKS, cluster_window_ms
            ),
        );
        return output;
    }

    for (idx, cluster) in cluster_analysis.clusters.iter().take(top).enumerate() {
        pushln(&mut output, render_cluster(idx + 1, cluster));
    }

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

    let data = fs::read_to_string(&spike_events_path)
        .with_context(|| format!("failed to read {}", spike_events_path.display()))?;
    let events = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse {}", spike_events_path.display()))?;
    Ok(Some(events))
}

fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

fn spike_cluster_analysis(
    session: &SessionFile,
    spike_events: Option<&[SpikeEvent]>,
    cluster_window_ns: u64,
    top: usize,
) -> SpikeClusterAnalysis {
    let (source, points) = match spike_events {
        Some(spike_events) => (
            SpikeClusterSource::SpikeEvents,
            flatten_spike_events(session, spike_events),
        ),
        None => (
            SpikeClusterSource::TopSpikesFallback,
            flatten_top_spikes(session),
        ),
    };
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

    let mut candidates = Vec::new();
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
            retain_cluster_candidate(
                &mut candidates,
                SpikeClusterCandidate {
                    start_idx: left_idx,
                    end_idx: right_idx + 1,
                    distinct_tasks: task_counts.len(),
                    min_switch_ns: points[left_idx].switch_ns,
                    max_switch_ns: points[right_idx].switch_ns,
                    max_latency_ns,
                },
            );
        }
    }

    candidates.sort_by_key(|candidate| {
        (
            std::cmp::Reverse(candidate.distinct_tasks),
            std::cmp::Reverse(candidate.max_latency_ns),
            std::cmp::Reverse(candidate.end_idx.saturating_sub(candidate.start_idx)),
            candidate.min_switch_ns,
        )
    });

    let mut selected_candidates = Vec::new();
    let max_selected = top.saturating_mul(4).min(MAX_CLUSTER_CANDIDATES); // Cap it. Using MAX_CLUSTER_CANDIDATES as upper bound if top is small.

    'candidate: for candidate in candidates {
        for existing in &selected_candidates {
            if cluster_candidates_overlap(existing, &candidate) {
                continue 'candidate;
            }
        }
        selected_candidates.push(candidate);
        if selected_candidates.len() >= max_selected {
            break;
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
            latency_ns: spike.latency_ns,
            wakeup_ns: spike.wakeup_ns,
            switch_ns: spike.switch_ns,
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
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
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

fn retain_cluster_candidate(
    candidates: &mut Vec<SpikeClusterCandidate>,
    candidate: SpikeClusterCandidate,
) {
    if candidates.len() < MAX_CLUSTER_CANDIDATES {
        candidates.push(candidate);
        return;
    }

    let Some((worst_idx, worst_candidate)) = candidates
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| compare_cluster_candidates(left, right))
    else {
        return;
    };

    if compare_cluster_candidates(&candidate, worst_candidate).is_gt() {
        candidates[worst_idx] = candidate;
    }
}

fn compare_cluster_candidates(
    left: &SpikeClusterCandidate,
    right: &SpikeClusterCandidate,
) -> std::cmp::Ordering {
    (
        left.distinct_tasks,
        left.max_latency_ns,
        left.end_idx.saturating_sub(left.start_idx),
        std::cmp::Reverse(left.min_switch_ns),
    )
        .cmp(&(
            right.distinct_tasks,
            right.max_latency_ns,
            right.end_idx.saturating_sub(right.start_idx),
            std::cmp::Reverse(right.min_switch_ns),
        ))
}

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

fn cluster_candidates_overlap(left: &SpikeClusterCandidate, right: &SpikeClusterCandidate) -> bool {
    left.min_switch_ns <= right.max_switch_ns && right.min_switch_ns <= left.max_switch_ns
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
        "{}({:?}:{} cpu={} latency={} switch_ns={} process_pid={} wakeup_ns={})",
        point.task,
        point.class,
        point.comm,
        point.cpu,
        format_latency(point.latency_ns),
        point.switch_ns,
        format_process_pid(point.process_pid),
        point.wakeup_ns
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
