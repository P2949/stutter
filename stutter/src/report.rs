use std::{
    collections::{BTreeSet, HashSet},
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

fn render_report(
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
        format!("active_tasks_at_end: {}", session.active_target_pids_count),
    );
    pushln(&mut output, "");

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
    let cluster_analysis = spike_cluster_analysis(session, spike_events, cluster_window_ns);

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
) -> SpikeClusterAnalysis {
    let (source, points) = match spike_events {
        Some(spike_events) => (
            SpikeClusterSource::SpikeEvents,
            flatten_spike_events(spike_events),
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
        clusters: spike_clusters_from_points(points, cluster_window_ns),
    }
}

#[cfg(test)]
fn spike_clusters(session: &SessionFile, cluster_window_ns: u64) -> Vec<SpikeCluster> {
    spike_clusters_from_points(flatten_top_spikes(session), cluster_window_ns)
}

fn spike_clusters_from_points(
    mut points: Vec<SpikePoint>,
    cluster_window_ns: u64,
) -> Vec<SpikeCluster> {
    points.sort_by_key(|point| point.switch_ns);

    let mut candidates = Vec::new();

    for start_idx in 0..points.len() {
        let start_ns = points[start_idx].switch_ns;
        let max_ns = start_ns.saturating_add(cluster_window_ns);
        let mut end_idx = start_idx;

        while end_idx < points.len() && points[end_idx].switch_ns <= max_ns {
            end_idx += 1;
        }

        let window = &points[start_idx..end_idx];
        let distinct_tasks = distinct_task_count(window);
        if distinct_tasks < MIN_CLUSTER_TASKS {
            continue;
        }

        candidates.push(cluster_from_points(window.to_vec(), distinct_tasks));
    }

    candidates.sort_by_key(|cluster| {
        (
            std::cmp::Reverse(cluster.distinct_tasks),
            std::cmp::Reverse(cluster.max_latency_ns),
            std::cmp::Reverse(cluster.points.len()),
            cluster.min_switch_ns,
        )
    });

    let mut selected = Vec::new();

    'candidate: for candidate in candidates {
        for existing in &selected {
            if clusters_overlap(existing, &candidate) {
                continue 'candidate;
            }
        }
        selected.push(candidate);
    }

    selected
}

