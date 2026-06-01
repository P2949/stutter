//! Low-level helpers for constructing session, spike, interval, and artifact records.

use super::*;

pub(in crate::test_fixture_builder) fn renamed_fixture(
    name: &str,
    (mut session, artifacts): (SessionFile, FixtureArtifacts),
) -> (SessionFile, FixtureArtifacts) {
    session.core.run_name = Some(name.to_owned());
    (session, artifacts)
}

pub(in crate::test_fixture_builder) fn apply_artifact_counts(
    session: &mut SessionFile,
    artifacts: &FixtureArtifacts,
) -> (SessionFile, FixtureArtifacts) {
    session.core.spike_events_retained_count = artifacts.spikes.len() as u64;
    session.core.interval_record_count = artifacts.intervals.len() as u64;
    session.core.irq_event_count = artifacts.irq_events.len() as u64;
    session.core.gpu_sample_count = artifacts.gpu_samples.len() as u64;
    session.core.frame_event_count = artifacts.frame_events.len() as u64;
    session.core.block_io_event_count = artifacts.block_io_events.len() as u64;
    session.core.foreground_event_count = artifacts.foreground_events.len() as u64;
    session.core.kms_flip_event_count = artifacts.kms_flip_events.len() as u64;
    session.core.drm_fence_event_count = artifacts.drm_fence_events.len() as u64;
    session.core.wayland_presentation_event_count =
        artifacts.wayland_presentation_events.len() as u64;
    session.core.dmabuf_event_count = artifacts.dmabuf_events.len() as u64;
    session.core.gpu_engine_sample_count = artifacts.gpu_engine_samples.len() as u64;
    (
        session.clone(),
        FixtureArtifacts {
            spikes: artifacts.spikes.clone(),
            intervals: artifacts.intervals.clone(),
            irq_events: artifacts.irq_events.clone(),
            gpu_samples: artifacts.gpu_samples.clone(),
            frame_events: artifacts.frame_events.clone(),
            block_io_events: artifacts.block_io_events.clone(),
            foreground_events: artifacts.foreground_events.clone(),
            kms_flip_events: artifacts.kms_flip_events.clone(),
            drm_fence_events: artifacts.drm_fence_events.clone(),
            wayland_presentation_events: artifacts.wayland_presentation_events.clone(),
            dmabuf_events: artifacts.dmabuf_events.clone(),
            gpu_engine_samples: artifacts.gpu_engine_samples.clone(),
            display_topology: artifacts.display_topology.clone(),
        },
    )
}

pub(in crate::test_fixture_builder) fn unknown_clustered_spikes(
    anchor_latency_ns: u64,
) -> Vec<SpikeEvent> {
    vec![
        spike_event(100, TaskClass::Unknown, "worker-a", anchor_latency_ns, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ]
}

pub(in crate::test_fixture_builder) fn spike_event(
    task: u32,
    class: TaskClass,
    comm: &str,
    latency_ns: u64,
    offset_ns: u64,
) -> SpikeEvent {
    let switch_ns = 100_000_000 + offset_ns;
    SpikeEvent {
        elapsed_ms: Some(100),
        task: task.into(),
        active: true,
        class,
        process_pid: Some(task.into()),
        process_comm: comm.into(),
        comm: comm.to_owned(),
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        latency_ns,
        wakeup_ns: switch_ns.saturating_sub(latency_ns),
        switch_ns,
        switch_prev_pid: 0.into(),
        switch_prev_state: 0,
        switch_prev_state_label: "running".to_owned(),
        ..Default::default()
    }
}

pub(in crate::test_fixture_builder) fn interval_record(
    elapsed_ms: u64,
    task: u32,
    comm: &str,
    cpu_psi_some: f64,
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task,
        active: true,
        class: TaskClass::Unknown,
        comm: comm.to_owned(),
        process_pid: Some(task),
        process_comm: comm.into(),
        samples: 10,
        stored_samples: 10,
        truncated_samples: 0,
        min_ns: 100,
        avg_ns: 500,
        p95_ns: 1_000,
        p99_ns: 2_000,
        max_ns: 5_000,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: 0,
        busiest_cpu: Some(0),
        busiest_cpu_samples: 10,
        worst_cpu: Some(0),
        worst_cpu_max_ns: 5_000,
        spikiest_cpu: Some(0),
        spikiest_cpu_spikes: 0,
        major_faults: 0,
        minor_faults: 0,
        cpu_psi_some,
        mem_psi_some: 0.0,
        mem_psi_full: 0.0,
        mem_psi_delta_us: 0,
        mem_psi_spike: false,
        io_psi_some: 0.0,
        io_psi_full: 0.0,
        percentile_scope: "exact".to_owned(),
        histogram: vec![],
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf: None,
    }
}

