#![allow(dead_code)]

use super::model::MonitorConfig;

pub struct ResolvedTargets {
    pub manual_pids: Vec<u32>,
    pub tree_roots: Vec<u32>,
}

pub fn recorded_config_from_final_config(
    config: &MonitorConfig,
    targets: &ResolvedTargets,
) -> crate::recorder::RecordedConfig {
    crate::recorder::RecordedConfig {
        manual_pids: targets.manual_pids.clone(),
        tree_roots: targets.tree_roots.clone(),
        cgroupv2: config.target.cgroupv2.clone(),
        exclude_tree_pids: config.target.exclude_tree_pids.clone(),
        persistent: config.target.persistent,
        keep_missing_pid: config.target.keep_missing_pid,
        watch_process: config.target.watch_process.clone(),
        max_tasks: config.target.max_tasks,
        summary_period_ms: config.timing.summary_period_ms,
        epoch_period_ms: config.timing.epoch_period_ms,
        irq_latency: config.probes.irq_latency,
        irqs: config.probes.irqs.clone(),
        hwmon: config.probes.hwmon,
        faults: config.probes.faults,
        cpu_perf: config.probes.cpu_perf,
        block_io: config.probes.block_io,
        stat_wait: config.probes.stat_wait,
        runtime_slices: config.probes.runtime_slices,
        otlp_endpoint: config.outputs.otlp_endpoint.clone(),
        otel_service_name: config.outputs.otel_service_name.clone(),
        auto_focus: config.focus.auto_focus,
        foreground_window: config.focus.foreground_window,
        foreground_poll_ms: config.focus.foreground_poll_ms,
        foreground_max_stale_ms: config.focus.foreground_max_stale_ms,
        foreground_include_title: config.focus.foreground_include_title,
        follow_exec: config.safety.follow_exec,
        ..Default::default()
    }
}
