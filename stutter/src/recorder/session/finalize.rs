use std::{cmp::Reverse, time::SystemTime};

use super::{
    super::{
        SESSION_SCHEMA_VERSION, SyncTracker,
        retention::{RecordingRetentionPolicy, apply_recording_retention},
        session_files::{
            MetadataFile, SessionFile, SessionMetadataCore, SessionSpike, SessionTask, WakerEntry,
        },
    },
    FinalizeRecordingInput,
    metadata::{
        display_path_metadata, load_display_topology_snapshot, monotonic_now_ns, recorded_config,
        recorded_cpu, recorded_latency, recorded_spike, recorded_time, saturating_u128_to_u64,
    },
    writers::{write_json, write_json_stream},
};
use crate::{
    artifacts::{ArtifactKind, artifact_path},
    config::TARGET_PIDS_MAX,
    metadata::collect_system_metadata,
};

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
        native_cgroup_filter,
        probe_activation_warnings,
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
    let display_topology = load_display_topology_snapshot(&recording.run_dir);

    let mut active_expanded_tasks = active_targets
        .keys()
        .map(|tid| tid.as_u32())
        .collect::<Vec<_>>();
    active_expanded_tasks.sort_unstable();

    let mut tasks = Vec::new();
    let mut top_spikes = Vec::new();

    for stats in stats_by_task.values() {
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

        let allowed_cpus = crate::affinity::read_allowed_mask(stats.task_id())
            .ok()
            .map(|mask| mask.to_range_string());

        tasks.push(SessionTask {
            task: stats.task_id().as_u32(),
            active: stats.active,
            first_seen_ms: stats.first_seen_ms,
            last_seen_ms: stats.last_seen_ms,
            removed_ms: stats.removed_ms,
            class: stats.class,
            process_pid: stats.process_id().map(|pid| pid.as_u32()),
            process_comm: stats.process_comm.clone(),
            process_starttime_ticks: stats.process_starttime_ticks,
            task_starttime_ticks: stats.task_starttime_ticks,
            exe_dev: stats.exe_dev,
            exe_ino: stats.exe_ino,
            comm: stats.comm.clone(),
            allowed_cpus,
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
                    waker_tid: waker_tid.as_u32(),
                    waker_comm: stats_by_task
                        .get(&waker_tid.as_u32())
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
                task: stats.task_id().as_u32(),
                active: stats.active,
                class: stats.class,
                process_pid: stats.process_id().map(|pid| pid.as_u32()),
                process_comm: stats.process_comm.clone(),
                comm: stats.comm.clone(),
                cpu: spike.cpu.as_u32(),
                wakeup_target_cpu: spike.wakeup_target_cpu.as_u32(),
                prio: spike.prio,
                latency_ns: spike.latency_ns,
                wakeup_ns: spike.wakeup_ns,
                switch_ns: spike.switch_ns,
                switch_prev_pid: spike.switch_prev_pid.as_u32(),
                switch_prev_state: spike.switch_prev_state,
                switch_prev_state_label: spike.switch_prev_state_label.clone(),
                ..Default::default()
            });
        }
    }

    tasks.sort_by_key(|task| Reverse(task.latency.max_ns));
    top_spikes.sort_by_key(|spike| Reverse(spike.latency_ns));
    top_spikes.truncate(64);

    let core = SessionMetadataCore {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: recording.run_name.clone(),
        scenario_name: config.recording.scenario_name.clone(),
        scenario_hash: config.recording.scenario_hash.clone(),
        workload_label: config.recording.workload_label.clone(),
        route_label: config.recording.route_label.clone(),
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
        kms_flip_event_count: recorder.counters.kms_flip_event_count,
        drm_fence_event_count: recorder.counters.drm_fence_event_count,
        wayland_presentation_event_count: recorder.counters.wayland_presentation_event_count,
        dmabuf_event_count: recorder.counters.dmabuf_event_count,
        gpu_engine_sample_count: recorder.counters.gpu_engine_sample_count,
        display_path: display_path_metadata(config, display_topology.as_ref()),
        foreground_source: final_foreground_event
            .as_ref()
            .map(|event| super::super::session_files::foreground_source_label(event.source)),
        final_foreground_pid: final_foreground_event
            .as_ref()
            .and_then(|event| event.decision.target.as_ref().and_then(|t| t.pid)),
        final_foreground_app_id: final_foreground_event.as_ref().and_then(|event| {
            event
                .decision
                .target
                .as_ref()
                .and_then(|t| t.app_id.clone())
                .clone()
        }),
        final_foreground_class: final_foreground_event.as_ref().and_then(|event| {
            event
                .decision
                .target
                .as_ref()
                .and_then(|t| t.class.clone())
                .clone()
        }),
        final_foreground_status: final_foreground_event
            .as_ref()
            .map(|event| format!("{:?}", event.status).to_ascii_lowercase()),
        final_foreground_window_id: final_foreground_event.as_ref().and_then(|event| {
            event
                .decision
                .target
                .as_ref()
                .and_then(|t| t.window_id.clone())
                .clone()
        }),
        final_foreground_workspace: final_foreground_event.as_ref().and_then(|event| {
            event
                .decision
                .target
                .as_ref()
                .and_then(|t| t.workspace.clone())
                .clone()
        }),
        final_foreground_confidence: final_foreground_event
            .as_ref()
            .map(|event| event.decision.confidence),
        final_foreground_stale_ms: final_foreground_event
            .as_ref()
            .and_then(|event| event.stale_ms),
        final_foreground_reason: final_foreground_event.as_ref().map(|event| {
            event
                .decision
                .reasons
                .first()
                .map(|r| r.reason.clone())
                .unwrap_or_default()
                .clone()
        }),
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
        block_io_correlation_basis,
        block_io_correlation_confidence,
        native_cgroup_filter,
        probe_activation_warnings,
        drop_counters,
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
        artifact_path(&recording.run_dir, ArtifactKind::Session),
        &session,
        &mut sync_tracker,
    )
    .map_err(map_write_err)?;
    write_json(
        artifact_path(&recording.run_dir, ArtifactKind::Metadata),
        &metadata_file,
        &mut sync_tracker,
    )
    .map_err(map_write_err)?;
    if !recorder.streams.contains(ArtifactKind::Interval) {
        write_json_stream(
            artifact_path(&recording.run_dir, ArtifactKind::Interval),
            interval_records,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !tree_events.is_empty() {
        write_json_stream(
            artifact_path(&recording.run_dir, ArtifactKind::TreeEvents),
            tree_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::SpikeEvents) && !spike_events.is_empty() {
        write_json_stream(
            artifact_path(&recording.run_dir, ArtifactKind::SpikeEvents),
            spike_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::IrqEvents)
        && !recorder.buffers.irq_events.is_empty()
    {
        write_json_stream(
            artifact_path(&recording.run_dir, ArtifactKind::IrqEvents),
            &recorder.buffers.irq_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::GpuSamples)
        && !recorder.buffers.gpu_samples.is_empty()
    {
        write_json_stream(
            artifact_path(&recording.run_dir, ArtifactKind::GpuSamples),
            &recorder.buffers.gpu_samples,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::FrameEvents) && !frame_events.is_empty() {
        write_json_stream(
            artifact_path(&recording.run_dir, ArtifactKind::FrameEvents),
            frame_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::ScxEvents)
        && !recorder.buffers.scx_events.is_empty()
    {
        write_json_stream(
            artifact_path(&recording.run_dir, ArtifactKind::ScxEvents),
            &recorder.buffers.scx_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }

    if !config.outputs.json_stream {
        println!("recording written to {}", recording.run_dir.display());
    }

    let retention_policy = RecordingRetentionPolicy::from_recording_config(&config.recording);
    if config.recording.output_dir.is_none()
        && let Some(run_root) = recording.run_dir.parent()
        && let Err(err) = apply_recording_retention(
            run_root,
            &retention_policy,
            Some(&recording.run_dir),
            SystemTime::now(),
        )
    {
        log::warn!(
            "recording_retention_finalize_failed run_root={} err={err:#}",
            run_root.display()
        );
    }

    Ok(())
}
