use std::{
    collections::BTreeSet,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{
    LiveRecorder, SESSION_SCHEMA_VERSION,
    event_types::FrameEvent,
    session_files::{
        MetadataFile, RecordedConfig, RecordedCpuSnapshot, RecordedLatency, RecordedSpike,
        RecordedTime, SessionFile, SessionMetadataCore, SessionSpike, SessionTask, WakerEntry,
        focus_source_label, foreground_source_arg_label, foreground_source_label,
    },
    writers::NdjsonWriter,
};
use crate::{
    artifacts::ArtifactKind,
    config::{TARGET_PIDS_MAX, model::MonitorConfig},
    foreground::ForegroundEvent,
    metadata::collect_system_metadata,
    metrics::{CpuSnapshot, SpikeRecord, TaskStats},
};

#[derive(Debug)]
pub struct RecordingRun {
    pub run_name: Option<String>,
    pub run_dir: PathBuf,
    pub started_at: SystemTime,
    pub started_instant: Instant,
    pub monotonic_start_ns: Option<u64>,
    pub mangohud_start_offset: Option<u64>,
    pub mangohud_first_frame_monotonic_ns: Option<u64>,
    pub mangohud_first_frame_raw_elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuPerfStatus {
    pub sample_count: u64,
    pub active_counter_tasks: u64,
    pub skipped_counter_tasks: u64,
    pub open_errors: u64,
    pub read_errors: u64,
    pub last_error: Option<String>,
}

pub struct FinalizeRecordingInput<'a> {
    pub recorder: &'a LiveRecorder,
    pub config: &'a MonitorConfig,
    pub tree_pids: &'a [u32],
    pub stop_reason: &'a str,
    pub tasks: &'a crate::tasks::TaskTracker,
    pub frame_events: &'a [FrameEvent],
    pub block_io_correlation_basis: String,
    pub block_io_correlation_confidence: String,
    pub drop_counters: crate::ebpf_loader::DropCountersSnapshot,
    pub cpu_perf_status: Option<CpuPerfStatus>,
    pub focus_mode: Option<String>,
    pub final_focus_kind: Option<String>,
    pub focus_switch_count: u64,
    #[allow(dead_code)]
    pub current_focus: Option<crate::focus::ResolvedFocus>,
    pub final_foreground_event: Option<ForegroundEvent>,
}

pub fn prepare_recording(config: &MonitorConfig) -> anyhow::Result<Option<RecordingRun>> {
    let recording = &config.recording;
    if recording.run_name.is_none() && recording.output_dir.is_none() {
        return Ok(None);
    }

    let started_at = SystemTime::now();
    let run_dir = resolve_run_dir(recording, started_at, env::var_os("HOME"));
    if let Err(err) = ensure_empty_dir(&run_dir) {
        return Err(err.context("record write failed"));
    }

    Ok(Some(RecordingRun {
        run_name: recording.run_name.clone(),
        run_dir,
        started_at,
        started_instant: Instant::now(),
        monotonic_start_ns: monotonic_now_ns(),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    }))
}

pub fn recording_warnings(recorder: &LiveRecorder) -> Vec<String> {
    let mut warnings = Vec::new();

    if recorder.counters.intervals_dropped > 0 {
        warnings.push(format!(
            "warning: {} interval record(s) were dropped due to --retain-intervals; reports may not include full interval history",
            recorder.counters.intervals_dropped
        ));
    }

    if recorder.counters.spike_events_dropped_count > 0 {
        warnings.push(format!(
            "warning: {} spike event record(s) were dropped because the in-memory spike buffer was full; reports may not include every spike",
            recorder.counters.spike_events_dropped_count
        ));
    }

    if recorder.counters.event_stream_write_errors > 0 {
        let first_err_suffix = if let Some(first_error) =
            recorder.counters.first_event_stream_write_error.as_deref()
        {
            format!("; first error: {}", first_error)
        } else {
            "".to_owned()
        };
        warnings.push(format!(
            "warning: {} event stream write error(s) occurred while recording{}; one or more NDJSON artifact files may be incomplete",
            recorder.counters.event_stream_write_errors, first_err_suffix
        ));
    }

    if recorder.counters.process_scan_budget_exceeded_count > 0 {
        warnings.push(format!(
            "warning: process tree scan budget exceeded {} times; reports may be incomplete due to skipping task discovery",
            recorder.counters.process_scan_budget_exceeded_count
        ));
    }

    if recorder.counters.thread_scan_limited_count > 0 {
        warnings.push(format!(
            "warning: thread scan limit exceeded {} times; reports may be incomplete due to skipping thread discovery within massive processes",
            recorder.counters.thread_scan_limited_count
        ));
    }

    warnings
}

pub fn print_recording_warnings(recorder: &LiveRecorder) {
    for warning in recording_warnings(recorder) {
        eprintln!("{warning}");
    }
}

pub fn finalize_recording(input: FinalizeRecordingInput<'_>) -> anyhow::Result<()> {
    let FinalizeRecordingInput {
        recorder,
        config,
        tree_pids,
        stop_reason,
        tasks: task_tracker,
        frame_events,
        block_io_correlation_basis,
        block_io_correlation_confidence,
        drop_counters,
        cpu_perf_status,
        focus_mode,
        final_focus_kind,
        focus_switch_count,
        current_focus: _,
        final_foreground_event,
    } = input;

    let Some(recording) = recorder.run.as_ref() else {
        return Ok(());
    };

    let active_targets = &task_tracker.active_targets;
    let stats_by_task = &task_tracker.stats_by_task;
    let interval_records = &recorder.buffers.interval_records;
    let interval_record_count = recorder.counters.interval_record_count;
    let tree_events = &recorder.buffers.tree_events;
    let spike_events = recorder
        .buffers
        .spike_events
        .as_ref()
        .map(|s| s.events.as_slice())
        .unwrap_or(&[]);

    let irq_event_count = recorder.counters.irq_event_count;
    let gpu_sample_count = recorder.counters.gpu_sample_count;
    let ended_at = SystemTime::now();
    let monotonic_end_ns = monotonic_now_ns();
    let duration_ms = recording.started_instant.elapsed().as_millis() as u64;
    let metadata = collect_system_metadata();

    let mut active_expanded_tasks = active_targets.keys().copied().collect::<Vec<_>>();
    active_expanded_tasks.sort_unstable();

    let mut tasks = Vec::new();
    let mut top_spikes = Vec::new();

    for (task, stats) in stats_by_task {
        let mut session_latency = stats.session_latency.clone();
        let Some(latency) = session_latency.snapshot() else {
            continue;
        };

        let cpu = stats.session_cpu.snapshot();

        let (stat_wait_sum_ns, stat_wait_sum_ns_saturated) = if stats.stat_wait_count > 0 {
            let (sum, saturated) = saturating_u128_to_u64(stats.stat_wait_sum_ns);
            (Some(sum), saturated)
        } else {
            (None, false)
        };

        let stat_wait_count = if stats.stat_wait_count > 0 {
            Some(stats.stat_wait_count)
        } else {
            None
        };

        tasks.push(SessionTask {
            task: *task,
            active: stats.active,
            first_seen_ms: stats.first_seen_ms,
            last_seen_ms: stats.last_seen_ms,
            removed_ms: stats.removed_ms,
            class: stats.class,
            process_pid: stats.process_pid,
            process_comm: stats.process_comm.clone(),
            process_starttime_ticks: stats.process_starttime_ticks,
            task_starttime_ticks: stats.task_starttime_ticks,
            exe_dev: stats.exe_dev,
            exe_ino: stats.exe_ino,
            comm: stats.comm.clone(),
            latency: recorded_latency(latency),
            cpu: recorded_cpu(cpu),
            top_spikes: stats
                .top_spikes
                .iter()
                .map(|spike| recorded_spike(stats, spike))
                .collect(),
            migration_count: stats.migration_count,
            cross_numa_migrations: stats.cross_numa_migrations,
            top_wakers: stats
                .waker_counts
                .iter()
                .map(|(waker_tid, count)| WakerEntry {
                    waker_tid: *waker_tid,
                    waker_comm: stats_by_task
                        .get(waker_tid)
                        .map(|s| s.comm.clone())
                        .unwrap_or_else(|| "?".to_owned()),
                    count: *count,
                })
                .collect(),
            sched_policy: stats.sched_policy.map(|p| {
                crate::process_tree::sched_policy_name(p)
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| format!("UNKNOWN({})", p))
            }),
            stat_wait_sum_ns,
            stat_wait_sum_ns_saturated,
            stat_wait_count,
            cpu_perf: stats
                .session_cpu_perf
                .as_ref()
                .and_then(|perf| perf.snapshot()),
        });

        for spike in &stats.top_spikes {
            top_spikes.push(SessionSpike {
                task: *task,
                active: stats.active,
                class: stats.class,
                process_pid: stats.process_pid,
                process_comm: stats.process_comm.clone(),
                comm: stats.comm.clone(),
                cpu: spike.cpu,
                wakeup_target_cpu: spike.wakeup_target_cpu,
                prio: spike.prio,
                latency_ns: spike.latency_ns,
                wakeup_ns: spike.wakeup_ns,
                switch_ns: spike.switch_ns,
                switch_prev_pid: spike.switch_prev_pid,
                switch_prev_state: spike.switch_prev_state,
                switch_prev_state_label: spike.switch_prev_state_label.clone(),
                ..Default::default()
            });
        }
    }

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));
    top_spikes.sort_by_key(|spike| std::cmp::Reverse(spike.latency_ns));
    top_spikes.truncate(64);

    let core = SessionMetadataCore {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: recording.run_name.clone(),
        started_at: recorded_time(recording.started_at),
        ended_at: recorded_time(ended_at),
        monotonic_start_ns: recording.monotonic_start_ns,
        monotonic_end_ns,
        duration_ms,
        mangohud_start_offset: recording.mangohud_start_offset,
        mangohud_first_frame_monotonic_ns: recording.mangohud_first_frame_monotonic_ns,
        mangohud_first_frame_raw_elapsed_ms: recording.mangohud_first_frame_raw_elapsed_ms,
        metadata,
        target_pids_max: TARGET_PIDS_MAX as u64,
        active_target_pids_count: active_targets.len() as u64,
        active_expanded_tasks,
        focus_mode,
        final_focus_kind,
        focus_switch_count,
        focus_event_count: recorder.counters.focus_event_count,
        foreground_event_count: recorder.counters.foreground_event_count,
        foreground_source: final_foreground_event
            .as_ref()
            .map(|event| foreground_source_label(event.source)),
        final_foreground_pid: final_foreground_event.as_ref().and_then(|event| event.pid),
        final_foreground_app_id: final_foreground_event
            .as_ref()
            .and_then(|event| event.app_id.clone()),
        final_foreground_class: final_foreground_event
            .as_ref()
            .and_then(|event| event.class.clone()),
        interval_record_count,
        intervals_dropped: recorder.counters.intervals_dropped,
        spike_events_retained_count: if recorder.streams.contains(ArtifactKind::SpikeEvents) {
            recorder.counters.spike_event_count
        } else {
            spike_events.len() as u64
        },
        spike_events_dropped_count: recorder.counters.spike_events_dropped_count,
        spike_events_truncated: if recorder.streams.contains(ArtifactKind::SpikeEvents) {
            false
        } else {
            recorder
                .buffers
                .spike_events
                .as_ref()
                .map(|s| s.truncated)
                .unwrap_or(false)
        },
        scx_event_count: recorder.counters.scx_event_count,
        irq_event_count,
        migration_event_count: Some(recorder.counters.migration_event_count),
        cpu_freq_sample_count: Some(recorder.counters.cpu_freq_sample_count),
        gpu_sample_count,
        frame_event_count: if recorder.streams.contains(ArtifactKind::FrameEvents) {
            recorder.counters.frame_event_count
        } else {
            frame_events.len() as u64
        },
        block_io_event_count: recorder.counters.block_io_event_count,
        runtime_slice_count: recorder.counters.runtime_slice_count,
        runtime_slice_read_errors: recorder.counters.runtime_slice_read_errors,
        runtime_slice_skipped_tasks: recorder.counters.runtime_slice_skipped_tasks,
        runtime_slice_source: if recorder.counters.runtime_slice_count > 0 {
            Some("procfs".to_owned())
        } else {
            None
        },
        event_stream_write_errors: recorder.counters.event_stream_write_errors,
        alert_events_dropped_count: recorder.counters.alert_events_dropped_count,
        alert_channel_closed_count: recorder.counters.alert_channel_closed_count,
        first_event_stream_write_error: recorder.counters.first_event_stream_write_error.clone(),
        block_io_correlation_basis: block_io_correlation_basis.clone(),
        block_io_correlation_confidence: block_io_correlation_confidence.clone(),
        drop_counters: drop_counters.clone(),
        cpu_perf_sample_count: cpu_perf_status
            .as_ref()
            .map(|status| status.sample_count)
            .unwrap_or(0),
        cpu_perf_open_errors: cpu_perf_status
            .as_ref()
            .map(|status| status.open_errors)
            .unwrap_or(0),
        cpu_perf_read_errors: cpu_perf_status
            .as_ref()
            .map(|status| status.read_errors)
            .unwrap_or(0),
        cpu_perf_skipped_tasks: cpu_perf_status
            .as_ref()
            .map(|status| status.skipped_counter_tasks)
            .unwrap_or(0),
        cpu_perf_last_error: cpu_perf_status
            .as_ref()
            .and_then(|status| status.last_error.clone()),
    };

    let session = SessionFile {
        core: core.clone(),
        stop_reason: stop_reason.to_owned(),
        config: recorded_config(config, tree_pids),
        tasks,
        top_spikes,
    };

    let metadata_file = MetadataFile { core };

    let map_write_err = |e: anyhow::Error| -> anyhow::Error { e.context("record write failed") };

    let mut sync_tracker = SyncTracker::default();

    write_json(
        recording.run_dir.join("session.json"),
        &session,
        &mut sync_tracker,
    )
    .map_err(map_write_err)?;
    write_json(
        recording.run_dir.join("metadata.json"),
        &metadata_file,
        &mut sync_tracker,
    )
    .map_err(map_write_err)?;

    if !recorder.streams.contains(ArtifactKind::Interval) {
        write_json_stream(
            recording.run_dir.join("interval.json"),
            interval_records,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !tree_events.is_empty() {
        write_json_stream(
            recording.run_dir.join("tree_events.json"),
            tree_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::SpikeEvents) && !spike_events.is_empty() {
        write_json_stream(
            recording.run_dir.join("spike_events.json"),
            spike_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::IrqEvents)
        && !recorder.buffers.irq_events.is_empty()
    {
        write_json_stream(
            recording.run_dir.join("irq_events.json"),
            &recorder.buffers.irq_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::GpuSamples)
        && !recorder.buffers.gpu_samples.is_empty()
    {
        write_json_stream(
            recording.run_dir.join("gpu_samples.json"),
            &recorder.buffers.gpu_samples,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::FrameEvents) && !frame_events.is_empty() {
        write_json_stream(
            recording.run_dir.join("frame_events.json"),
            frame_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::ScxEvents)
        && !recorder.buffers.scx_events.is_empty()
    {
        write_json_stream(
            recording.run_dir.join("scx_events.json"),
            &recorder.buffers.scx_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }

    if !input.config.outputs.json_stream {
        println!("recording written to {}", recording.run_dir.display());
    }
    Ok(())
}

pub fn recorded_config(config: &MonitorConfig, tree_pids: &[u32]) -> RecordedConfig {
    RecordedConfig {
        manual_pids: config.target.target_pids.clone(),
        tree_roots: tree_pids.to_vec(),
        cgroupv2: config.target.cgroupv2.clone(),
        exclude_tree_pids: config.target.exclude_tree_pids.clone(),
        include_comm: config
            .target
            .task_filters
            .include_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        exclude_comm: config
            .target
            .task_filters
            .exclude_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        watch_process: config.target.watch_process.clone(),
        persistent: config.target.persistent,
        keep_missing_pid: config.target.keep_missing_pid,
        watch_poll_ms: config.watch.poll_ms,
        watch_timeout_ms: config
            .watch
            .timeout
            .map(|timeout| timeout.as_millis() as u64),
        csv_stream: config.streams.csv.clone(),
        irq_latency: config.probes.irq_latency,
        irqs: config.probes.irqs.clone(),
        hwmon: config.probes.hwmon,
        hwmon_root: config.hwmon.root.clone(),
        hwmon_drm_card: config.hwmon.drm_card.clone(),
        hwmon_render_node: config.hwmon.render_node.clone(),
        mangohud_log: config.mangohud.log.clone(),
        mangohud_log_live: config.mangohud.log_live,
        tui: config.ui.tui,
        summary_period_ms: config.timing.summary_period_ms,
        epoch_period_ms: config.timing.epoch_period_ms,
        retain_intervals: config.recording.retain_intervals,
        max_tasks: config.target.max_tasks,
        spike_threshold_ns: config.timing.spike_threshold_ns,
        alert_threshold_ns: config.alerts.threshold_ns,
        alert_webhook_url: config.alerts.webhook_url.clone(),
        follow_exec: config.safety.follow_exec,
        verbose: config.streams.verbose,
        faults: config.probes.faults,
        cpu_perf: config.probes.cpu_perf,
        cpu_perf_kernel: config.cpu_perf.include_kernel,
        cpu_perf_max_tasks: config.cpu_perf.max_tasks,
        cpu_perf_cache_refs: config.cpu_perf.collect_cache_refs,
        block_io: config.probes.block_io,
        stat_wait: config.probes.stat_wait,
        runtime_slices: config.probes.runtime_slices,
        runtime_slices_max_tasks: config.runtime_slices.max_tasks,
        otlp_endpoint: config.outputs.otlp_endpoint.clone(),
        otel_service_name: config.outputs.otel_service_name.clone(),
        auto_focus: config.focus.auto_focus,
        foreground_window: config.focus.foreground_window,
        focus_source: focus_source_label(config.focus.focus_source),
        foreground_source: foreground_source_arg_label(config.focus.foreground_source),
        foreground_poll_ms: config.focus.foreground_poll_ms,
        foreground_max_stale_ms: config.focus.foreground_max_stale_ms,
        foreground_include_title: config.focus.foreground_include_title,
        auto_focus_poll_ms: config.focus.auto_focus_poll_ms,
        auto_focus_min_confidence: config.focus.auto_focus_min_confidence,
        auto_focus_switch_cooldown_ms: config.focus.auto_focus_switch_cooldown_ms,
        auto_focus_switch_margin: config.focus.auto_focus_switch_margin,
        auto_focus_required_polls: config.focus.auto_focus_required_polls,
        auto_focus_max_roots: config.focus.auto_focus_max_roots,
    }
}

pub(crate) fn elapsed_ms_from_monotonic(
    monotonic_start_ns: Option<u64>,
    switch_ns: u64,
) -> Option<u64> {
    let start_ns = monotonic_start_ns?;
    switch_ns
        .checked_sub(start_ns)
        .map(|elapsed_ns| elapsed_ns / 1_000_000)
}

pub(crate) fn saturating_u128_to_u64(value: u128) -> (u64, bool) {
    if value > u64::MAX as u128 {
        (u64::MAX, true)
    } else {
        (value as u64, false)
    }
}

fn recorded_latency(latency: crate::metrics::LatencySnapshot) -> RecordedLatency {
    RecordedLatency {
        samples: latency.count,
        stored_samples: latency.stored_samples,
        truncated_samples: latency.samples_truncated,
        percentile_scope: latency.percentile_scope,
        histogram: latency.histogram,
        min_ns: latency.min_ns,
        avg_ns: latency.avg_ns,
        p95_ns: latency.p95_ns,
        p99_ns: latency.p99_ns,
        max_ns: latency.max_ns,
        over_1ms: latency.over_1ms,
        over_2ms: latency.over_2ms,
        over_5ms: latency.over_5ms,
    }
}

fn recorded_cpu(cpu: CpuSnapshot) -> RecordedCpuSnapshot {
    RecordedCpuSnapshot {
        busiest_cpu: cpu.busiest_cpu,
        busiest_cpu_samples: cpu.busiest_cpu_samples,
        worst_cpu: cpu.worst_cpu,
        worst_cpu_max_ns: cpu.worst_cpu_max_ns,
        spikiest_cpu: cpu.spikiest_cpu,
        spikiest_cpu_spikes: cpu.spikiest_cpu_spikes,
        per_cpu: cpu.per_cpu,
    }
}

fn recorded_spike(stats: &TaskStats, spike: &SpikeRecord) -> RecordedSpike {
    RecordedSpike {
        class: stats.class,
        process_pid: stats.process_pid,
        process_comm: stats.process_comm.clone(),
        cpu: spike.cpu,
        wakeup_target_cpu: spike.wakeup_target_cpu,
        switch_prev_pid: spike.switch_prev_pid,
        switch_prev_state: spike.switch_prev_state,
        switch_prev_state_label: spike.switch_prev_state_label.clone(),
        prio: spike.prio,
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        waker_tid: 0, // Not currently persisted in SpikeRecord
        waker_comm: String::new(),
        target_pending_wakeups: spike.target_pending_wakeups,
        observed_runnable_depth: spike.observed_runnable_depth,
        major_faults: spike.major_faults,
        minor_faults: spike.minor_faults,
        scx_ops: spike.scx_ops.clone(),
        scx_state: spike.scx_state.clone(),
        scx_enable_seq: spike.scx_enable_seq.clone(),
        cause_tags: spike.cause_tags.clone(),
        primary_cause: spike.primary_cause.clone(),
    }
}

fn resolve_run_dir(
    recording: &crate::config::model::RecordingConfig,
    started_at: SystemTime,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(out_dir) = &recording.output_dir {
        return out_dir.clone();
    }

    let mut base = home
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push(".local");
    base.push("state");
    base.push("stutter");
    base.push("runs");

    let run_name = recording.run_name.as_deref().unwrap_or("run");
    base.push(format!(
        "{}_{}",
        timestamp_for_path(started_at),
        sanitize_run_name(run_name)
    ));
    base
}

fn ensure_empty_dir(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::bail!("output directory already exists: {}", path.display());
    }

    fs::create_dir_all(path)?;
    Ok(())
}

pub fn recorded_time(time: SystemTime) -> RecordedTime {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();

    RecordedTime {
        unix_seconds: duration.as_secs(),
        unix_nanos: duration.subsec_nanos(),
        system_time_debug: format!("{time:?}"),
    }
}

fn timestamp_for_path(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}_{:09}", duration.as_secs(), duration.subsec_nanos())
}

fn sanitize_run_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn monotonic_now_ns() -> Option<u64> {
    static CLOCK_ID: std::sync::OnceLock<libc::clockid_t> = std::sync::OnceLock::new();
    let clock_id = CLOCK_ID.get_or_init(|| {
        if is_kernel_before_5_7() {
            libc::CLOCK_MONOTONIC_RAW
        } else {
            libc::CLOCK_MONOTONIC
        }
    });

    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // SAFETY: clock_gettime writes to the provided valid timespec pointer and
    // does not retain it after the call. We select CLOCK_MONOTONIC or
    // CLOCK_MONOTONIC_RAW based on the kernel version to match bpf_ktime_get_ns()
    // behavior, so recorded elapsed times line up with eBPF timestamps.
    let result = unsafe { libc::clock_gettime(*clock_id, &mut timespec) };
    if result != 0 {
        return None;
    }

    timespec_to_ns(timespec)
}

fn is_kernel_before_5_7() -> bool {
    let mut uts = std::mem::MaybeUninit::<libc::utsname>::uninit();
    // SAFETY: uts is a valid pointer to a libc::utsname struct.
    if unsafe { libc::uname(uts.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: uname succeeded and initialized the struct.
    let uts = unsafe { uts.assume_init() };
    // SAFETY: release field is a null-terminated string.
    let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) };
    let release_str = release.to_string_lossy();

    let mut parts = release_str.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    major < 5 || (major == 5 && minor < 7)
}

fn timespec_to_ns(timespec: libc::timespec) -> Option<u64> {
    if timespec.tv_sec < 0 || timespec.tv_nsec < 0 {
        return None;
    }

    let seconds = u64::try_from(timespec.tv_sec).ok()?;
    let nanos = u64::try_from(timespec.tv_nsec).ok()?;
    if nanos >= 1_000_000_000 {
        return None;
    }

    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}

#[derive(Debug, Default)]
pub struct SyncTracker {
    synced_dirs: BTreeSet<PathBuf>,
}

impl SyncTracker {
    pub fn sync_parent_once(&mut self, path: &Path) -> anyhow::Result<()> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };

        let parent = parent.to_path_buf();

        if self.synced_dirs.insert(parent.clone()) {
            let dir = fs::File::open(&parent).with_context(|| {
                format!(
                    "failed to open parent directory {} for sync",
                    parent.display()
                )
            })?;

            dir.sync_all()
                .with_context(|| format!("failed to sync parent directory {}", parent.display()))?;
        }

        Ok(())
    }

    #[cfg(test)]
    fn synced_dir_count_for_test(&self) -> usize {
        self.synced_dirs.len()
    }

    #[cfg(test)]
    fn mark_parent_for_test(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            self.synced_dirs.insert(parent.to_path_buf());
        }
    }
}

fn write_json<T: ?Sized + Serialize>(
    path: PathBuf,
    value: &T,
    sync_tracker: &mut SyncTracker,
) -> anyhow::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| {
            if name.is_empty() {
                None
            } else {
                Some(name.to_string_lossy())
            }
        })
        .ok_or_else(|| anyhow::anyhow!("JSON destination has no file name: {}", path.display()))?;
    let tmp_path = path.with_file_name(format!("{file_name}.tmp"));
    let mut file = fs::File::create(&tmp_path)
        .with_context(|| format!("failed to create temp JSON {}", tmp_path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("failed to write temp JSON {}", tmp_path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to finalize temp JSON {}", tmp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync temp JSON {}", tmp_path.display()))?;
    drop(file);
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("failed to rename temp JSON {}", tmp_path.display()))?;

    sync_tracker.sync_parent_once(&path)?;

    Ok(())
}

