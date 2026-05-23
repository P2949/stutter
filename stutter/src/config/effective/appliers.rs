//! Mechanical layer-to-config assignment helpers for effective monitor config resolution.
//!
//! This module intentionally owns the repetitive `Option<T>` merge assignments so
//! `effective.rs` can stay focused on source ordering, diagnostics, and provenance.

use crate::config::{
    layer::MonitorConfigLayer,
    model::{
        AlertConfig, CpuPerfConfig, DiagnosisConfig, DisplayPathConfig, DmaBufConfig,
        DrmFenceConfig, EbpfSizingConfig, FocusConfig, HwmonConfig, KmsTimingConfig,
        MangoHudConfig, MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, RemoteConfig,
        RuntimeSlicesConfig, SafetyConfig, StreamConfig, TargetConfig, TimingConfig, UiConfig,
        WatchConfig, WaylandPresentationConfig,
    },
};

pub(super) fn apply_config_layer(config: &mut MonitorConfig, layer: &MonitorConfigLayer) {
    apply_target_layer(&mut config.target, layer);
    apply_timing_layer(&mut config.timing, layer);
    apply_diagnosis_layer(&mut config.diagnosis, layer);
    apply_probe_layer(&mut config.probes, layer);
    apply_recording_layer(&mut config.recording, layer);
    apply_output_layer(&mut config.outputs, layer);
    apply_focus_layer(&mut config.focus, layer);
    apply_safety_layer(&mut config.safety, layer);
    apply_watch_layer(&mut config.watch, layer);
    apply_alert_layer(&mut config.alerts, layer);
    apply_stream_layer(&mut config.streams, layer);
    apply_hwmon_layer(&mut config.hwmon, layer);
    apply_mangohud_layer(&mut config.mangohud, layer);
    apply_cpu_perf_layer(&mut config.cpu_perf, layer);
    apply_runtime_slices_layer(&mut config.runtime_slices, layer);
    apply_kms_timing_layer(&mut config.kms_timing, layer);
    apply_drm_fence_layer(&mut config.drm_fence, layer);
    apply_wayland_presentation_layer(&mut config.wayland_presentation, layer);
    apply_dmabuf_layer(&mut config.dmabuf, layer);
    apply_display_path_layer(&mut config.display_path, layer);
    apply_ebpf_sizing_layer(&mut config.ebpf_sizing, layer);
    apply_ui_layer(&mut config.ui, layer);
    apply_remote_layer(&mut config.remote, layer);
}

fn apply_target_layer(config: &mut TargetConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.target_pids {
        config.target_pids = value.clone();
    }
    if let Some(value) = &layer.tree_pids {
        config.tree_pids = value.clone();
    }
    if let Some(value) = &layer.cgroupv2 {
        config.cgroupv2 = value.clone();
    }
    if let Some(value) = &layer.exclude_tree_pids {
        config.exclude_tree_pids = value.clone();
    }
    if let Some(value) = &layer.include_comm {
        config.include_comm = value.clone();
    }
    if let Some(value) = &layer.exclude_comm {
        config.exclude_comm = value.clone();
    }
    if let Some(value) = &layer.watch_process {
        config.watch_process = value.clone();
    }
    if let Some(value) = layer.persistent {
        config.persistent = value;
    }
    if let Some(value) = layer.keep_missing_pid {
        config.keep_missing_pid = value;
    }
    if let Some(value) = layer.max_tasks {
        config.max_tasks = value;
    }
}

fn apply_timing_layer(config: &mut TimingConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.summary_period_ms {
        config.summary_period_ms = value;
    }
    if let Some(value) = layer.epoch_period_ms {
        config.epoch_period_ms = value;
    }
    if let Some(value) = layer.max_duration {
        config.max_duration = value;
    }
    if let Some(value) = layer.spike_threshold_ns {
        config.spike_threshold_ns = value;
    }
}

fn apply_diagnosis_layer(config: &mut DiagnosisConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.live_diagnosis_cluster_window_ms {
        config.live_cluster_window_ms = value;
    }
}