fn flatten_spike_events(spike_events: &[SpikeEvent]) -> Vec<SpikePoint> {
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
            elapsed_ms: Some(spike.elapsed_ms),
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

fn distinct_task_count(points: &[SpikePoint]) -> usize {
    points
        .iter()
        .map(|point| point.task)
        .collect::<HashSet<_>>()
        .len()
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

fn clusters_overlap(left: &SpikeCluster, right: &SpikeCluster) -> bool {
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
    let points = cluster
        .points
        .iter()
        .map(render_cluster_point)
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "#{rank} elapsed={} span={} tasks={} spikes={} cpus={} labels={} max={} switch_ns={}..{} points={}",
        format_elapsed(elapsed),
        format_latency(span_ns),
        cluster.distinct_tasks,
        cluster.points.len(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        metadata::SystemMetadata,
        recorder::{
            RecordedConfig, RecordedCpuSnapshot, RecordedLatency, RecordedTime, SessionSpike,
        },
    };

    #[test]
    fn report_warning_distinguishes_histogram_and_legacy_capped_prefix() {
        assert!(percentile_warning_note("histogram").contains("histogram estimates"));
        assert!(percentile_warning_note("capped_prefix").contains("capped prefix"));
    }

    #[test]
    fn clusters_group_spikes_within_window_and_require_distinct_tasks() {
        let session = session_with_spikes(
            Some(1_000_000_000),
            vec![
                spike_task(
                    10,
                    "RenderThread",
                    TaskClass::Game,
                    1,
                    1_001_000_000,
                    5_000_000,
                ),
                spike_task(11, "dxvk-cs", TaskClass::Game, 2, 1_003_000_000, 2_000_000),
                spike_task(
                    12,
                    "wineserver",
                    TaskClass::WineServer,
                    3,
                    1_005_000_000,
                    1_500_000,
                ),
                spike_task(
                    10,
                    "RenderThread",
                    TaskClass::Game,
                    4,
                    1_020_000_000,
                    7_000_000,
                ),
                spike_task(
                    10,
                    "RenderThread",
                    TaskClass::Game,
                    5,
                    1_021_000_000,
                    3_000_000,
                ),
            ],
        );

        let clusters = spike_clusters(&session, 5_000_000);

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].distinct_tasks, 3);
        assert_eq!(clusters[0].points.len(), 3);
        assert_eq!(clusters[0].max_latency_ns, 5_000_000);
    }

    #[test]
    fn clusters_split_events_outside_window() {
        let session = session_with_spikes(
            None,
            vec![
                spike_task(1, "Main", TaskClass::Game, 0, 10_000_000, 2_000_000),
                spike_task(2, "dxvk-cs", TaskClass::Game, 1, 11_000_000, 1_500_000),
                spike_task(
                    3,
                    "wineserver",
                    TaskClass::WineServer,
                    2,
                    12_000_000,
                    1_250_000,
                ),
                spike_task(4, "AudioThread", TaskClass::Game, 3, 30_000_000, 3_000_000),
                spike_task(5, "dxvk-submit", TaskClass::Game, 4, 31_000_000, 1_750_000),
                spike_task(
                    6,
                    "winedevice.exe",
                    TaskClass::Helper,
                    5,
                    32_000_000,
                    1_500_000,
                ),
            ],
        );

        let clusters = spike_clusters(&session, 5_000_000);

        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].distinct_tasks, 3);
        assert_eq!(clusters[1].distinct_tasks, 3);
        assert!(!clusters_overlap(&clusters[0], &clusters[1]));
    }

    #[test]
    fn clusters_deduplicate_overlapping_candidate_windows() {
        let session = session_with_spikes(
            None,
            vec![
                spike_task(1, "Main", TaskClass::Game, 0, 10_000_000, 5_000_000),
                spike_task(2, "dxvk-cs", TaskClass::Game, 1, 11_000_000, 4_000_000),
                spike_task(
                    3,
                    "wineserver",
                    TaskClass::WineServer,
                    2,
                    12_000_000,
                    3_000_000,
                ),
                spike_task(4, "AudioThread", TaskClass::Game, 3, 13_000_000, 2_000_000),
            ],
        );

        let clusters = spike_clusters(&session, 5_000_000);

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].distinct_tasks, 4);
        assert_eq!(clusters[0].points.len(), 4);
    }

    #[test]
    fn clusters_sort_by_task_count_then_latency() {
        let session = session_with_spikes(
            None,
            vec![
                spike_task(1, "Main", TaskClass::Game, 0, 10_000_000, 2_000_000),
                spike_task(2, "dxvk-cs", TaskClass::Game, 1, 11_000_000, 2_000_000),
                spike_task(
                    3,
                    "wineserver",
                    TaskClass::WineServer,
                    2,
                    12_000_000,
                    2_000_000,
                ),
                spike_task(4, "AudioThread", TaskClass::Game, 3, 50_000_000, 8_000_000),
                spike_task(5, "dxvk-submit", TaskClass::Game, 4, 51_000_000, 1_000_000),
                spike_task(
                    6,
                    "winedevice.exe",
                    TaskClass::Helper,
                    5,
                    52_000_000,
                    1_000_000,
                ),
                spike_task(7, "RenderThread", TaskClass::Game, 6, 53_000_000, 1_000_000),
            ],
        );

        let clusters = spike_clusters(&session, 5_000_000);

        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].distinct_tasks, 4);
        assert_eq!(clusters[0].max_latency_ns, 8_000_000);
        assert_eq!(clusters[1].distinct_tasks, 3);
    }

    #[test]
    fn elapsed_time_formats_when_monotonic_start_is_available() {
        assert_eq!(elapsed_ms(Some(1_000_000_000), 1_250_000_000), Some(250));
        assert_eq!(elapsed_ms(None, 1_250_000_000), None);
        assert_eq!(elapsed_ms(Some(2_000_000_000), 1_250_000_000), None);
        assert_eq!(format_elapsed(Some(42)), "42ms");
        assert_eq!(format_elapsed(None), "-");
    }

    #[test]
    fn report_text_contains_cluster_details() {
        let session = session_with_spikes(
            Some(1_000_000_000),
            vec![
                spike_task(1, "Main", TaskClass::Game, 0, 1_010_000_000, 6_000_000),
                spike_task(2, "dxvk-cs", TaskClass::Game, 1, 1_011_000_000, 2_000_000),
                spike_task(
                    3,
                    "wineserver",
                    TaskClass::WineServer,
                    2,
                    1_012_000_000,
                    1_500_000,
                ),
                spike_task(
                    4,
                    "AudioThread",
                    TaskClass::Game,
                    3,
                    1_013_000_000,
                    1_250_000,
                ),
            ],
        );

        let text = render_report(Path::new("session.json"), &session, None, 10, 5);

        assert!(text.contains("spike clusters"));
        assert!(text.contains("source=top_spikes fallback"));
        assert!(text.contains("elapsed=10ms"));
        assert!(text.contains("labels=render-main,dxvk,wine,audio"));
        assert!(text.contains("1(Game:Main"));
        assert!(text.contains("cpu=0"));
        assert!(text.contains("latency=6.000ms"));
    }

    #[test]
    fn cluster_analysis_prefers_durable_spike_events() {
        let session = session_with_spikes(
            None,
            vec![spike_task(
                99,
                "RenderThread",
                TaskClass::Game,
                0,
                100_000_000,
                9_000_000,
            )],
        );
        let spike_events = vec![
            spike_event(1, "Main", TaskClass::Game, 0, 10_000_000, 3_000_000),
            spike_event(2, "dxvk-cs", TaskClass::Game, 1, 11_000_000, 2_000_000),
            spike_event(
                3,
                "wineserver",
                TaskClass::WineServer,
                2,
                12_000_000,
                1_500_000,
            ),
        ];

        let analysis = spike_cluster_analysis(&session, Some(&spike_events), 5_000_000);

        assert_eq!(analysis.source, SpikeClusterSource::SpikeEvents);
        assert_eq!(analysis.source_count, 3);
        assert_eq!(analysis.clusters.len(), 1);
        assert_eq!(analysis.clusters[0].distinct_tasks, 3);
        assert_eq!(analysis.clusters[0].points[0].elapsed_ms, Some(10));
    }

    #[test]
    fn cluster_analysis_falls_back_to_retained_top_spikes() {
        let session = session_with_spikes(
            None,
            vec![
                spike_task(1, "Main", TaskClass::Game, 0, 10_000_000, 3_000_000),
                spike_task(2, "dxvk-cs", TaskClass::Game, 1, 11_000_000, 2_000_000),
                spike_task(
                    3,
                    "wineserver",
                    TaskClass::WineServer,
                    2,
                    12_000_000,
                    1_500_000,
                ),
            ],
        );

        let analysis = spike_cluster_analysis(&session, None, 5_000_000);

        assert_eq!(analysis.source, SpikeClusterSource::TopSpikesFallback);
        assert_eq!(analysis.source_count, 3);
        assert_eq!(analysis.clusters.len(), 1);
    }

    fn session_with_spikes(
        monotonic_start_ns: Option<u64>,
        tasks: Vec<SessionTask>,
    ) -> SessionFile {
        let mut top_spikes = Vec::new();
        for task in &tasks {
            for spike in &task.top_spikes {
                top_spikes.push(SessionSpike {
                    task: task.task,
                    active: task.active,
                    class: spike.class,
                    process_pid: spike.process_pid,
                    process_comm: spike.process_comm.clone(),
                    comm: task.comm.clone(),
                    cpu: spike.cpu,
                    prio: spike.prio,
                    latency_ns: spike.latency_ns,
                    wakeup_ns: spike.wakeup_ns,
                    switch_ns: spike.switch_ns,
                });
            }
        }
        top_spikes.sort_by_key(|spike| std::cmp::Reverse(spike.latency_ns));

        SessionFile {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some("test".to_owned()),
            started_at: recorded_time(),
            ended_at: recorded_time(),
            monotonic_start_ns,
            monotonic_end_ns: None,
            duration_ms: 1_000,
            stop_reason: "test".to_owned(),
            config: RecordedConfig {
                manual_pids: Vec::new(),
                tree_roots: Vec::new(),
                summary_period_ms: 1_000,
                spike_threshold_ns: 1_000_000,
                verbose: false,
            },
            metadata: system_metadata(),
            target_pids_max: 1024,
            active_target_pids_count: tasks.len(),
            active_expanded_tasks: tasks.iter().map(|task| task.task).collect(),
            spike_event_count: 0,
            spike_events_truncated: false,
            tasks,
            top_spikes,
        }
    }

    fn spike_task(
        task: u32,
        comm: &str,
        class: TaskClass,
        cpu: u32,
        switch_ns: u64,
        latency_ns: u64,
    ) -> SessionTask {
        SessionTask {
            task,
            active: true,
            first_seen_ms: 0,
            last_seen_ms: 1_000,
            removed_ms: None,
            class,
            process_pid: Some(100),
            process_comm: "process".to_owned(),
            comm: comm.to_owned(),
            latency: RecordedLatency {
                samples: 1,
                stored_samples: 1,
                truncated_samples: 0,
                percentile_scope: "exact".to_owned(),
                histogram: Vec::new(),
                min_ns: latency_ns,
                avg_ns: latency_ns,
                p95_ns: latency_ns,
                p99_ns: latency_ns,
                max_ns: latency_ns,
                over_1ms: u64::from(latency_ns >= 1_000_000),
                over_2ms: u64::from(latency_ns >= 2_000_000),
                over_5ms: u64::from(latency_ns >= 5_000_000),
            },
            cpu: RecordedCpuSnapshot {
                busiest_cpu: Some(cpu),
                busiest_cpu_samples: 1,
                worst_cpu: Some(cpu),
                worst_cpu_max_ns: latency_ns,
                spikiest_cpu: Some(cpu),
                spikiest_cpu_spikes: 1,
                per_cpu: Vec::new(),
            },
            top_spikes: vec![RecordedSpike {
                class,
                process_pid: Some(100),
                process_comm: "process".to_owned(),
                cpu,
                prio: 120,
                latency_ns,
                wakeup_ns: switch_ns.saturating_sub(latency_ns),
                switch_ns,
            }],
        }
    }

    fn spike_event(
        task: u32,
        comm: &str,
        class: TaskClass,
        cpu: u32,
        switch_ns: u64,
        latency_ns: u64,
    ) -> SpikeEvent {
        SpikeEvent {
            elapsed_ms: u128::from(switch_ns / 1_000_000),
            task,
            active: true,
            class,
            process_pid: Some(100),
            process_comm: "process".to_owned(),
            comm: comm.to_owned(),
            cpu,
            prio: 120,
            latency_ns,
            wakeup_ns: switch_ns.saturating_sub(latency_ns),
            switch_ns,
        }
    }

    fn recorded_time() -> RecordedTime {
        RecordedTime {
            unix_seconds: 0,
            unix_nanos: 0,
            local: "test".to_owned(),
        }
    }

    fn system_metadata() -> SystemMetadata {
        SystemMetadata {
            kernel_osrelease: None,
            kernel_version: None,
            cpu_online: None,
            cpu_possible: None,
            cpu_topology: Vec::new(),
            scx_state: None,
            scx_ops: None,
            scx_enable_seq: None,
        }
    }
}
