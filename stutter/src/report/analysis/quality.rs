//! Report data-quality and block-I/O correlation helpers.
//!
//! Owns data-quality level downgrade/reporting and compact block-I/O correlation labels. Does not
//! own task row selection, pressure timelines, clustering, diagnosis, or report orchestration.

use super::*;
use crate::artifacts::{ArtifactKind, artifact_file_name};

pub(crate) fn format_optional_ratio(value: Option<f64>) -> String {
    value
        .map(|ratio| format!("{:.1}%", ratio * 100.0))
        .unwrap_or_else(|| "-".to_owned())
}

pub(crate) fn format_pressure_option(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "-".to_owned())
}

pub(crate) fn block_io_correlation_basis(session: &SessionFile) -> &str {
    if session.core.block_io_correlation_basis.is_empty() {
        "dev+sector"
    } else {
        &session.core.block_io_correlation_basis
    }
}

pub(crate) fn block_io_correlation_confidence(session: &SessionFile) -> &str {
    if session.core.block_io_correlation_confidence.is_empty() {
        crate::ebpf_loader::BlockIoCorrelationBasis::from_str(block_io_correlation_basis(session))
            .confidence()
    } else {
        &session.core.block_io_correlation_confidence
    }
}

pub(crate) fn block_io_correlation_warning(session: &SessionFile) -> Option<String> {
    crate::ebpf_loader::BlockIoCorrelationBasis::from_str(block_io_correlation_basis(session))
        .warning()
        .map(str::to_owned)
}

