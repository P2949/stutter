//! Diagnosis evidence builders; this module owns supporting evidence and contextual weak candidates.

use std::{cmp::Ordering, collections::BTreeMap};

use super::{
    DiagnosisCandidate, DiagnosisConfig, EvidenceItem, EvidenceKind, StutterCause,
    anchor::select_anchor,
    candidate::{is_scheduler_cause, push_candidate, push_supporting_evidence},
};
use crate::{
    metrics::RuntimeSliceRecord,
    process_tree::TaskClass,
    recorder::{CpuFreqRecord, IntervalRecord, MigrationEventRecord},
    spike::{SpikeCluster, SpikePoint},
};

pub(super) fn push_runnable_depth_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    point: &SpikePoint,
) {
    if point.observed_runnable_depth >= 4 {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: 0.40,
                message: format!(
                    "high monitored runnable depth ({} target tasks) suggests target-local CPU contention",
                    point.observed_runnable_depth
                ),
                timestamp_ms: point.elapsed_ms,
                start_ns: Some(point.wakeup_ns),
                end_ns: Some(point.switch_ns),
            },
        );
    }

    if point.target_pending_wakeups >= 4 {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: 0.20,
                message: format!(
                    "monitored wakeup backlog ({} tasks) (diagnostic-only)",
                    point.target_pending_wakeups
                ),
                timestamp_ms: point.elapsed_ms,
                start_ns: Some(point.wakeup_ns),
                end_ns: Some(point.switch_ns),
            },
        );
    }
}

pub(super) fn push_scx_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    point: &SpikePoint,
) {
    if point.scx_ops.is_none() && point.scx_state.is_none() {
        return;
    }

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::ScxState,
            strength: 0.25,
            message: format!(
                "SCX state near spike: ops={} state={}",
                point.scx_ops.as_deref().unwrap_or("-"),
                point.scx_state.as_deref().unwrap_or("-")
            ),
            timestamp_ms: point.elapsed_ms,
            start_ns: Some(point.wakeup_ns),
            end_ns: Some(point.switch_ns),
        },
    );
}

pub(super) fn push_cpu_frequency_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    events: &[CpuFreqRecord],
    config: DiagnosisConfig,
) {
    let mut by_cpu: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for event in events {
        let entry = by_cpu
            .entry(event.cpu)
            .or_insert((event.freq_khz, event.freq_khz));
        entry.0 = entry.0.min(event.freq_khz);
        entry.1 = entry.1.max(event.freq_khz);
    }

    let Some((cpu, (min_freq, max_freq))) = by_cpu
        .into_iter()
        .filter(|(_, (_, max_freq))| *max_freq > 0)
        .map(|(cpu, (min_freq, max_freq))| {
            let drop_percent =
                ((max_freq.saturating_sub(min_freq)) as f64 / max_freq as f64) * 100.0;
            (cpu, (min_freq, max_freq, drop_percent))
        })
        .filter(|(_, (_, _, drop_percent))| *drop_percent >= config.cpu_freq_drop_percent)
        .max_by(|a, b| a.1.2.partial_cmp(&b.1.2).unwrap_or(Ordering::Equal))
        .map(|(cpu, (min_freq, max_freq, _))| (cpu, (min_freq, max_freq)))
    else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::CpuFrequency,
            strength: 0.25,
            message: format!(
                "CPU frequency drop near spike: cpu={} min={}kHz max={}kHz",
                cpu, min_freq, max_freq
            ),
            timestamp_ms: None,
            start_ns: None,
            end_ns: None,
        },
    );
}

