//! Code extracted from the parent module to keep the public CLI surface below the architecture size gate.

use super::*;

impl MonitorArgs {
    pub(super) fn into_monitor_config_layer(
        self,
        presence: MonitorArgPresence,
    ) -> MonitorConfigLayer {
        MonitorConfigLayer {
            target_pids: (!self.target_pids.is_empty()).then(|| self.target_pids.clone()),
            tree_pids: (!self.tree_pids.is_empty()).then(|| self.tree_pids.clone()),
            exclude_tree_pids: (!self.exclude_tree_pids.is_empty())
                .then(|| self.exclude_tree_pids.clone()),
            summary_period_ms: self.summary_period_ms,
            epoch_period_ms: self.epoch_period_ms.map(Some),
            spike_threshold_ns: self
                .spike_threshold_us
                .map(|value| value.saturating_mul(1_000)),
            live_diagnosis_cluster_window_ms: presence
                .live_diagnosis_cluster_window_ms
                .then_some(self.live_diagnosis_cluster_window_ms)
                .flatten(),
            alert_threshold_ns: self
                .alert_threshold_ms
                .map(|value| Some(value.saturating_mul(1_000_000))),
            alert_webhook_url: self.alert_webhook_url.clone().map(Some),
            verbose: self.verbose.then_some(true),
            watch_poll_ms: presence.watch_poll_ms.then_some(self.watch_poll_ms),
            watch_timeout: self
                .watch_timeout_seconds
                .map(|seconds| Some(Duration::from_secs(seconds))),
            include_comm: (!self.include_comm.is_empty()).then(|| self.include_comm.clone()),
            exclude_comm: (!self.exclude_comm.is_empty()).then(|| self.exclude_comm.clone()),
            keep_missing_pid: self.keep_missing_pid.then_some(true),
            watch_process: self.watch_process.clone().map(Some),
            persistent: self.persistent.then_some(true),
            max_tasks: self.max_tasks,
            csv_stream: match (&self.csv_path, &self.stream_csv) {
                (Some(path), None) => Some(Some(CsvStreamTarget::File(path.clone()))),
                (None, Some(value)) if value == "-" => Some(Some(CsvStreamTarget::Stdout)),
                (None, Some(value)) if value.trim().is_empty() => None,
                (None, Some(value)) => Some(Some(CsvStreamTarget::File(PathBuf::from(value)))),
                (None, None) => None,
                (Some(_), Some(_)) => None,
            },
            irq_latency: self.irq_latency.then_some(true),
            irqs: (!self.irqs.is_empty()).then(|| self.irqs.clone()),
            hwmon: if self.no_hwmon {
                Some(false)
            } else if self.hwmon {
                Some(true)
            } else {
                None
            },
            hwmon_root: self.hwmon_root.clone().map(Some),
            hwmon_drm_card: self.hwmon_drm_card.clone().map(Some),
            hwmon_render_node: self.hwmon_render_node.clone().map(Some),
            cpu_freq: if self.no_cpu_freq {
                Some(false)
            } else if self.cpu_freq {
                Some(true)
            } else {
                None
            },
            cgroupv2: self.cgroupv2.clone().map(Some),
            native_cgroup_filter: self.native_cgroup_filter.then_some(true),
            follow_exec: if self.no_follow_exec {
                Some(false)
            } else if presence.follow_exec {
                Some(self.follow_exec)
            } else {
                None
            },
            faults: if self.no_faults {
                Some(false)
            } else if self.faults {
                Some(true)
            } else {
                None
            },
            cpu_perf: self.cpu_perf.then_some(true),
            cpu_perf_kernel: self.cpu_perf_kernel.then_some(true),
            cpu_perf_max_tasks: presence
                .cpu_perf_max_tasks
                .then_some(self.cpu_perf_max_tasks),
            cpu_perf_cache_refs: self.cpu_perf_cache_refs.then_some(true),
            block_io: if self.no_block_io {
                Some(false)
            } else if self.block_io {
                Some(true)
            } else {
                None
            },
            stat_wait: if self.no_stat_wait {
                Some(false)
            } else if self.stat_wait {
                Some(true)
            } else {
                None
            },
            runtime_slices: if self.no_runtime_slices {
                Some(false)
            } else if self.runtime_slices {
                Some(true)
            } else {
                None
            },
            runtime_slices_max_tasks: presence
                .runtime_slices_max_tasks
                .then_some(self.runtime_slices_max_tasks),
            kms_timing: self.kms_timing.then_some(true),
            kms_drm_card: self.kms_card.clone().map(Some),
            kms_connector: self.kms_connector.clone().map(Some),
            kms_crtc: self.kms_crtc.map(Some),
            drm_fence_latency: self.drm_fence_latency.then_some(true),
            drm_fence_render_card: self.drm_fence_render_card.clone().map(Some),
            drm_fence_display_card: self.drm_fence_display_card.clone().map(Some),
            drm_fence_driver_filter: self.drm_fence_driver.clone().map(Some),
            wayland_presentation: self.wayland_presentation.then_some(true),
            wayland_presentation_log: self.wayland_presentation_log.clone().map(Some),
            wayland_presentation_source: presence
                .wayland_presentation_source
                .then_some(self.wayland_presentation_source),
            dmabuf_tracking: (self.dmabuf_tracking || self.dmabuf_log.is_some()).then_some(true),
            dmabuf_log: self.dmabuf_log.clone().map(Some),
            gpu_engine_sampling: self.gpu_engine_sampling.then_some(true),
            display_path_label: self.display_path_label.clone().map(Some),
            display_render_gpu: self.display_render_gpu.clone().map(Some),
            display_scanout_gpu: self.display_scanout_gpu.clone().map(Some),
            display_connector: self.display_connector.clone().map(Some),
            mangohud_log: self.mangohud_log.clone().map(Some),
            mangohud_log_live: self.mangohud_log_live.then_some(true),
            tui: self.tui.then_some(true),
            json_stream: self.json_stream.then_some(true),
            metrics_port: self.metrics_port.map(Some),
            ringbuf_size_kb: self.ringbuf_size_kb.map(Some),
            wakeup_map_factor: self.wakeup_map_factor.map(Some),
            block_start_entries: self.block_start_entries.map(Some),
            drm_fence_wait_start_entries: self.drm_fence_wait_start_entries.map(Some),
            drm_fence_signal_entries: self.drm_fence_signal_entries.map(Some),
            otlp_endpoint: self.otlp_endpoint.clone().map(Some),
            otel_service_name: presence
                .otel_service_name
                .then(|| self.otel_service_name.clone()),
            auto_focus: self.auto_focus.then_some(true),
            focus_source: presence.focus_source.then_some(self.focus_source),
            foreground_window: self.foreground_window.then_some(true),
            foreground_source: presence.foreground_source.then_some(self.foreground_source),
            foreground_poll_ms: presence
                .foreground_poll_ms
                .then_some(self.foreground_poll_ms),
            foreground_max_stale_ms: presence
                .foreground_max_stale_ms
                .then_some(self.foreground_max_stale_ms),
            foreground_include_title: self.foreground_include_title.then_some(true),
            auto_focus_poll_ms: presence
                .auto_focus_poll_ms
                .then_some(self.auto_focus_poll_ms),
            auto_focus_min_confidence: presence
                .auto_focus_min_confidence
                .then_some(self.auto_focus_min_confidence),
            auto_focus_switch_cooldown_ms: presence
                .auto_focus_switch_cooldown_ms
                .then_some(self.auto_focus_switch_cooldown_ms),
            auto_focus_switch_margin: presence
                .auto_focus_switch_margin
                .then_some(self.auto_focus_switch_margin),
            auto_focus_required_polls: presence
                .auto_focus_required_polls
                .then_some(self.auto_focus_required_polls),
            auto_focus_max_roots: presence
                .auto_focus_max_roots
                .then_some(self.auto_focus_max_roots),
            retain_intervals: self.retain_intervals.map(Some),
            retention_max_run_count: self.retention_max_run_count.map(Some),
            retention_max_total_bytes: self.retention_max_total_bytes.map(Some),
            retention_max_age_seconds: self.retention_max_age_seconds.map(Some),
            retention_min_free_bytes: self.retention_min_free_bytes.map(Some),
            run_name: self.run_name.clone().map(Some),
            output_dir: self.out_dir.clone().map(Some),
            remote: self.remote.clone().map(Some),
            ..MonitorConfigLayer::default()
        }
    }
}

