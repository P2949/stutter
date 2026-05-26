//! Shared report-analysis formatting and cluster display helpers.

use super::*;

pub(crate) fn format_task_cpu_perf(task: &SessionTask) -> String {
    let Some(perf) = &task.cpu_perf else {
        return String::new();
    };

    let mut parts = Vec::new();
    if let Some(ipc) = perf.ipc {
        parts.push(format!("ipc={ipc:.2}"));
    }
    if let Some(cache_mpki) = perf.cache_mpki {
        parts.push(format!("cache_mpki={cache_mpki:.1}"));
    }
    if let Some(cache_miss_rate) = perf.cache_miss_rate {
        parts.push(format!("cache_miss_rate={:.1}%", cache_miss_rate * 100.0));
    }
    if let Some(cycles) = perf.cycles {
        parts.push(format!("cycles={cycles}"));
    }
    if let Some(instructions) = perf.instructions {
        parts.push(format!("instructions={instructions}"));
    }
    parts.push(format!("multiplexed={}", perf.multiplexed));

    format!(" cpu_perf: {}", parts.join(" "))
}

pub(crate) fn cluster_elapsed_ms(cluster: &SpikeCluster) -> Option<u64> {
    cluster
        .points
        .iter()
        .filter_map(|point| point.elapsed_ms)
        .min()
}

pub(crate) fn cluster_elapsed_range(cluster: &SpikeCluster) -> Option<(u64, u64)> {
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

pub(crate) fn format_option<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

pub(crate) fn cluster_elapsed(cluster: &SpikeCluster) -> Option<u64> {
    cluster
        .points
        .iter()
        .filter_map(|point| point.elapsed_ms)
        .min()
}

pub(crate) fn percentile_warning_note(percentile_scope: &str) -> &'static str {
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

pub(crate) fn format_elapsed(elapsed_ms: Option<u64>) -> String {
    match elapsed_ms {
        Some(elapsed_ms) => format!("{elapsed_ms}ms"),
        None => "-".to_owned(),
    }
}
