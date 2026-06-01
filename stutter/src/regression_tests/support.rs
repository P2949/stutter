//! Shared builders and fixture helpers for regression test modules.

use super::*;

pub(super) mod events {
    use super::MonitorEventSink;
    pub use crate::artifacts::push_artifact_event;

    pub struct HandleEventInput<'a> {
        pub event: &'a super::SchedulerEvent,
        pub config: &'a crate::config::model::MonitorConfig,
        pub started: super::Instant,
        pub tasks: &'a mut super::tasks::TaskTracker,
        pub monotonic_start_ns: Option<u64>,
        pub recorder: &'a mut super::recorder::LiveRecorder,
        pub diagnostics: super::raw_events::interpret::SchedulerEventDiagnostics<'a>,
    }

    pub fn handle_event(input: HandleEventInput<'_>) -> Option<crate::recorder::SpikeEvent> {
        let runtime_config =
            super::raw_events::EventRuntimeConfig::from_monitor_config(input.config);
        let update = super::raw_events::handle_event_with_runtime_config(
            input.event,
            super::raw_events::EventHandlingContext {
                config: &runtime_config,
                started: input.started,
                tasks: input.tasks,
                monotonic_start_ns: input.monotonic_start_ns,
                diagnostics: input.diagnostics,
            },
        );

        for event in &update.events {
            let mut ctx = super::MonitorSinkContext {
                recorder: &mut *input.recorder,
                alert_sender: None,
                output: super::MonitorOutputConfig::default(),
            };
            let mut sink = super::RecorderSink::new();
            sink.on_event(event, &mut ctx).unwrap();
        }

        update.spike_event
    }
}
pub(super) fn test_config(
    target_pids: Vec<u32>,
    tree_pids: Vec<u32>,
    max_duration: Option<Duration>,
) -> crate::config::model::MonitorConfig {
    crate::config::model::MonitorConfig {
        target: crate::config::model::TargetConfig {
            target_pids,
            tree_pids,
            max_tasks: 1024,
            ..Default::default()
        },
        timing: crate::config::model::TimingConfig {
            summary_period_ms: 1_000,
            spike_threshold_ns: 1_000_000,
            max_duration,
            ..Default::default()
        },
        watch: crate::config::model::WatchConfig {
            poll_ms: 2_000,
            ..Default::default()
        },
        ..Default::default()
    }
}

pub(super) fn minimal_session_for_report() -> SessionFile {
    let mut config = test_config(vec![], vec![], None);
    config.probes.irq_latency = true;
    config.probes.irqs = vec![137];
    config.probes.hwmon = true;
    let monitor_config = config.clone();

    SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some("report-correlation".to_owned()),
            started_at: recorded_time(UNIX_EPOCH),
            ended_at: recorded_time(UNIX_EPOCH),
            monotonic_start_ns: Some(0),
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
            monotonic_end_ns: Some(20_000_000),
            duration_ms: 20,
            metadata: SystemMetadata::default(),
            target_pids_max: 1024,
            active_target_pids_count: 0,
            active_expanded_tasks: Vec::new(),
            spike_events_retained_count: 3,
            spike_events_dropped_count: 0,
            spike_events_truncated: false,
            scx_event_count: 0,
            irq_event_count: 1,
            migration_event_count: Some(0),
            cpu_freq_sample_count: Some(0),
            gpu_sample_count: 1,
            frame_event_count: 1,
            block_io_event_count: 0,
            event_stream_write_errors: 0,
            alert_events_dropped_count: 0,
            alert_channel_closed_count: 0,
            first_event_stream_write_error: None,
            block_io_correlation_basis: "dev+sector".to_owned(),
            drop_counters: DropCountersSnapshot::default(),
            interval_record_count: 0,
            intervals_dropped: 0,
            cpu_perf_sample_count: 0,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
            cpu_perf_last_error: None,
            ..Default::default()
        },
        stop_reason: "test".to_owned(),
        config: recorded_config(&monitor_config, &config.target.tree_pids),
        tasks: Vec::new(),
        top_spikes: Vec::new(),
    }
}