fn apply_probe_layer(config: &mut ProbeConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.irq_latency {
        config.irq_latency = value;
    }
    if let Some(value) = &layer.irqs {
        config.irqs = value.clone();
    }
    if let Some(value) = layer.hwmon {
        config.hwmon = value;
    }
    if let Some(value) = layer.cpu_freq {
        config.cpu_freq = value;
    }
    if let Some(value) = layer.faults {
        config.faults = value;
    }
    if let Some(value) = layer.cpu_perf {
        config.cpu_perf = value;
    }
    if let Some(value) = layer.block_io {
        config.block_io = value;
    }
    if let Some(value) = layer.stat_wait {
        config.stat_wait = value;
    }
    if let Some(value) = layer.runtime_slices {
        config.runtime_slices = value;
    }
    if let Some(value) = layer.kms_timing {
        config.kms_timing = value;
    }
    if let Some(value) = layer.drm_fence_latency {
        config.drm_fence_latency = value;
    }
    if let Some(value) = layer.wayland_presentation {
        config.wayland_presentation = value;
    }
    if let Some(value) = layer.dmabuf_tracking {
        config.dmabuf_tracking = value;
    }
    if let Some(value) = layer.gpu_engine_sampling {
        config.gpu_engine_sampling = value;
    }
    if let Some(value) = layer.display_topology {
        config.display_topology = value;
    }
}

fn apply_recording_layer(config: &mut RecordingConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.run_name {
        config.run_name = value.clone();
    }
    if let Some(value) = &layer.output_dir {
        config.output_dir = value.clone();
    }
    if let Some(value) = layer.retain_intervals {
        config.retain_intervals = value;
    }
    if let Some(value) = layer.retention_max_run_count {
        config.retention.max_run_count = value;
    }
    if let Some(value) = layer.retention_max_total_bytes {
        config.retention.max_total_bytes = value;
    }
    if let Some(value) = layer.retention_max_age_seconds {
        config.retention.max_age_seconds = value;
    }
    if let Some(value) = layer.retention_min_free_bytes {
        config.retention.min_free_bytes = value;
    }
}

fn apply_output_layer(config: &mut OutputConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.json_stream {
        config.json_stream = value;
    }
    if let Some(value) = layer.metrics_port {
        config.metrics_port = value;
    }
    if let Some(value) = &layer.otlp_endpoint {
        config.otlp_endpoint = value.clone();
    }
    if let Some(value) = &layer.otel_service_name {
        config.otel_service_name = value.clone();
    }
}

fn apply_focus_layer(config: &mut FocusConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.auto_focus {
        config.auto_focus = value;
    }
    if let Some(value) = layer.focus_source {
        config.focus_source = value;
    }
    if let Some(value) = layer.foreground_window {
        config.foreground_window = value;
    }
    if let Some(value) = layer.foreground_source {
        config.foreground_source = value;
    }
    if let Some(value) = layer.foreground_poll_ms {
        config.foreground_poll_ms = value;
    }
    if let Some(value) = layer.foreground_max_stale_ms {
        config.foreground_max_stale_ms = value;
    }
    if let Some(value) = layer.foreground_include_title {
        config.foreground_include_title = value;
    }
    if let Some(value) = layer.auto_focus_poll_ms {
        config.auto_focus_poll_ms = value;
    }
    if let Some(value) = layer.auto_focus_min_confidence {
        config.auto_focus_min_confidence = value;
    }
    if let Some(value) = layer.auto_focus_switch_cooldown_ms {
        config.auto_focus_switch_cooldown_ms = value;
    }
    if let Some(value) = layer.auto_focus_switch_margin {
        config.auto_focus_switch_margin = value;
    }
    if let Some(value) = layer.auto_focus_required_polls {
        config.auto_focus_required_polls = value;
    }
    if let Some(value) = layer.auto_focus_max_roots {
        config.auto_focus_max_roots = value;
    }
}

fn apply_safety_layer(config: &mut SafetyConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.follow_exec {
        config.follow_exec = value;
    }
    if let Some(value) = layer.native_cgroup_filter {
        config.native_cgroup_filter = value;
    }
}