pub(in crate::test_fixture_builder) fn interval_record_with_class(
    elapsed_ms: u64,
    task: u32,
    comm: &str,
    class: TaskClass,
    cpu_psi_some: f64,
    max_ns: u64,
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task,
        active: true,
        class,
        comm: comm.to_owned(),
        process_pid: Some(task),
        process_comm: comm.into(),
        samples: 10,
        stored_samples: 10,
        truncated_samples: 0,
        min_ns: 100,
        avg_ns: max_ns / 4,
        p95_ns: max_ns.saturating_sub(250_000),
        p99_ns: max_ns.saturating_sub(1),
        max_ns,
        over_1ms: u64::from(max_ns > 1_000_000),
        over_2ms: u64::from(max_ns > 2_000_000),
        over_5ms: u64::from(max_ns > 5_000_000),
        busiest_cpu: Some(0),
        busiest_cpu_samples: 10,
        worst_cpu: Some(0),
        worst_cpu_max_ns: max_ns,
        spikiest_cpu: Some(0),
        spikiest_cpu_spikes: u64::from(max_ns > 1_000_000),
        major_faults: 0,
        minor_faults: 0,
        cpu_psi_some,
        mem_psi_some: 0.0,
        mem_psi_full: 0.0,
        mem_psi_delta_us: 0,
        mem_psi_spike: false,
        io_psi_some: 0.0,
        io_psi_full: 0.0,
        percentile_scope: "exact".to_owned(),
        histogram: vec![],
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf: None,
    }
}

pub(in crate::test_fixture_builder) fn apply_spike_session_fields(
    session: &mut SessionFile,
    spikes: &[SpikeEvent],
) {
    session.core.active_target_pids_count = spikes.len() as u64;
    session.core.active_expanded_tasks = spikes.iter().map(|spike| spike.task.as_u32()).collect();
    session.tasks = spikes
        .iter()
        .map(|spike| {
            task_for_fixture(
                spike.task.as_u32(),
                spike.class,
                &spike.comm,
                10,
                spike.latency_ns,
            )
        })
        .collect();
}

pub(in crate::test_fixture_builder) fn task_for_fixture(
    task: u32,
    class: TaskClass,
    comm: &str,
    samples: u64,
    max_latency_ns: u64,
) -> SessionTask {
    let over_1ms = u64::from(max_latency_ns > 1_000_000);
    let over_2ms = u64::from(max_latency_ns > 2_000_000);
    let over_5ms = u64::from(max_latency_ns > 5_000_000);

    SessionTask {
        task,
        active: true,
        first_seen_ms: 0,
        last_seen_ms: 1000,
        removed_ms: None,
        class,
        process_pid: Some(task),
        process_comm: comm.into(),
        process_starttime_ticks: Some(1_000 + u64::from(task)),
        task_starttime_ticks: Some(2_000 + u64::from(task)),
        exe_dev: Some(10),
        exe_ino: Some(10_000 + u64::from(task)),
        comm: comm.to_owned(),
        allowed_cpus: None,
        latency: RecordedLatency {
            samples,
            stored_samples: samples,
            truncated_samples: 0,
            percentile_scope: "exact".to_owned(),
            histogram: vec![],
            min_ns: 100,
            avg_ns: 500,
            p95_ns: max_latency_ns / 2,
            p99_ns: max_latency_ns.saturating_sub(1),
            max_ns: max_latency_ns,
            over_1ms,
            over_2ms,
            over_5ms,
        },
        cpu: RecordedCpuSnapshot {
            busiest_cpu: Some(0),
            busiest_cpu_samples: samples,
            worst_cpu: Some(0),
            worst_cpu_max_ns: max_latency_ns,
            spikiest_cpu: Some(0),
            spikiest_cpu_spikes: over_1ms,
            per_cpu: vec![],
        },
        top_spikes: vec![],
        migration_count: 0,
        cross_numa_migrations: 0,
        top_wakers: vec![],
        sched_policy: None,
        stat_wait_sum_ns: None,
        stat_wait_sum_ns_saturated: false,
        stat_wait_count: None,
        cpu_perf: None,
    }
}