pub(super) fn session_task(
    tid: u32,
    process_pid: u32,
    class: TaskClass,
    comm: &str,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> SessionTask {
    SessionTask {
        task: tid,
        active: true,
        first_seen_ms: 0,
        last_seen_ms: 100,
        removed_ms: None,
        class,
        process_pid: Some(process_pid),
        process_comm: "game".into(),
        process_starttime_ticks,
        task_starttime_ticks,
        exe_dev: Some(1),
        exe_ino: Some(2),
        comm: comm.to_owned(),
        allowed_cpus: None,
        latency: RecordedLatency {
            samples: 1,
            stored_samples: 1,
            truncated_samples: 0,
            percentile_scope: "histogram".to_owned(),
            histogram: Vec::new(),
            min_ns: 1,
            avg_ns: 1,
            p95_ns: 1,
            p99_ns: 1,
            max_ns: 1,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
        },
        cpu: RecordedCpuSnapshot {
            busiest_cpu: None,
            busiest_cpu_samples: 0,
            worst_cpu: None,
            worst_cpu_max_ns: 0,
            spikiest_cpu: None,
            spikiest_cpu_spikes: 0,
            per_cpu: Vec::new(),
        },
        top_spikes: Vec::new(),
        migration_count: 0,
        cross_numa_migrations: 0,
        top_wakers: Vec::new(),
        sched_policy: None,
        stat_wait_sum_ns: None,
        stat_wait_sum_ns_saturated: false,
        stat_wait_count: None,
        cpu_perf: None,
    }
}

pub(super) fn interval_record(
    task: u32,
    class: TaskClass,
    samples: u64,
    comm: &str,
) -> metrics::IntervalRecord {
    metrics::IntervalRecord {
        elapsed_ms: 1_000,
        task,
        active: true,
        class,
        comm: comm.to_owned(),
        process_pid: Some(100),
        process_comm: "game".into(),
        samples,
        stored_samples: samples,
        truncated_samples: 0,
        min_ns: 0,
        avg_ns: 0,
        p95_ns: 0,
        p99_ns: 0,
        max_ns: 0,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: 0,
        busiest_cpu: None,
        busiest_cpu_samples: 0,
        worst_cpu: None,
        worst_cpu_max_ns: 0,
        spikiest_cpu: None,
        spikiest_cpu_spikes: 0,
        cpu_psi_some: 0.0,
        mem_psi_some: 0.0,
        mem_psi_full: 0.0,
        mem_psi_delta_us: 0,
        mem_psi_spike: false,
        io_psi_some: 0.0,
        io_psi_full: 0.0,
        major_faults: 0,
        minor_faults: 0,
        percentile_scope: "histogram".to_owned(),
        histogram: Vec::new(),
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf: None,
    }
}

pub(super) fn task_stats_with_info(
    tid: u32,
    process_pid: u32,
    process_comm: &str,
    comm: &str,
    class: TaskClass,
    first_seen_ms: u64,
) -> metrics::TaskStats {
    let mut stats = metrics::TaskStats::new(tid, comm.to_owned(), first_seen_ms);
    stats.apply_task_info(&task_info(tid, process_pid, process_comm, comm, class));
    stats
}

pub(super) fn task_info(
    tid: u32,
    process_pid: u32,
    process_comm: &str,
    comm: &str,
    class: TaskClass,
) -> TaskInfo {
    TaskInfo {
        tid: tid.into(),
        process_pid: process_pid.into(),
        process_ppid: 1.into(),
        comm: comm.into(),
        process_comm: process_comm.into(),
        process_starttime_ticks: Some(u64::from(process_pid) * 10),
        task_starttime_ticks: Some(u64::from(tid) * 10),
        exe_dev: None,
        exe_ino: None,
        class,
        sched_policy: None,
        from_cgroup: false,
    }
}

pub(super) fn scheduler_event(pid: u32, comm: &str) -> SchedulerEvent {
    scheduler_event_with_latency(pid, comm, 10)
}

pub(super) fn scheduler_event_with_latency(
    pid: u32,
    comm: &str,
    latency_ns: u64,
) -> SchedulerEvent {
    let mut comm_bytes = [0; 16];
    for (idx, byte) in comm.as_bytes().iter().take(15).enumerate() {
        comm_bytes[idx] = *byte;
    }

    SchedulerEvent {
        kind: EVENT_RUNNABLE_LATENCY,
        tid: pid,
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        wakeup_ns: 100,
        switch_ns: 100 + latency_ns,
        latency_ns,
        comm: comm_bytes,
        waker_tid: 0,
        target_pending_wakeups: 0,
        observed_runnable_depth: 0,
        maj_flt: 0,
        min_flt: 0,
        switch_prev_pid: 0,
        _pad0: 0,
        switch_prev_state: 0,
    }
}

pub(super) fn spike_event(task: u32, switch_ns: u64) -> SpikeEvent {
    SpikeEvent {
        elapsed_ms: Some(switch_ns / 1_000_000),
        task: task.into(),
        active: true,
        class: TaskClass::Game,
        process_pid: Some(task.into()),
        process_comm: "game".into(),
        comm: "game".to_owned(),
        latency_ns: 1_000_000,
        wakeup_ns: switch_ns.saturating_sub(1_000_000),
        switch_ns,
        ..Default::default()
    }
}
pub(super) fn temp_test_dir(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    dir
}

pub(super) fn create_fake_proc(
    proc_root: &Path,
    pid: u32,
    ppid: u32,
    name: &str,
    cmdline: &str,
    tids: &[u32],
) {
    let proc_dir = proc_root.join(pid.to_string());
    fs::create_dir_all(proc_dir.join("task")).unwrap();
    fs::write(
        proc_dir.join("status"),
        format!("Name:\t{name}\nPPid:\t{ppid}\n"),
    )
    .unwrap();
    fs::write(proc_dir.join("cmdline"), cmdline.as_bytes()).unwrap();
    fs::write(proc_dir.join("stat"), fake_stat(name, u64::from(pid) * 10)).unwrap();
    fs::write(proc_dir.join("exe"), format!("{name}\n")).unwrap();

    for tid in tids {
        let task_dir = proc_dir.join("task").join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("comm"), format!("{name}-{tid}\n")).unwrap();
        fs::write(task_dir.join("stat"), fake_stat(name, u64::from(*tid) * 10)).unwrap();
    }
}