fn apply_watch_layer(config: &mut WatchConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.watch_poll_ms {
        config.poll_ms = value;
    }
    if let Some(value) = layer.watch_timeout {
        config.timeout = value;
    }
}

fn apply_alert_layer(config: &mut AlertConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.alert_threshold_ns {
        config.threshold_ns = value;
    }
    if let Some(value) = &layer.alert_webhook_url {
        config.webhook_url = value.clone();
    }
}

fn apply_stream_layer(config: &mut StreamConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.csv_stream {
        config.csv = value.clone();
    }
    if let Some(value) = layer.json_stream {
        config.json_stream = value;
    }
    if let Some(value) = layer.verbose {
        config.verbose = value;
    }
}

fn apply_hwmon_layer(config: &mut HwmonConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.hwmon {
        config.enabled = value;
    }
    if let Some(value) = &layer.hwmon_root {
        config.root = value.clone();
    }
    if let Some(value) = &layer.hwmon_drm_card {
        config.drm_card = value.clone();
    }
    if let Some(value) = &layer.hwmon_render_node {
        config.render_node = value.clone();
    }
}

fn apply_mangohud_layer(config: &mut MangoHudConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.mangohud_log {
        config.log = value.clone();
    }
    if let Some(value) = layer.mangohud_log_live {
        config.log_live = value;
    }
}

fn apply_cpu_perf_layer(config: &mut CpuPerfConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.cpu_perf {
        config.enabled = value;
    }
    if let Some(value) = layer.cpu_perf_kernel {
        config.include_kernel = value;
    }
    if let Some(value) = layer.cpu_perf_max_tasks {
        config.max_tasks = value;
    }
    if let Some(value) = layer.cpu_perf_cache_refs {
        config.collect_cache_refs = value;
    }
}

fn apply_runtime_slices_layer(config: &mut RuntimeSlicesConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.runtime_slices {
        config.enabled = value;
    }
    if let Some(value) = layer.runtime_slices_max_tasks {
        config.max_tasks = value;
    }
}

fn apply_kms_timing_layer(config: &mut KmsTimingConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.kms_drm_card {
        config.drm_card = value.clone();
    }
    if let Some(value) = &layer.kms_connector {
        config.connector = value.clone();
    }
    if let Some(value) = layer.kms_crtc {
        config.crtc = value;
    }
}

fn apply_drm_fence_layer(config: &mut DrmFenceConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.drm_fence_render_card {
        config.render_card = value.clone();
    }
    if let Some(value) = &layer.drm_fence_display_card {
        config.display_card = value.clone();
    }
    if let Some(value) = &layer.drm_fence_driver_filter {
        config.driver_filter = value.clone();
    }
}

fn apply_wayland_presentation_layer(
    config: &mut WaylandPresentationConfig,
    layer: &MonitorConfigLayer,
) {
    if let Some(value) = &layer.wayland_presentation_log {
        config.log_path = value.clone();
    }
    if let Some(value) = layer.wayland_presentation_source {
        config.source = value;
    }
}

fn apply_dmabuf_layer(config: &mut DmaBufConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.dmabuf_log {
        config.log_path = value.clone();
    }
}

fn apply_display_path_layer(config: &mut DisplayPathConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.display_path_label {
        config.label = value.clone();
    }
    if let Some(value) = &layer.display_render_gpu {
        config.render_gpu = value.clone();
    }
    if let Some(value) = &layer.display_scanout_gpu {
        config.scanout_gpu = value.clone();
    }
    if let Some(value) = &layer.display_connector {
        config.connector = value.clone();
    }
}

fn apply_ebpf_sizing_layer(config: &mut EbpfSizingConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.ringbuf_size_kb {
        config.ringbuf_size_kb = value;
    }
    if let Some(value) = layer.wakeup_map_factor {
        config.wakeup_map_factor = value;
    }
}

fn apply_ui_layer(config: &mut UiConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.tui {
        config.tui = value;
    }
}

fn apply_remote_layer(config: &mut RemoteConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.remote {
        config.endpoint = value.clone();
    }
}
