use super::{frame::map_diagnosis, *};

pub(super) fn render_task_latency_sections(
    session: &SessionFile,
    top: usize,
    filter_class: Option<TaskClass>,
) -> String {
    let mut writer = ReportTextWriter::new();
    let mut tasks = latency_tasks(session, filter_class);
    let duration_secs = session.core.duration_ms as f64 / 1000.0;

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));
    push_max_latency_tasks(&mut writer, &tasks, top, duration_secs);

    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.over_5ms),
            std::cmp::Reverse(task.latency.over_2ms),
            std::cmp::Reverse(task.latency.over_1ms),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });
    push_threshold_tasks(&mut writer, &tasks, top, duration_secs);

    writer.finish()
}

pub(super) fn render_top_spikes(session: &SessionFile, top: usize) -> String {
    let mut writer = ReportTextWriter::new();

    writer.line("top spikes");
    writer.line("----------");
    for spike in session.top_spikes.iter().take(top) {
        writer.line(format!(
            "task={} active={} class={:?} comm={} cpu={} wakeup_target_cpu={} latency={} wakeup_ns={} switch_ns={} observed_runnable_depth={} target_pending_wakeups={}(diagnostic) switch_prev_pid={} switch_prev_state={} switch_prev_state_label={}",
            spike.task,
            spike.active,
            spike.class,
            spike.comm,
            spike.cpu,
            spike.wakeup_target_cpu,
            format_latency(spike.latency_ns),
            spike.wakeup_ns,
            spike.switch_ns,
            spike.observed_runnable_depth,
            spike.target_pending_wakeups,
            spike.switch_prev_pid,
            spike.switch_prev_state,
            classify_switch_prev_state(spike.switch_prev_state),
        ));
    }
    writer.blank();
    writer.finish()
}

pub(super) fn render_spike_clusters(
    cluster_analysis: &SpikeClusterAnalysis,
    top: usize,
    cluster_window_ms: u64,
) -> String {
    let mut writer = ReportTextWriter::new();

    writer.line("spike clusters");
    writer.line("--------------");
    writer.line(
        stutter_report::render::text::cluster::render_cluster_source(
            &cluster_source_for_render(cluster_analysis),
            cluster_window_ms,
        ),
    );
    push_runqueue_depth_notes(&mut writer);
    if cluster_analysis.clusters.is_empty() {
        writer.line(format!(
            "none min_tasks={} window_ms={}",
            MIN_CLUSTER_TASKS, cluster_window_ms
        ));
    } else {
        for (rank, cluster) in cluster_analysis.clusters.iter().take(top).enumerate() {
            writer.line(stutter_report::render::text::cluster::render_cluster(
                rank + 1,
                &map_cluster(cluster),
            ));
        }
    }
    writer.blank();
    writer.finish()
}

pub(crate) fn map_spike_point(p: &crate::spike::SpikePoint) -> stutter_report::model::SpikePoint {
    stutter_report::model::SpikePoint {
        task: p.task,
        class: format!("{:?}", p.class),
        process_pid: p.process_pid,
        comm: p.comm.clone(),
        cpu: p.cpu,
        wakeup_target_cpu: p.wakeup_target_cpu,
        latency_ns: p.latency_ns,
        wakeup_ns: p.wakeup_ns,
        switch_ns: p.switch_ns,
        target_pending_wakeups: p.target_pending_wakeups,
        observed_runnable_depth: p.observed_runnable_depth,
        switch_prev_pid: p.switch_prev_pid,
        switch_prev_state: p.switch_prev_state,
        switch_prev_state_label: p.switch_prev_state_label.clone(),
        scx_ops: p.scx_ops.clone(),
        primary_cause: p.primary_cause.clone(),
        cause_tags: p.cause_tags.clone(),
    }
}

pub(crate) fn map_cluster(c: &crate::spike::SpikeCluster) -> stutter_report::model::SpikeCluster {
    stutter_report::model::SpikeCluster {
        points: c.points.iter().map(map_spike_point).collect(),
        distinct_tasks: c.distinct_tasks,
        min_switch_ns: c.min_switch_ns,
        max_switch_ns: c.max_switch_ns,
        max_latency_ns: c.max_latency_ns,
        diagnosis: c.diagnosis.as_ref().map(map_diagnosis),
        wake_graph: c
            .wake_graph
            .iter()
            .map(|w| stutter_report::model::WakeGraphEdge {
                waker_tid: w.waker_tid,
                waker_comm: w.waker_comm.clone(),
                wakee_tid: w.wakee_tid,
                wakee_comm: w.wakee_comm.clone(),
                count: w.count,
                max_latency_ns: w.max_latency_ns,
            })
            .collect(),
    }
}

fn latency_tasks(session: &SessionFile, filter_class: Option<TaskClass>) -> Vec<&SessionTask> {
    session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|class| task.class == class))
        .collect()
}

fn push_max_latency_tasks(
    writer: &mut ReportTextWriter,
    tasks: &[&SessionTask],
    top: usize,
    duration_secs: f64,
) {
    writer.line("top tasks by max latency");
    writer.line("------------------------");
    for task in tasks.iter().take(top) {
        writer.line(format!(
            "task={} active={} class={:?} comm={} process_pid={:?} samples={} max={} over_1ms={} over_2ms={} over_5ms={} spike_rate_per_s={:.1} percentile_scope={}{}",
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
            spike_rate(task.latency.over_1ms, duration_secs),
            task.latency.percentile_scope,
            format_task_cpu_perf(task),
        ));
    }
    writer.blank();
}

fn push_threshold_tasks(
    writer: &mut ReportTextWriter,
    tasks: &[&SessionTask],
    top: usize,
    duration_secs: f64,
) {
    writer.line("top tasks by threshold counters");
    writer.line("-------------------------------");
    for task in tasks.iter().take(top) {
        writer.line(format!(
            "task={} active={} class={:?} comm={} over_5ms={} over_2ms={} over_1ms={} spike_rate_per_s={:.1} max={}",
            task.task,
            task.active,
            task.class,
            task.comm,
            task.latency.over_5ms,
            task.latency.over_2ms,
            task.latency.over_1ms,
            spike_rate(task.latency.over_1ms, duration_secs),
            format_latency(task.latency.max_ns),
        ));
    }
    writer.blank();
}

fn spike_rate(over_1ms: u64, duration_secs: f64) -> f64 {
    if duration_secs > 0.0 {
        over_1ms as f64 / duration_secs
    } else {
        0.0
    }
}

fn cluster_source_for_render(
    cluster_analysis: &SpikeClusterAnalysis,
) -> stutter_report::model::SpikeClusterAnalysis {
    stutter_report::model::SpikeClusterAnalysis {
        source: match cluster_analysis.source {
            crate::report::model::SpikeClusterSource::SpikeEvents => {
                stutter_report::model::SpikeClusterSource::SpikeEvents
            }
            crate::report::model::SpikeClusterSource::TopSpikesFallback => {
                stutter_report::model::SpikeClusterSource::TopSpikesFallback
            }
        },
        source_count: cluster_analysis.source_count,
        clusters: vec![],
    }
}

fn push_runqueue_depth_notes(writer: &mut ReportTextWriter) {
    writer.line(
        "observed_runnable_depth is an approximation of runnable pressure on the CPU reconstructed",
    );
    writer
        .line("from sched tracepoints. target_pending_wakeups is diagnostic-only monitored-target backlog.");
    writer.line(
        "It is not kernel runqueue depth and must not be used for scoring or tuning decisions.",
    );
    writer.blank();
}
