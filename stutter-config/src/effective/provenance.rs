//! Field provenance recording for effective monitor config layers.

use crate::{
    monitor_layer::MonitorConfigLayer,
    source::{ConfigSource, FieldProvenance},
};

pub(super) fn record_layer_provenance(
    layer: &MonitorConfigLayer,
    source: ConfigSource,
    provenance: &mut Vec<FieldProvenance>,
) {
    fn record_if_present<T>(
        value: &Option<T>,
        provenance: &mut Vec<FieldProvenance>,
        field: &'static str,
        source: ConfigSource,
    ) {
        if value.is_some() {
            provenance.push(FieldProvenance::new(field, source));
        }
    }

    record_if_present(&layer.target_pids, provenance, "target.target_pids", source);
    record_if_present(&layer.tree_pids, provenance, "target.tree_pids", source);
    record_if_present(&layer.cgroupv2, provenance, "target.cgroupv2", source);
    record_if_present(
        &layer.exclude_tree_pids,
        provenance,
        "target.exclude_tree_pids",
        source,
    );
    record_if_present(
        &layer.include_comm,
        provenance,
        "target.include_comm",
        source,
    );
    record_if_present(
        &layer.exclude_comm,
        provenance,
        "target.exclude_comm",
        source,
    );
    record_if_present(
        &layer.watch_process,
        provenance,
        "target.watch_process",
        source,
    );
    record_if_present(&layer.persistent, provenance, "target.persistent", source);
    record_if_present(
        &layer.keep_missing_pid,
        provenance,
        "target.keep_missing_pid",
        source,
    );
    record_if_present(&layer.max_tasks, provenance, "target.max_tasks", source);

    record_if_present(
        &layer.summary_period_ms,
        provenance,
        "timing.summary_period_ms",
        source,
    );
    record_if_present(
        &layer.epoch_period_ms,
        provenance,
        "timing.epoch_period_ms",
        source,
    );
    record_if_present(
        &layer.max_duration,
        provenance,
        "timing.max_duration",
        source,
    );
    record_if_present(
        &layer.spike_threshold_ns,
        provenance,
        "timing.spike_threshold_ns",
        source,
    );
    record_if_present(
        &layer.live_diagnosis_cluster_window_ms,
        provenance,
        "diagnosis.live_cluster_window_ms",
        source,
    );

    record_if_present(&layer.irq_latency, provenance, "probes.irq_latency", source);
    record_if_present(&layer.irqs, provenance, "probes.irqs", source);
    record_if_present(&layer.hwmon, provenance, "probes.hwmon", source);
    record_if_present(&layer.hwmon, provenance, "hwmon.enabled", source);
    record_if_present(&layer.cpu_freq, provenance, "probes.cpu_freq", source);
    record_if_present(&layer.faults, provenance, "probes.faults", source);
    record_if_present(&layer.cpu_perf, provenance, "probes.cpu_perf", source);
    record_if_present(&layer.cpu_perf, provenance, "cpu_perf.enabled", source);
    record_if_present(&layer.block_io, provenance, "probes.block_io", source);
    record_if_present(&layer.stat_wait, provenance, "probes.stat_wait", source);
    record_if_present(
        &layer.runtime_slices,
        provenance,
        "probes.runtime_slices",
        source,
    );
    record_if_present(
        &layer.runtime_slices,
        provenance,
        "runtime_slices.enabled",
        source,
    );
    record_if_present(&layer.kms_timing, provenance, "probes.kms_timing", source);
    record_if_present(
        &layer.drm_fence_latency,
        provenance,
        "probes.drm_fence_latency",
        source,
    );
    record_if_present(
        &layer.wayland_presentation,
        provenance,
        "probes.wayland_presentation",
        source,
    );
    record_if_present(
        &layer.dmabuf_tracking,
        provenance,
        "probes.dmabuf_tracking",
        source,
    );
    record_if_present(
        &layer.gpu_engine_sampling,
        provenance,
        "probes.gpu_engine_sampling",
        source,
    );
    record_if_present(
        &layer.display_topology,
        provenance,
        "probes.display_topology",
        source,
    );
    record_if_present(
        &layer.kms_drm_card,
        provenance,
        "kms_timing.drm_card",
        source,
    );
    record_if_present(
        &layer.kms_connector,
        provenance,
        "kms_timing.connector",
        source,
    );
    record_if_present(&layer.kms_crtc, provenance, "kms_timing.crtc", source);
    record_if_present(
        &layer.drm_fence_render_card,
        provenance,
        "drm_fence.render_card",
        source,
    );
    record_if_present(
        &layer.drm_fence_display_card,
        provenance,
        "drm_fence.display_card",
        source,
    );
    record_if_present(
        &layer.drm_fence_driver_filter,
        provenance,
        "drm_fence.driver_filter",
        source,
    );
    record_if_present(
        &layer.wayland_presentation_log,
        provenance,
        "wayland_presentation.log_path",
        source,
    );
    record_if_present(
        &layer.wayland_presentation_source,
        provenance,
        "wayland_presentation.source",
        source,
    );
    record_if_present(&layer.dmabuf_log, provenance, "dmabuf.log_path", source);
    record_if_present(
        &layer.display_path_label,
        provenance,
        "display_path.label",
        source,
    );
    record_if_present(
        &layer.display_render_gpu,
        provenance,
        "display_path.render_gpu",
        source,
    );
    record_if_present(
        &layer.display_scanout_gpu,
        provenance,
        "display_path.scanout_gpu",
        source,
    );
    record_if_present(
        &layer.display_connector,
        provenance,
        "display_path.connector",
        source,
    );

    record_if_present(&layer.run_name, provenance, "recording.run_name", source);
    record_if_present(
        &layer.output_dir,
        provenance,
        "recording.output_dir",
        source,
    );
    record_if_present(
        &layer.retain_intervals,
        provenance,
        "recording.retain_intervals",
        source,
    );
    record_if_present(
        &layer.retention_max_run_count,
        provenance,
        "recording.retention.max_run_count",
        source,
    );
    record_if_present(
        &layer.retention_max_total_bytes,
        provenance,
        "recording.retention.max_total_bytes",
        source,
    );
    record_if_present(
        &layer.retention_max_age_seconds,
        provenance,
        "recording.retention.max_age_seconds",
        source,
    );
    record_if_present(
        &layer.retention_min_free_bytes,
        provenance,
        "recording.retention.min_free_bytes",
        source,
    );

    record_if_present(
        &layer.json_stream,
        provenance,
        "outputs.json_stream",
        source,
    );
    record_if_present(
        &layer.json_stream,
        provenance,
        "streams.json_stream",
        source,
    );
    record_if_present(
        &layer.metrics_port,
        provenance,
        "outputs.metrics_port",
        source,
    );
    record_if_present(
        &layer.otlp_endpoint,
        provenance,
        "outputs.otlp_endpoint",
        source,
    );
    record_if_present(
        &layer.otel_service_name,
        provenance,
        "outputs.otel_service_name",
        source,
    );

    record_if_present(&layer.auto_focus, provenance, "focus.auto_focus", source);
    record_if_present(
        &layer.focus_source,
        provenance,
        "focus.focus_source",
        source,
    );
    record_if_present(
        &layer.foreground_window,
        provenance,
        "focus.foreground_window",
        source,
    );
    record_if_present(
        &layer.foreground_source,
        provenance,
        "focus.foreground_source",
        source,
    );
    record_if_present(
        &layer.foreground_poll_ms,
        provenance,
        "focus.foreground_poll_ms",
        source,
    );
    record_if_present(
        &layer.foreground_max_stale_ms,
        provenance,
        "focus.foreground_max_stale_ms",
        source,
    );
    record_if_present(
        &layer.foreground_include_title,
        provenance,
        "focus.foreground_include_title",
        source,
    );
    record_if_present(
        &layer.auto_focus_poll_ms,
        provenance,
        "focus.auto_focus_poll_ms",
        source,
    );
    record_if_present(
        &layer.auto_focus_min_confidence,
        provenance,
        "focus.auto_focus_min_confidence",
        source,
    );
    record_if_present(
        &layer.auto_focus_switch_cooldown_ms,
        provenance,
        "focus.auto_focus_switch_cooldown_ms",
        source,
    );
    record_if_present(
        &layer.auto_focus_switch_margin,
        provenance,
        "focus.auto_focus_switch_margin",
        source,
    );
    record_if_present(
        &layer.auto_focus_required_polls,
        provenance,
        "focus.auto_focus_required_polls",
        source,
    );
    record_if_present(
        &layer.auto_focus_max_roots,
        provenance,
        "focus.auto_focus_max_roots",
        source,
    );

    record_if_present(&layer.follow_exec, provenance, "safety.follow_exec", source);
    record_if_present(
        &layer.native_cgroup_filter,
        provenance,
        "safety.native_cgroup_filter",
        source,
    );

    record_if_present(&layer.watch_poll_ms, provenance, "watch.poll_ms", source);
    record_if_present(&layer.watch_timeout, provenance, "watch.timeout", source);

    record_if_present(
        &layer.alert_threshold_ns,
        provenance,
        "alerts.threshold_ns",
        source,
    );
    record_if_present(
        &layer.alert_webhook_url,
        provenance,
        "alerts.webhook_url",
        source,
    );
    record_if_present(
        &layer.alert_desktop_timeout_ms,
        provenance,
        "alerts.desktop_timeout_ms",
        source,
    );

    record_if_present(&layer.csv_stream, provenance, "streams.csv", source);
    record_if_present(&layer.verbose, provenance, "streams.verbose", source);

    record_if_present(&layer.hwmon_root, provenance, "hwmon.root", source);
    record_if_present(&layer.hwmon_drm_card, provenance, "hwmon.drm_card", source);
    record_if_present(
        &layer.hwmon_render_node,
        provenance,
        "hwmon.render_node",
        source,
    );

    record_if_present(&layer.mangohud_log, provenance, "mangohud.log", source);
    record_if_present(
        &layer.mangohud_log_live,
        provenance,
        "mangohud.log_live",
        source,
    );
    record_if_present(
        &layer.mangohud_tail_idle_sleep_ms,
        provenance,
        "mangohud.tail_idle_sleep_ms",
        source,
    );
    record_if_present(
        &layer.mangohud_alignment_poll_ms,
        provenance,
        "mangohud.alignment_poll_ms",
        source,
    );

    record_if_present(&layer.tui, provenance, "ui.tui", source);

    record_if_present(
        &layer.cpu_perf_kernel,
        provenance,
        "cpu_perf.include_kernel",
        source,
    );
    record_if_present(
        &layer.cpu_perf_max_tasks,
        provenance,
        "cpu_perf.max_tasks",
        source,
    );
    record_if_present(
        &layer.cpu_perf_cache_refs,
        provenance,
        "cpu_perf.collect_cache_refs",
        source,
    );

    record_if_present(
        &layer.runtime_slices_max_tasks,
        provenance,
        "runtime_slices.max_tasks",
        source,
    );

    record_if_present(
        &layer.ringbuf_size_kb,
        provenance,
        "ebpf_sizing.ringbuf_size_kb",
        source,
    );
    record_if_present(
        &layer.wakeup_map_factor,
        provenance,
        "ebpf_sizing.wakeup_map_factor",
        source,
    );
    record_if_present(
        &layer.target_pids_entries,
        provenance,
        "ebpf_sizing.target_pids_entries",
        source,
    );
    record_if_present(
        &layer.target_cgroup_ids_entries,
        provenance,
        "ebpf_sizing.target_cgroup_ids_entries",
        source,
    );
    record_if_present(
        &layer.target_irqs_entries,
        provenance,
        "ebpf_sizing.target_irqs_entries",
        source,
    );
    record_if_present(
        &layer.runnable_task_cpu_factor,
        provenance,
        "ebpf_sizing.runnable_task_cpu_factor",
        source,
    );
    record_if_present(
        &layer.prev_faults_factor,
        provenance,
        "ebpf_sizing.prev_faults_factor",
        source,
    );
    record_if_present(
        &layer.irq_start_entries,
        provenance,
        "ebpf_sizing.irq_start_entries",
        source,
    );
    record_if_present(
        &layer.block_start_entries,
        provenance,
        "ebpf_sizing.block_start_entries",
        source,
    );
    record_if_present(
        &layer.kms_flip_start_entries,
        provenance,
        "ebpf_sizing.kms_flip_start_entries",
        source,
    );
    record_if_present(
        &layer.drm_fence_wait_start_entries,
        provenance,
        "ebpf_sizing.drm_fence_wait_start_entries",
        source,
    );
    record_if_present(
        &layer.drm_fence_signal_entries,
        provenance,
        "ebpf_sizing.drm_fence_signal_entries",
        source,
    );

    record_if_present(&layer.remote, provenance, "remote.endpoint", source);
}