pub(super) fn fake_stat(comm: &str, starttime: u64) -> String {
    let mut fields = vec!["0".to_owned(); 18];
    fields.push(starttime.to_string());
    format!("1 ({comm}) S {}\n", fields.join(" "))
}
pub(super) fn render_report_for_test(
    session: &crate::recorder::SessionFile,
    artifacts: &crate::session_io::RunArtifacts,
    cluster_window_ms: u64,
    top: usize,
) -> String {
    let spikes = if artifacts.spikes.is_empty() {
        None
    } else {
        Some(artifacts.spikes.as_slice())
    };
    let cluster_analysis = crate::report::spike_cluster_analysis(
        session,
        spikes,
        cluster_window_ms.saturating_mul(1_000_000),
        top,
        None,
    );
    let data_quality = crate::report::data_quality_summary(session, &artifacts.validation);
    let pressure_timeline = crate::report::PressureTimelineSummary::default();
    let runtime_slices = crate::report::RuntimeSliceAnalysisSummary::default();
    let correlation_sections = crate::report::text_report_correlation_sections(
        &cluster_analysis.clusters,
        artifacts,
        crate::report::analysis::block_io_correlation_basis(session),
        cluster_window_ms.saturating_mul(1_000_000),
        top,
    );
    crate::report::render_report(crate::report::TextReportRenderInput {
        path: Path::new("session.json"),
        session,
        cluster_analysis: &cluster_analysis,
        frame_diagnoses: &[],
        data_quality: &data_quality,
        pressure_timeline: &pressure_timeline,
        runtime_slice_summary: &runtime_slices,
        correlation_sections: &correlation_sections,
        focus_summary: &crate::report::FocusReportSummary::default(),
        foreground_summary: &crate::report::ForegroundReportSummary::default(),
        display_path_diagnosis: None,
        top,
        cluster_window_ms,
        filter_class: None,
    })
}