pub(in crate::test_fixture_builder) fn base_session(run_name: &str) -> SessionFile {
    SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some(run_name.to_owned()),
            started_at: dummy_time(),
            ended_at: dummy_time(),
            monotonic_start_ns: Some(0),
            monotonic_end_ns: Some(1_000_000_000),
            duration_ms: 1000,
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
            metadata: SystemMetadata::default(),
            target_pids_max: 1024,
            active_target_pids_count: 1,
            active_expanded_tasks: vec![100],
            interval_record_count: 1,
            intervals_dropped: 0,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            spike_events_truncated: false,
            scx_event_count: 0,
            irq_event_count: 0,
            migration_event_count: Some(0),
            cpu_freq_sample_count: Some(0),
            gpu_sample_count: 0,
            frame_event_count: 0,
            block_io_event_count: 0,
            event_stream_write_errors: 0,
            alert_events_dropped_count: 0,
            alert_channel_closed_count: 0,
            first_event_stream_write_error: None,
            block_io_correlation_basis: "dev+sector".to_owned(),
            drop_counters: DropCountersSnapshot::default(),
            cpu_perf_sample_count: 0,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
            cpu_perf_last_error: None,
            ..Default::default()
        },
        stop_reason: "test".to_owned(),
        config: dummy_config(),
        tasks: vec![task_for_fixture(
            100,
            TaskClass::Unknown,
            "worker-a",
            10,
            5_000,
        )],
        top_spikes: vec![],
    }
}

pub(in crate::test_fixture_builder) fn dummy_time() -> RecordedTime {
    RecordedTime {
        unix_seconds: 1_625_097_600,
        unix_nanos: 0,
        system_time_debug: "2021-07-01T00:00:00Z".to_owned(),
    }
}

pub(in crate::test_fixture_builder) fn dummy_config() -> RecordedConfig {
    RecordedConfig {
        manual_pids: vec![],
        tree_roots: vec![],
        cgroupv2: None,
        exclude_tree_pids: vec![],
        include_comm: vec![],
        exclude_comm: vec![],
        watch_process: None,
        persistent: false,
        keep_missing_pid: false,
        watch_poll_ms: 100,
        watch_timeout_ms: None,
        csv_stream: None,
        irq_latency: false,
        irqs: vec![],
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        mangohud_log_live: false,
        tui: false,
        summary_period_ms: 1000,
        epoch_period_ms: None,
        retain_intervals: None,
        max_tasks: 1024,
        spike_threshold_ns: 1_000_000,
        live_diagnosis_cluster_window_ms:
            crate::config::model::DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS,
        alert_threshold_ns: None,
        alert_webhook_url: None,
        follow_exec: true,
        verbose: false,
        faults: false,
        cpu_perf: false,
        cpu_perf_kernel: false,
        cpu_perf_max_tasks: 128,
        cpu_perf_cache_refs: false,
        block_io: false,
        stat_wait: false,
        otlp_endpoint: None,
        otel_service_name: "stutter".to_owned(),
        ..Default::default()
    }
}