pub(super) fn push_migration_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    events: &[MigrationEventRecord],
    cluster_elapsed_range: Option<(u64, u64)>,
    config: DiagnosisConfig,
) {
    let Some(event) = events.iter().find(|event| {
        cluster_elapsed_range.is_none_or(|(min, max)| {
            event.elapsed_ms >= min.saturating_sub(config.migration_window_ms)
                && event.elapsed_ms <= max.saturating_add(config.migration_window_ms)
        })
    }) else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::Migration,
            strength: 0.20,
            message: format!(
                "task migration event near spike: tid={} cpu {}->{}",
                event.tid, event.from_cpu, event.to_cpu
            ),
            timestamp_ms: Some(event.elapsed_ms),
            start_ns: Some(event.timestamp_ns),
            end_ns: Some(event.timestamp_ns),
        },
    );
}

pub(super) fn push_page_fault_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    intervals: &[IntervalRecord],
    config: DiagnosisConfig,
) {
    let Some(record) = intervals.iter().find(|record| {
        record.major_faults.saturating_add(record.minor_faults) >= config.page_fault_delta_threshold
    }) else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::PageFaults,
            strength: 0.20,
            message: "page fault activity near spike; treat as supporting evidence only".to_owned(),
            timestamp_ms: Some(record.elapsed_ms),
            start_ns: None,
            end_ns: None,
        },
    );
}

pub(super) fn push_cpu_perf_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    intervals: &[IntervalRecord],
    cluster: &SpikeCluster,
    config: DiagnosisConfig,
) {
    let anchor = select_anchor(cluster);
    let anchor_process_pid = cluster
        .points
        .iter()
        .find(|point| point.task == anchor.task)
        .and_then(|point| point.process_pid);

    let mut matching = intervals
        .iter()
        .filter(|record| record.cpu_perf.is_some() && record.task == anchor.task)
        .collect::<Vec<_>>();

    if matching.is_empty()
        && let Some(process_pid) = anchor_process_pid
    {
        matching = intervals
            .iter()
            .filter(|record| record.cpu_perf.is_some() && record.process_pid == Some(process_pid))
            .collect();
    }

    if matching.is_empty() && is_cpu_perf_class(anchor.class) {
        matching = intervals
            .iter()
            .filter(|record| record.cpu_perf.is_some() && record.class == anchor.class)
            .collect();
    }

    let Some(record) = matching.into_iter().max_by(|a, b| {
        cpu_perf_evidence_score(a, config)
            .partial_cmp(&cpu_perf_evidence_score(b, config))
            .unwrap_or(Ordering::Equal)
    }) else {
        return;
    };

    let score = cpu_perf_evidence_score(record, config);
    if score <= 0.0 {
        return;
    }

    let Some(perf) = &record.cpu_perf else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::CpuPerf,
            strength: 0.25,
            message: format!(
                "CPU perf near spike: task={} ipc={} cache_mpki={} cache_miss_rate={}",
                record.task,
                format_optional_float(perf.ipc, 2),
                format_optional_float(perf.cache_mpki, 1),
                format_optional_percent(perf.cache_miss_rate),
            ),
            timestamp_ms: Some(record.elapsed_ms),
            start_ns: None,
            end_ns: None,
        },
    );
}

