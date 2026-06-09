use std::collections::BTreeSet;

use super::diagnosis::render_diagnosis_lines;
use crate::model::{MIN_CLUSTER_TASKS, SpikeCluster, SpikeClusterAnalysis, SpikeClusterSource};

const MAX_INLINE_CLUSTER_POINTS: usize = 8;

pub fn render_cluster_source(analysis: &SpikeClusterAnalysis, cluster_window_ms: u64) -> String {
    let source = match analysis.source {
        SpikeClusterSource::SpikeEvents => "source=spike_events",
        SpikeClusterSource::TopSpikesFallback => "source=top_spikes fallback",
    };
    format!(
        "{source} count={} window_ms={} min_tasks={}",
        analysis.source_count, cluster_window_ms, MIN_CLUSTER_TASKS
    )
}

pub fn render_cluster(rank: usize, cluster: &SpikeCluster) -> String {
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
        format!("\n{}", render_diagnosis_lines(d, "  "))
    } else {
        String::new()
    };

    let wake_block = if cluster.wake_graph.is_empty() {
        String::new()
    } else {
        let mut wake_lines = Vec::new();
        wake_lines.push("\n  wake relationships:".to_owned());
        for edge in &cluster.wake_graph {
            wake_lines.push(format!(
                "    {} [{}] woke {} [{}] (count={}, max_lat={})",
                edge.waker_comm,
                edge.waker_tid,
                edge.wakee_comm,
                edge.wakee_tid,
                edge.count,
                format_latency(edge.max_latency_ns)
            ));
        }
        wake_lines.join("\n")
    };

    format!(
        "#{rank} elapsed={} span={} tasks={} total_spikes={} shown_points={} omitted_points={} cpus={} labels={} max={} switch_ns={}..{} points={}{}{}",
        format_elapsed_ns(elapsed),
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
        diagnosis_line,
        wake_block
    )
}

pub fn render_cluster_point(point: &crate::model::SpikePoint) -> String {
    let scx = if let Some(ops) = &point.scx_ops {
        format!(" scx_ops={ops}")
    } else {
        String::new()
    };
    format!(
        "{}({}:{} cpu={} wakeup_target_cpu={} latency={} switch_ns={} process_pid={} wakeup_ns={} observed_runnable_depth={} target_pending_wakeups={}(diag) switch_prev_pid={} switch_prev_state={} switch_prev_state_label={}{}{}{})",
        point.task,
        point.class,
        point.comm,
        point.cpu,
        point.wakeup_target_cpu,
        format_latency(point.latency_ns),
        point.switch_ns,
        point
            .process_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        point.wakeup_ns,
        point.observed_runnable_depth,
        point.target_pending_wakeups,
        point.switch_prev_pid,
        point.switch_prev_state,
        point.switch_prev_state_label,
        scx,
        if let Some(p) = &point.primary_cause {
            format!(" primary_cause={}", p)
        } else {
            String::new()
        },
        if !point.cause_tags.is_empty() {
            format!(" tags={}", point.cause_tags.join(","))
        } else {
            String::new()
        }
    )
}

pub fn cluster_labels(cluster: &SpikeCluster) -> Vec<String> {
    let mut labels = Vec::new();

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "RenderThread" || point.comm == "Main")
    {
        labels.push("render-main".to_owned());
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm.starts_with("dxvk-"))
    {
        labels.push("dxvk".to_owned());
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "wineserver" || point.comm.contains("winedevice"))
    {
        labels.push("wine".to_owned());
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "AudioThread")
    {
        labels.push("audio".to_owned());
    }

    labels
}

pub fn cluster_elapsed(cluster: &SpikeCluster) -> u64 {
    cluster
        .points
        .iter()
        .map(|p| p.wakeup_ns.saturating_sub(p.latency_ns))
        .min()
        .unwrap_or(cluster.min_switch_ns)
}

pub fn format_elapsed_ns(ns: u64) -> String {
    format!("{:.3}s", ns as f64 / 1_000_000_000.0)
}

pub fn format_latency(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.3}ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.3}us", ns as f64 / 1_000.0)
    }
}