fn write_json_stream<T: Serialize>(
    path: PathBuf,
    values: &[T],
    sync_tracker: &mut SyncTracker,
) -> anyhow::Result<()> {
    let mut writer = NdjsonWriter::create(path.clone())?;
    for value in values {
        writer.push(value)?;
    }
    writer.finish()?;

    sync_tracker.sync_parent_once(&path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{super::RecordingCounters, *};

    #[test]
    fn write_json_rejects_path_without_file_name() {
        let err = write_json(
            PathBuf::from("/"),
            &serde_json::json!({}),
            &mut SyncTracker::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no file name"));
    }

    #[test]
    fn spike_point_preserves_switch_prev_context() {
        let stats = crate::metrics::TaskStats::new(42, "t".to_owned(), 0);
        let spike = crate::metrics::SpikeRecord {
            latency_ns: 100,
            cpu: 1,
            wakeup_target_cpu: 0,
            prio: 0,
            wakeup_ns: 10,
            switch_ns: 110,
            switch_prev_pid: 99,
            switch_prev_state: 1,
            switch_prev_state_label: "voluntary_sleep_interruptible".to_owned(),
            ..crate::metrics::SpikeRecord::default()
        };

        let rec = recorded_spike(&stats, &spike);
        assert_eq!(rec.switch_prev_pid, 99);
        assert_eq!(rec.switch_prev_state, 1);
    }

    #[test]
    fn recording_warnings_include_intervals_dropped() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                intervals_dropped: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("3 interval record(s) were dropped"));
        assert!(warnings[0].contains("--retain-intervals"));
    }

    #[test]
    fn recording_warnings_include_spike_events_dropped() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                spike_events_dropped_count: 2,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("2 spike event record(s)"));
    }

    #[test]
    fn recording_warnings_include_event_stream_write_errors() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                event_stream_write_errors: 4,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("4 event stream write error(s)"));
        assert!(warnings[0].contains("incomplete"));
    }

    #[test]
    fn recording_warnings_include_all_recording_problems() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                intervals_dropped: 1,
                spike_events_dropped_count: 2,
                event_stream_write_errors: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn recording_warnings_empty_for_clean_recorder() {
        let recorder = LiveRecorder::default();

        let warnings = recording_warnings(&recorder);

        assert!(warnings.is_empty());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }

    #[test]
    fn sync_tracker_tracks_parent_once_for_same_directory() {
        let mut tracker = SyncTracker::default();

        tracker.mark_parent_for_test(Path::new("run-a/session.json"));
        tracker.mark_parent_for_test(Path::new("run-a/metadata.json"));

        assert_eq!(tracker.synced_dir_count_for_test(), 1);
    }

    #[test]
    fn sync_tracker_tracks_distinct_parent_directories() {
        let mut tracker = SyncTracker::default();

        tracker.mark_parent_for_test(Path::new("run-a/session.json"));
        tracker.mark_parent_for_test(Path::new("run-b/session.json"));

        assert_eq!(tracker.synced_dir_count_for_test(), 2);
    }

    #[test]
    fn sync_parent_once_does_not_error_for_existing_parent() {
        let dir = temp_dir("sync-tracker");
        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("session.json");
        fs::write(&path, "{}\n").unwrap();

        let mut tracker = SyncTracker::default();
        tracker.sync_parent_once(&path).unwrap();
        tracker
            .sync_parent_once(&dir.join("metadata.json"))
            .unwrap();

        assert_eq!(tracker.synced_dir_count_for_test(), 1);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn live_recorder_writes_foreground_event_to_dedicated_stream() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-foreground-event-stream-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("foreground_events.json");

        let mut recorder = LiveRecorder::default();
        recorder
            .streams
            .create_stream(&dir, ArtifactKind::ForegroundEvents)
            .unwrap();

        let event = ForegroundEvent {
            elapsed_ms: 42,
            source: crate::foreground::ForegroundSource::X11,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(1000),
            app_id: Some("Navigator".to_owned()),
            class: Some("Firefox".to_owned()),
            title: None,
            window_id: Some("0x1200007".to_owned()),
            workspace: None,
            confidence: 0.90,
            reason: "active X11 window from xprop".to_owned(),
        };

        recorder.write_foreground_event(event.clone()).unwrap();
        recorder.streams.finish_all().unwrap();

        assert_eq!(recorder.counters.foreground_event_count, 1);
        assert_eq!(
            recorder.last_foreground_event.as_ref().unwrap().pid,
            Some(1000)
        );

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"source\":\"x11\""));
        assert!(text.contains("\"pid\":1000"));
        assert!(!text.contains("focus"));

        std::fs::remove_dir_all(dir).ok();
    }
}