pub(crate) fn data_quality_summary(
    session: &SessionFile,
    validation: &crate::session_io::RunValidationReport,
) -> DataQualitySummary {
    let mut reasons = Vec::new();
    let mut level = DataQualityLevel::High;

    if !validation.errors.is_empty() {
        level = DataQualityLevel::Low;
        reasons.push("run directory has validation errors".to_owned());
    }

    if validation
        .warnings
        .iter()
        .any(|warning| warning.starts_with("DRM fence "))
    {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("DRM fence latency evidence is degraded or incomplete".to_owned());
    }

    if session.core.schema_version != SESSION_SCHEMA_VERSION {
        if session.core.schema_version > SESSION_SCHEMA_VERSION {
            level = DataQualityLevel::Low;
            reasons.push("session schema is newer than this stutter binary".to_owned());
        } else {
            level = downgrade_quality(level, DataQualityLevel::Medium);
            reasons.push("session schema is older than this stutter binary".to_owned());
        }
    }

    if session.core.event_stream_write_errors > 0 {
        level = DataQualityLevel::Low;
        reasons.push("recording stream had write errors".to_owned());
    }

    let missing_non_focus_optional = validation
        .missing_optional_files
        .iter()
        .filter(|file| missing_artifact_downgrades_quality(file.as_str()))
        .collect::<Vec<_>>();

    if !missing_non_focus_optional.is_empty() {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("optional correlation artifacts are missing".to_owned());
    }

    if session.core.spike_events_truncated {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("spike event stream was truncated".to_owned());
    }

    if session.core.spike_events_dropped_count > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("spike events were dropped".to_owned());
    }

    if session.core.interval_record_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("no interval records are available".to_owned());
    }

    if session.core.active_target_pids_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("no active target tasks were present at end of run".to_owned());
    }

    let drop_counters = &session.core.drop_counters;

    if drop_counters.total() > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);

        if drop_counters.wakeup_data_insert_failed > 0 {
            reasons.push(format!(
                "wakeup timestamp inserts failed: {} runnable-latency samples may be missing",
                drop_counters.wakeup_data_insert_failed
            ));
        }

        if drop_counters.wakeup_data_stale_entries > 0 {
            reasons.push(format!(
                "wakeup timestamp stale entries detected: {} runnable-latency samples may be stale or dropped",
                drop_counters.wakeup_data_stale_entries
            ));
        }

        if drop_counters.wakeup_data_replaced_entries > 0 {
            reasons.push(format!(
                "wakeup timestamp records were replaced before sched_switch consumed them: {} runnable-latency samples may have been coalesced or overwritten during wakeup bursts",
                drop_counters.wakeup_data_replaced_entries
            ));
        }

        if drop_counters.wakeup_data_consumed_read_failed > 0 {
            reasons.push(format!(
                "wakeup timestamp records were consumed but sched_switch tracepoint reads failed: {} runnable-latency samples were dropped after wakeup consume",
                drop_counters.wakeup_data_consumed_read_failed
            ));
        }

        if drop_counters.ringbuf_reserve_failed > 0 {
            reasons.push(format!(
                "ring buffer reserve failures detected: {} eBPF events were dropped before userspace",
                drop_counters.ringbuf_reserve_failed
            ));
        }

        if drop_counters.irq_start_times_insert_failed > 0 {
            reasons.push(format!(
                "IRQ start timestamp inserts failed: {} IRQ latency samples may be missing",
                drop_counters.irq_start_times_insert_failed
            ));
        }

        if drop_counters.block_start_insert_failed > 0 {
            reasons.push(format!(
                "block I/O start inserts failed: {} block I/O latency samples may be missing",
                drop_counters.block_start_insert_failed
            ));
        }

        if drop_counters.block_fallback_key_collisions > 0 {
            reasons.push(format!(
                "block I/O fallback key collisions detected: {} ambiguous fallback samples were dropped; block I/O latency coverage may be incomplete",
                drop_counters.block_fallback_key_collisions
            ));
        }

        if drop_counters.cpu_accounting_untracked > 0 {
            reasons.push(format!(
                "CPU accounting skipped {} events on CPU ids outside the tracked eBPF accounting range; runnable-depth and pending-wakeup diagnostics may be incomplete on very large systems",
                drop_counters.cpu_accounting_untracked
            ));
        }
    }

    if session.core.native_cgroup_filter.enabled && !session.core.native_cgroup_filter.verified {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push(
            session
                .core
                .native_cgroup_filter
                .warning
                .clone()
                .unwrap_or_else(|| {
                    "native cgroup filtering is enabled but not runtime-verified; PID expansion remains the authoritative scheduler-wakeup targeting path"
                        .to_owned()
                }),
        );
    }

    let mut percentile_scope_counts = BTreeMap::new();
    for task in &session.tasks {
        *percentile_scope_counts
            .entry(task.latency.percentile_scope.clone())
            .or_insert(0) += 1;
    }

    if percentile_scope_counts.contains_key("histogram") {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("some percentile values are histogram-estimated".to_owned());
    }

    if percentile_scope_counts.contains_key("capped_prefix") {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("some percentile values are based on capped prefix samples".to_owned());
    }

    let block_io_correlation_basis = block_io_correlation_basis(session).to_owned();
    let block_io_correlation_confidence = block_io_correlation_confidence(session).to_owned();
    let block_io_correlation_warning = block_io_correlation_warning(session);
    match block_io_correlation_basis.as_str() {
        "dev+sector" if session.core.block_io_event_count > 0 => {
            level = downgrade_quality(level, DataQualityLevel::Medium);
            if let Some(warning) = &block_io_correlation_warning {
                reasons.push(warning.clone());
            } else {
                reasons.push("block I/O correlation is approximate dev+sector matching".to_owned());
            }
        }
        "unavailable" if session.config.block_io => {
            level = downgrade_quality(level, DataQualityLevel::Medium);
            if let Some(warning) = &block_io_correlation_warning {
                reasons.push(warning.clone());
            } else {
                reasons.push("block I/O correlation is unavailable".to_owned());
            }
        }
        _ => {}
    }

    let frame_timestamp_alignment = if session.core.frame_event_count == 0 {
        "none".to_owned()
    } else if session.core.mangohud_first_frame_monotonic_ns.is_some() {
        "monotonic_observed".to_owned()
    } else {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("MangoHud frame timestamp alignment is approximate".to_owned());
        "approximate_first_row".to_owned()
    };

    let cpu_perf_requested = session.config.cpu_perf;
    let task_cpu_perf_count = session
        .tasks
        .iter()
        .filter(|task| task.cpu_perf.is_some())
        .count();
    if cpu_perf_requested && task_cpu_perf_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf was requested but no counters were recorded".to_owned());
    }
    if session.core.cpu_perf_open_errors > 0 || session.core.cpu_perf_read_errors > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf counters had open/read errors".to_owned());
    }
    if session.core.cpu_perf_skipped_tasks > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push(format!(
            "CPU perf skipped {} active tasks due to cpu_perf_max_tasks limit",
            session.core.cpu_perf_skipped_tasks
        ));
    }
    if session
        .tasks
        .iter()
        .filter_map(|task| task.cpu_perf.as_ref())
        .any(|perf| perf.multiplexed)
    {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf counters were multiplexed; values are scaled estimates".to_owned());
    }

    if reasons.is_empty() {
        reasons.push("no data-quality problems detected".to_owned());
    }

    DataQualitySummary {
        level,
        reasons,
        missing_optional_files: validation.missing_optional_files.clone(),
        validation_errors: validation.errors.clone(),
        validation_warnings: validation.warnings.clone(),
        schema_version: session.core.schema_version,
        expected_schema_version: SESSION_SCHEMA_VERSION,
        event_stream_write_errors: session.core.event_stream_write_errors,
        spike_events_truncated: session.core.spike_events_truncated,
        spike_events_retained_count: session.core.spike_events_retained_count,
        spike_events_dropped_count: session.core.spike_events_dropped_count,
        interval_record_count: session.core.interval_record_count,
        active_target_pids_count: session.core.active_target_pids_count,
        drop_counters_nonzero: drop_counters.total() > 0,
        percentile_scope_counts,
        block_io_correlation_basis,
        block_io_correlation_confidence,
        block_io_correlation_warning,
        frame_timestamp_alignment,
        cpu_perf_requested,
        cpu_perf_open_errors: session.core.cpu_perf_open_errors,
        cpu_perf_read_errors: session.core.cpu_perf_read_errors,
        cpu_perf_skipped_tasks: session.core.cpu_perf_skipped_tasks,
    }
}

fn missing_artifact_downgrades_quality(file: &str) -> bool {
    ![
        ArtifactKind::FocusEvents,
        ArtifactKind::ForegroundEvents,
        ArtifactKind::KmsFlipEvents,
        ArtifactKind::DrmFenceEvents,
        ArtifactKind::WaylandPresentationEvents,
        ArtifactKind::DisplayTopology,
        ArtifactKind::DmaBufEvents,
        ArtifactKind::GpuEngineSamples,
    ]
    .iter()
    .any(|kind| file == artifact_file_name(*kind))
}

pub(crate) fn downgrade_quality(
    current: DataQualityLevel,
    candidate: DataQualityLevel,
) -> DataQualityLevel {
    use DataQualityLevel::{High, Low, Medium};

    match (current, candidate) {
        (Low, _) | (_, Low) => Low,
        (Medium, _) | (_, Medium) => Medium,
        (High, High) => High,
    }
}