pub(super) fn push_runtime_slice_supporting_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    runtime_slices: &[RuntimeSliceRecord],
    cluster: &SpikeCluster,
    config: DiagnosisConfig,
) {
    let anchor = select_anchor(cluster);
    let Some(anchor_record) = runtime_slices
        .iter()
        .filter(|record| record.task == anchor.task)
        .max_by(|a, b| {
            a.wait_ratio
                .unwrap_or(0.0)
                .partial_cmp(&b.wait_ratio.unwrap_or(0.0))
                .unwrap_or(Ordering::Equal)
        })
    else {
        return;
    };

    if let Some(wait_ratio) = anchor_record.wait_ratio
        && wait_ratio >= config.runtime_wait_high_ratio
    {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::RuntimeRunqueueWait,
                strength: 0.35,
                message: format!(
                    "runtime slice context: anchor task={} waited on runqueue for {:.1}% of sample interval",
                    anchor_record.task,
                    wait_ratio * 100.0
                ),
                timestamp_ms: Some(anchor_record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    if let Some(runtime_ratio) = anchor_record.runtime_ratio
        && runtime_ratio >= config.runtime_high_ratio
    {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::RuntimeCpuUse,
                strength: 0.20,
                message: format!(
                    "runtime slice context: anchor task={} was running for {:.1}% of sample interval; this weakens a pure waiting-only explanation",
                    anchor_record.task,
                    runtime_ratio * 100.0
                ),
                timestamp_ms: Some(anchor_record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }
}

pub(super) fn push_runtime_slice_context_candidates(
    candidates: &mut Vec<DiagnosisCandidate>,
    runtime_slices: &[RuntimeSliceRecord],
    cluster: &SpikeCluster,
    config: DiagnosisConfig,
) {
    let cluster_tasks = cluster
        .points
        .iter()
        .map(|point| point.task)
        .collect::<std::collections::BTreeSet<_>>();

    let high_background_runtime = runtime_slices
        .iter()
        .filter(|record| !cluster_tasks.contains(&record.task))
        .filter(|record| record.runtime_ratio.unwrap_or(0.0) >= config.runtime_high_ratio)
        .max_by(|a, b| {
            a.runtime_ratio
                .unwrap_or(0.0)
                .partial_cmp(&b.runtime_ratio.unwrap_or(0.0))
                .unwrap_or(Ordering::Equal)
        });

    if let Some(record) = high_background_runtime {
        let score = (record.runtime_ratio.unwrap_or(0.0) as f32).min(0.35);
        push_candidate(
            candidates,
            StutterCause::CpuMonopolizationCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::RuntimeCpuUse,
                strength: score,
                message: format!(
                    "runtime slice context: non-anchor task={} comm='{}' consumed {:.1}% CPU time near spike",
                    record.task,
                    record.comm,
                    record.runtime_ratio.unwrap_or(0.0) * 100.0
                ),
                timestamp_ms: Some(record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    let high_wait = runtime_slices
        .iter()
        .filter(|record| record.wait_ratio.unwrap_or(0.0) >= config.runtime_wait_high_ratio)
        .max_by(|a, b| {
            a.wait_ratio
                .unwrap_or(0.0)
                .partial_cmp(&b.wait_ratio.unwrap_or(0.0))
                .unwrap_or(Ordering::Equal)
        });

    if let Some(record) = high_wait
        && !candidates
            .iter()
            .any(|candidate| is_scheduler_cause(candidate.cause))
    {
        let score = (record.wait_ratio.unwrap_or(0.0) as f32).min(0.35);
        push_candidate(
            candidates,
            StutterCause::RuntimeWaitCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::RuntimeRunqueueWait,
                strength: score,
                message: format!(
                    "runtime slice context: task={} comm='{}' waited on runqueue for {:.1}% of sample interval",
                    record.task,
                    record.comm,
                    record.wait_ratio.unwrap_or(0.0) * 100.0
                ),
                timestamp_ms: Some(record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }
}

fn cpu_perf_evidence_score(record: &IntervalRecord, config: DiagnosisConfig) -> f64 {
    let Some(perf) = &record.cpu_perf else {
        return 0.0;
    };

    let low_ipc_score = perf
        .ipc
        .filter(|ipc| *ipc < config.low_ipc_threshold)
        .map(|ipc| 10_000.0 + config.low_ipc_threshold - ipc)
        .unwrap_or(0.0);
    if low_ipc_score > 0.0 {
        return low_ipc_score;
    }

    perf.cache_mpki
        .filter(|mpki| *mpki > config.high_cache_mpki_threshold)
        .map(|mpki| mpki - config.high_cache_mpki_threshold)
        .unwrap_or(0.0)
}

fn is_cpu_perf_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game | TaskClass::GameHelper | TaskClass::WineServer | TaskClass::GameScope
    )
}

fn format_optional_float(value: Option<f64>, decimals: usize) -> String {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return "-".to_owned();
    };
    format!("{value:.decimals$}")
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{:.1}%", value * 100.0))
        .unwrap_or_else(|| "-".to_owned())
}
