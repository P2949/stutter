use std::collections::BTreeMap;

use log::info;
use stutter_common::SchedulerEvent;
use stutter_core::ids::CpuId;

use super::{
    CpuSnapshot, IntervalRecord, IntervalRecordFromSnapshotInput, LatencySnapshot, TaskStatsMap,
    format_latency, interval_record_from_snapshot,
};

fn format_cpu(cpu: Option<CpuId>) -> String {
    match cpu {
        Some(cpu) => cpu.to_string(),
        None => "-".to_owned(),
    }
}

pub fn print_event(event: &SchedulerEvent, comm: &str, label: &str) {
    let latency_us = event.latency_ns as f64 / 1_000.0;
    let latency_ms = event.latency_ns as f64 / 1_000_000.0;

    info!(
        "{label} runnable_latency={latency_us:.3}us ({latency_ms:.6}ms) task={} cpu={} wakeup_target_cpu={} prio={} comm={} wakeup_ns={} switch_ns={}",
        event.tid,
        event.cpu,
        event.wakeup_target_cpu,
        event.prio,
        comm,
        event.wakeup_ns,
        event.switch_ns,
    );
}

pub fn print_latency_line(
    label: &str,
    task: u32,
    comm: &str,
    latency: &LatencySnapshot,
    cpu: &CpuSnapshot,
) {
    let percentile_note = if latency.samples_truncated > 0 {
        format!(" percentile_scope={}", latency.percentile_scope)
    } else {
        String::new()
    };

    info!(
        "{label} task={} comm={} samples={} stored_samples={} truncated_samples={} min={} avg={} p95={} p99={} max={} over_1ms={} over_2ms={} over_5ms={} busiest_cpu={} busiest_cpu_samples={} worst_cpu={} worst_cpu_max={} spikiest_cpu={} spikiest_cpu_spikes={}{}",
        task,
        comm,
        latency.count,
        latency.stored_samples,
        latency.samples_truncated,
        format_latency(latency.min_ns),
        format_latency(latency.avg_ns),
        format_latency(latency.p95_ns),
        format_latency(latency.p99_ns),
        format_latency(latency.max_ns),
        latency.over_1ms,
        latency.over_2ms,
        latency.over_5ms,
        format_cpu(cpu.busiest_cpu),
        cpu.busiest_cpu_samples,
        format_cpu(cpu.worst_cpu),
        format_latency(cpu.worst_cpu_max_ns),
        format_cpu(cpu.spikiest_cpu),
        cpu.spikiest_cpu_spikes,
        percentile_note,
    );
}

pub fn collect_interval_summaries_labeled(
    label: &str,
    stats_by_task: &mut TaskStatsMap,
    elapsed_ms: u64,
    drop_counters: &crate::ebpf_loader::DropCountersSnapshot,
    prev_faults_map: Option<&aya::maps::HashMap<aya::maps::MapData, u32, [u64; 2]>>,
    psi_snapshot: Option<&crate::psi::PsiDelta>,
    prev_faults_snapshot: &mut BTreeMap<u32, (u64, u64)>,
) -> Vec<IntervalRecord> {
    if prev_faults_map.is_none() {
        prev_faults_snapshot.clear();
    }

    let mut interval_records = Vec::new();

    for (task, stats) in stats_by_task.iter_mut() {
        let task_raw = task.as_u32();
        // Read cumulative fault counters from the eBPF map (if present),
        // compute the interval delta relative to the previous snapshot, and
        // update both the stats and the snapshot for next interval.
        let mut maj_delta = 0u64;
        let mut min_delta = 0u64;
        let mut counters = None;

        if let Some(map) = prev_faults_map
            && let Ok(c) = map.get(&task_raw, 0)
        {
            let current_maj = c[0];
            let current_min = c[1];
            let prev = prev_faults_snapshot
                .get(&task_raw)
                .copied()
                .unwrap_or((current_maj, current_min));

            maj_delta = current_maj.saturating_sub(prev.0);
            min_delta = current_min.saturating_sub(prev.1);
            counters = Some((current_maj, current_min));

            // We no longer update stats.last_spike_major_faults here, as interval
            // summarization and spike recording now use separate baselines.
            // Spikes track their own cumulative counts to compute deltas since
            // the previous spike (or task start).
        }

        let Some(latency) = stats.interval_latency.snapshot_and_reset() else {
            if let Some(perf) = stats.interval_cpu_perf.as_mut() {
                let _ = perf.snapshot_and_reset();
            }
            continue;
        };

        if let Some((current_maj, current_min)) = counters {
            prev_faults_snapshot.insert(task_raw, (current_maj, current_min));
        } else {
            // If we didn't get a fresh reading from the eBPF map this interval
            // (either because the map is globally missing or this task was absent),
            // we must clear any previous snapshot state for this task.
            prev_faults_snapshot.remove(&task_raw);
        }

        let cpu = stats.interval_cpu.snapshot_and_reset();

        print_latency_line(label, task_raw, &stats.comm, &latency, &cpu);
        interval_records.push(interval_record_from_snapshot(
            IntervalRecordFromSnapshotInput {
                task: task_raw,
                stats,
                latency: &latency,
                cpu: &cpu,
                elapsed_ms,
                drop_counters,
                psi: psi_snapshot,
                faults_delta: (maj_delta, min_delta),
            },
        ));
    }

    interval_records
}

pub fn print_session_summaries(stats_by_task: &mut TaskStatsMap) {
    for (task, stats) in stats_by_task.iter_mut() {
        let task_raw = task.as_u32();
        let Some(latency) = stats.session_latency.snapshot() else {
            continue;
        };

        let cpu = stats.session_cpu.snapshot();

        print_latency_line("session", task_raw, &stats.comm, &latency, &cpu);

        if latency.samples_truncated > 0 {
            info!(
                "session_percentile_warning task={} comm={} truncated_samples={} percentile_scope={} note=\"p95/p99 are histogram estimates across the full session; prefer max and over_1ms/over_2ms/over_5ms for exact spike counts\"",
                task, stats.comm, latency.samples_truncated, latency.percentile_scope
            );
        }

        for cpu_line in cpu.per_cpu {
            info!(
                "session_task_cpu task={} comm={} cpu={} samples={} max={} spikes={}",
                task,
                stats.comm,
                cpu_line.cpu,
                cpu_line.samples,
                format_latency(cpu_line.max_ns),
                cpu_line.spikes,
            );
        }

        for spike in &stats.top_spikes {
            info!(
                "session_spike task={} comm={} cpu={} prio={} latency={} wakeup_ns={} switch_ns={}",
                task,
                stats.comm,
                spike.cpu,
                spike.prio,
                format_latency(spike.latency_ns),
                spike.wakeup_ns,
                spike.switch_ns,
            );
        }
    }
}