impl Default for MonitorArgs {
    fn default() -> Self {
        Self {
            target_pids: Vec::new(),
            tree_pids: Vec::new(),
            exclude_tree_pids: Vec::new(),
            summary_period_ms: None,
            epoch_period_ms: None,
            spike_threshold_us: None,
            live_diagnosis_cluster_window_ms: None,
            alert_threshold_ms: None,
            alert_webhook_url: None,
            verbose: false,
            run_name: None,
            out_dir: None,
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            keep_missing_pid: false,
            watch_process: None,
            persistent: false,
            watch_poll_ms: 2000,
            watch_timeout_seconds: None,
            max_tasks: None,
            csv_path: None,
            stream_csv: None,
            irq_latency: false,
            irqs: Vec::new(),
            hwmon: false,
            no_hwmon: false,
            hwmon_root: None,
            hwmon_drm_card: None,
            hwmon_render_node: None,
            mangohud_log: None,
            mangohud_log_live: false,
            tui: false,
            retain_intervals: None,
            retention_max_run_count: None,
            retention_max_total_bytes: None,
            retention_max_age_seconds: None,
            retention_min_free_bytes: None,
            no_record: false,
            cpu_freq: false,
            no_cpu_freq: false,
            cgroupv2: None,
            native_cgroup_filter: false,
            follow_exec: true,
            no_follow_exec: false,
            faults: false,
            no_faults: false,
            cpu_perf: false,
            cpu_perf_kernel: false,
            cpu_perf_max_tasks: 128,
            cpu_perf_cache_refs: false,
            block_io: false,
            no_block_io: false,
            stat_wait: false,
            no_stat_wait: false,
            runtime_slices: false,
            no_runtime_slices: false,
            runtime_slices_max_tasks: 256,
            kms_timing: false,
            kms_card: None,
            kms_connector: None,
            kms_crtc: None,
            drm_fence_latency: false,
            drm_fence_render_card: None,
            drm_fence_display_card: None,
            drm_fence_driver: None,
            wayland_presentation: false,
            wayland_presentation_log: None,
            wayland_presentation_source: WaylandPresentationSource::ExternalLog,
            dmabuf_tracking: false,
            dmabuf_log: None,
            gpu_engine_sampling: false,
            display_path_label: None,
            display_render_gpu: None,
            display_scanout_gpu: None,
            display_connector: None,
            json_stream: false,
            metrics_port: None,
            preset: None,
            ringbuf_size_kb: None,
            wakeup_map_factor: None,
            block_start_entries: None,
            drm_fence_wait_start_entries: None,
            drm_fence_signal_entries: None,
            otlp_endpoint: None,
            otel_service_name: "stutter".to_owned(),
            auto_focus: false,
            focus_source: FocusSource::Heuristic,
            foreground_window: false,
            foreground_source: ForegroundSource::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            foreground_include_title: false,
            auto_focus_poll_ms: 1000,
            auto_focus_min_confidence: 0.60,
            auto_focus_switch_cooldown_ms: 5000,
            auto_focus_switch_margin: 0.20,
            auto_focus_required_polls: 2,
            auto_focus_max_roots: 4,
            remote: None,
        }
    }
}
