use stutter_config::monitor_layer::MonitorConfigLayer;

use crate::{config_file::UserConfigFile, presets::PresetDefaults};

pub fn layer_from_user_file(user_file: &UserConfigFile) -> anyhow::Result<MonitorConfigLayer> {
    let focus_source = match user_file.focus_source.as_deref() {
        Some(value) => Some(crate::config_file::parse_focus_source_value(value)?),
        None => None,
    };

    let foreground_source = match user_file.foreground_source.as_deref() {
        Some(value) => Some(crate::config_file::parse_foreground_source_value(value)?),
        None => None,
    };

    let ebpf_sizing = user_file.ebpf_sizing.as_ref();
    let mangohud = user_file.mangohud.as_ref();
    let alerts = user_file.alerts.as_ref();

    Ok(MonitorConfigLayer {
        summary_period_ms: user_file.summary_period_ms.or(user_file.summary_ms),
        spike_threshold_ns: user_file
            .spike_threshold_ns
            .or_else(|| user_file.spike_us.map(|value| value.saturating_mul(1_000))),
        live_diagnosis_cluster_window_ms: user_file.live_diagnosis_cluster_window_ms,
        hwmon: user_file.hwmon,
        cpu_freq: match (user_file.cpu_freq, user_file.no_cpu_freq) {
            (_, Some(true)) => Some(false),
            (Some(value), _) => Some(value),
            _ => None,
        },
        include_comm: user_file.include_comm.clone(),
        exclude_comm: user_file.exclude_comm.clone(),
        max_tasks: user_file.max_tasks,
        retain_intervals: user_file.retain_intervals.map(Some),
        retention_max_run_count: user_file.retention_max_run_count.map(Some),
        retention_max_total_bytes: user_file.retention_max_total_bytes.map(Some),
        retention_max_age_seconds: user_file.retention_max_age_seconds.map(Some),
        retention_min_free_bytes: user_file.retention_min_free_bytes.map(Some),
        foreground_window: user_file.foreground_window,
        focus_source,
        foreground_source,
        foreground_poll_ms: user_file.foreground_poll_ms,
        foreground_max_stale_ms: user_file.foreground_max_stale_ms,
        foreground_include_title: user_file.foreground_include_title,
        dmabuf_tracking: user_file
            .dmabuf_tracking
            .or(user_file.dmabuf_log.as_ref().map(|_| true)),
        dmabuf_log: user_file.dmabuf_log.clone().map(Some),
        gpu_engine_sampling: user_file.gpu_engine_sampling,
        display_topology: user_file.display_topology,
        alert_desktop_timeout_ms: alerts.and_then(|config| config.desktop_timeout_ms),
        mangohud_tail_idle_sleep_ms: mangohud.and_then(|config| config.tail_idle_sleep_ms),
        mangohud_alignment_poll_ms: mangohud.and_then(|config| config.alignment_poll_ms),
        ringbuf_size_kb: ebpf_sizing
            .and_then(|config| config.ringbuf_size_kb)
            .map(Some),
        wakeup_map_factor: ebpf_sizing
            .and_then(|config| config.wakeup_map_factor)
            .map(Some),
        target_pids_entries: ebpf_sizing
            .and_then(|config| config.target_pids_entries)
            .map(Some),
        target_cgroup_ids_entries: ebpf_sizing
            .and_then(|config| config.target_cgroup_ids_entries)
            .map(Some),
        target_irqs_entries: ebpf_sizing
            .and_then(|config| config.target_irqs_entries)
            .map(Some),
        runnable_task_cpu_factor: ebpf_sizing
            .and_then(|config| config.runnable_task_cpu_factor)
            .map(Some),
        prev_faults_factor: ebpf_sizing
            .and_then(|config| config.prev_faults_factor)
            .map(Some),
        irq_start_entries: ebpf_sizing
            .and_then(|config| config.irq_start_entries)
            .map(Some),
        block_start_entries: ebpf_sizing
            .and_then(|config| config.block_start_entries)
            .map(Some),
        kms_flip_start_entries: ebpf_sizing
            .and_then(|config| config.kms_flip_start_entries)
            .map(Some),
        drm_fence_wait_start_entries: ebpf_sizing
            .and_then(|config| config.drm_fence_wait_start_entries)
            .map(Some),
        drm_fence_signal_entries: ebpf_sizing
            .and_then(|config| config.drm_fence_signal_entries)
            .map(Some),
        ..MonitorConfigLayer::default()
    })
}

pub fn layer_from_preset_defaults(defaults: PresetDefaults) -> MonitorConfigLayer {
    MonitorConfigLayer {
        hwmon: defaults.hwmon,
        cpu_freq: defaults.cpu_freq,
        faults: defaults.faults,
        stat_wait: defaults.stat_wait,
        block_io: defaults.block_io,
        runtime_slices: defaults.runtime_slices,
        irq_latency: defaults.irq_latency,
        kms_timing: defaults.kms_timing,
        drm_fence_latency: defaults.drm_fence_latency,
        wayland_presentation: defaults.wayland_presentation,
        foreground_window: defaults.foreground_window,
        gpu_engine_sampling: defaults.gpu_engine_sampling,
        display_topology: defaults.display_topology,
        ..MonitorConfigLayer::default()
    }
}
