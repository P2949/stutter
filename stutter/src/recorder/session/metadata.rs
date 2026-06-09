use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use super::super::session_files::{
    DisplayPathMetadata, RecordedConfig, RecordedCpuSnapshot, RecordedLatency, RecordedSpike,
    RecordedTime,
};
use crate::{
    artifacts::{ArtifactKind, artifact_path},
    config::{WaylandPresentationSource, model::MonitorConfig},
    display_topology::DisplayTopologySnapshot,
    metrics::{CpuSnapshot, SpikeRecord, TaskStats},
};

pub fn recorded_config(config: &MonitorConfig, tree_pids: &[u32]) -> RecordedConfig {
    RecordedConfig {
        manual_pids: config.target.target_pids.clone(),
        tree_roots: tree_pids.to_vec(),
        scenario_name: config.recording.scenario_name.clone(),
        scenario_hash: config.recording.scenario_hash.clone(),
        workload_label: config.recording.workload_label.clone(),
        route_label: config.recording.route_label.clone(),
        cgroupv2: config.target.cgroupv2.clone(),
        exclude_tree_pids: config.target.exclude_tree_pids.clone(),
        include_comm: config.target.include_comm.clone(),
        exclude_comm: config.target.exclude_comm.clone(),
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
        live_diagnosis_cluster_window_ms: config.diagnosis.live_cluster_window_ms,
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
        kms_timing: config.probes.kms_timing,
        kms_card: config.kms_timing.drm_card.clone(),
        kms_connector: config.kms_timing.connector.clone(),
        kms_crtc: config.kms_timing.crtc,
        drm_fence_latency: config.probes.drm_fence_latency,
        drm_fence_render_card: config.drm_fence.render_card.clone(),
        drm_fence_display_card: config.drm_fence.display_card.clone(),
        drm_fence_driver: config.drm_fence.driver_filter.clone(),
        wayland_presentation: config.probes.wayland_presentation,
        wayland_presentation_log: config.wayland_presentation.log_path.clone(),
        wayland_presentation_source: wayland_presentation_source_label(
            config.wayland_presentation.source,
        ),
        dmabuf_tracking: config.probes.dmabuf_tracking,
        dmabuf_log: config.dmabuf.log_path.clone(),
        gpu_engine_sampling: config.probes.gpu_engine_sampling,
        display_topology: config.probes.display_topology,
        display_path_label: config.display_path.label.clone(),
        display_render_gpu: config.display_path.render_gpu.clone(),
        display_scanout_gpu: config.display_path.scanout_gpu.clone(),
        display_connector: config.display_path.connector.clone(),
        otlp_endpoint: config.outputs.otlp_endpoint.clone(),
        otel_service_name: config.outputs.otel_service_name.clone(),
        auto_focus: config.focus.auto_focus,
        foreground_window: config.focus.foreground_window,
        focus_source: super::super::session_files::focus_source_label(config.focus.focus_source),
        foreground_source: super::super::session_files::foreground_source_arg_label(
            config.focus.foreground_source,
        ),
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

pub(super) fn load_display_topology_snapshot(run_dir: &Path) -> Option<DisplayTopologySnapshot> {
    let path = artifact_path(run_dir, ArtifactKind::DisplayTopology);
    let file = fs::File::open(path).ok()?;
    serde_json::from_reader(file).ok()
}

pub(super) fn display_path_metadata(
    config: &MonitorConfig,
    topology: Option<&DisplayTopologySnapshot>,
) -> Option<DisplayPathMetadata> {
    let guess = topology.and_then(|topology| topology.guessed_path.as_ref());
    let metadata = DisplayPathMetadata {
        label: config.display_path.label.clone().or_else(|| {
            guess.map(|guess| match guess.is_cross_gpu {
                Some(true) => "cross-gpu".to_owned(),
                Some(false) => "direct-scanout-gpu".to_owned(),
                None => "unknown".to_owned(),
            })
        }),
        render_gpu: config.display_path.render_gpu.clone().or_else(|| {
            guess.and_then(|guess| display_gpu_label(&guess.render_card, &guess.render_driver))
        }),
        scanout_gpu: config.display_path.scanout_gpu.clone().or_else(|| {
            guess.and_then(|guess| display_gpu_label(&guess.scanout_card, &guess.scanout_driver))
        }),
        connector: config
            .display_path
            .connector
            .clone()
            .or_else(|| guess.and_then(|guess| guess.connector.clone())),
        render_card: guess.and_then(|guess| guess.render_card.clone()),
        render_render_node: guess
            .and_then(|guess| guess.render_card.as_deref())
            .and_then(|card| {
                topology.and_then(|topology| {
                    topology
                        .drm_devices
                        .iter()
                        .find(|device| device.card == card)
                })
            })
            .and_then(|device| device.render_node.as_ref())
            .map(|render_node| {
                if render_node.starts_with("/dev/") {
                    render_node.clone()
                } else {
                    format!("/dev/dri/{render_node}")
                }
            }),
        render_driver: guess.and_then(|guess| guess.render_driver.clone()),
        scanout_card: guess.and_then(|guess| guess.scanout_card.clone()),
        scanout_driver: guess.and_then(|guess| guess.scanout_driver.clone()),
        is_cross_gpu: guess.and_then(|guess| guess.is_cross_gpu),
        session_type: topology.and_then(|topology| topology.session_type.clone()),
        compositor: topology
            .and_then(|topology| topology.compositor.as_ref())
            .map(|compositor| compositor.name.clone()),
        topology_confidence: guess.map(|guess| guess.confidence.clone()),
        topology_warnings: topology
            .map(|topology| topology.warnings.clone())
            .unwrap_or_default(),
    };
    let has_metadata = metadata.label.is_some()
        || metadata.render_gpu.is_some()
        || metadata.scanout_gpu.is_some()
        || metadata.connector.is_some()
        || metadata.render_card.is_some()
        || metadata.render_render_node.is_some()
        || metadata.render_driver.is_some()
        || metadata.scanout_card.is_some()
        || metadata.scanout_driver.is_some()
        || metadata.is_cross_gpu.is_some()
        || metadata.session_type.is_some()
        || metadata.compositor.is_some()
        || metadata.topology_confidence.is_some()
        || !metadata.topology_warnings.is_empty();

    has_metadata.then_some(metadata)
}

fn display_gpu_label(card: &Option<String>, driver: &Option<String>) -> Option<String> {
    match (card.as_deref(), driver.as_deref()) {
        (Some(card), Some(driver)) => Some(format!("{card}/{driver}")),
        (Some(card), None) => Some(card.to_owned()),
        (None, Some(driver)) => Some(driver.to_owned()),
        (None, None) => None,
    }
}

fn wayland_presentation_source_label(source: WaylandPresentationSource) -> String {
    match source {
        WaylandPresentationSource::ExternalLog => "external-log",
        WaylandPresentationSource::Gamescope => "gamescope",
        WaylandPresentationSource::SelfTest => "self-test",
    }
    .to_owned()
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

pub(super) fn recorded_latency(latency: crate::metrics::LatencySnapshot) -> RecordedLatency {
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

pub(super) fn recorded_cpu(cpu: CpuSnapshot) -> RecordedCpuSnapshot {
    RecordedCpuSnapshot {
        busiest_cpu: cpu.busiest_cpu.map(|cpu| cpu.as_u32()),
        busiest_cpu_samples: cpu.busiest_cpu_samples,
        worst_cpu: cpu.worst_cpu.map(|cpu| cpu.as_u32()),
        worst_cpu_max_ns: cpu.worst_cpu_max_ns,
        spikiest_cpu: cpu.spikiest_cpu.map(|cpu| cpu.as_u32()),
        spikiest_cpu_spikes: cpu.spikiest_cpu_spikes,
        per_cpu: cpu.per_cpu,
    }
}

pub(super) fn recorded_spike(stats: &TaskStats, spike: &SpikeRecord) -> RecordedSpike {
    RecordedSpike {
        class: stats.class,
        process_pid: stats.process_id().map(|pid| pid.as_u32()),
        process_comm: stats.process_comm.clone(),
        cpu: spike.cpu.as_u32(),
        wakeup_target_cpu: spike.wakeup_target_cpu.as_u32(),
        switch_prev_pid: spike.switch_prev_pid.as_u32(),
        switch_prev_state: spike.switch_prev_state,
        switch_prev_state_label: spike.switch_prev_state_label.clone(),
        prio: spike.prio,
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        waker_tid: 0,
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

pub fn recorded_time(time: SystemTime) -> RecordedTime {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();

    RecordedTime {
        unix_seconds: duration.as_secs(),
        unix_nanos: duration.subsec_nanos(),
        system_time_debug: format!("{time:?}"),
    }
}

pub(super) fn timestamp_for_path(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}_{:09}", duration.as_secs(), duration.subsec_nanos())
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

    crate::syscall::clock_gettime_ns(*clock_id).ok()
}

fn is_kernel_before_5_7() -> bool {
    let Ok(release) = crate::syscall::uname_release() else {
        return false;
    };

    let mut parts = release.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    major < 5 || (major == 5 && minor < 7)
}
