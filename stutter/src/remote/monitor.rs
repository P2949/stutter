use std::time::Duration;

use serde::{Deserialize, Serialize};
use stutter_config::monitor_layer::MonitorConfigLayer;

use crate::config::{FocusSource, ForegroundSource, model::MonitorConfig};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteMonitorRequest {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub exclude_tree_pids: Vec<u32>,
    pub duration_seconds: Option<u64>,

    pub spike_us: Option<u64>,
    pub summary_ms: Option<u64>,

    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,

    pub hwmon: bool,
    pub cpu_freq: bool,
    pub faults: bool,
    pub stat_wait: bool,
    pub block_io: bool,
    #[serde(default)]
    pub runtime_slices: bool,
    #[serde(default)]
    pub runtime_slices_max_tasks: Option<usize>,

    pub irq_latency: bool,
    pub irqs: Vec<u32>,

    #[serde(default)]
    pub foreground_window: bool,
    #[serde(default)]
    pub focus_source: Option<String>,
    #[serde(default)]
    pub foreground_source: Option<String>,
    #[serde(default)]
    pub foreground_poll_ms: Option<u64>,
    #[serde(default)]
    pub foreground_max_stale_ms: Option<u64>,
    #[serde(default)]
    pub foreground_include_title: bool,

    pub record: bool,
    pub run_name: Option<String>,
}

impl RemoteMonitorRequest {
    pub fn into_monitor_config_layer(self) -> anyhow::Result<MonitorConfigLayer> {
        let focus_source = match self.focus_source.as_deref() {
            Some(value) => Some(crate::config_file::parse_focus_source_value(value)?),
            None => None,
        };
        let foreground_source = match self.foreground_source.as_deref() {
            Some(value) => Some(crate::config_file::parse_foreground_source_value(value)?),
            None => None,
        };

        let spike_threshold_ns = self.spike_us.map(|value| value.saturating_mul(1_000));

        Ok(MonitorConfigLayer {
            target_pids: (!self.target_pids.is_empty()).then_some(self.target_pids),
            tree_pids: (!self.tree_pids.is_empty()).then_some(self.tree_pids),
            exclude_tree_pids: (!self.exclude_tree_pids.is_empty())
                .then_some(self.exclude_tree_pids),
            include_comm: (!self.include_comm.is_empty()).then_some(self.include_comm),
            exclude_comm: (!self.exclude_comm.is_empty()).then_some(self.exclude_comm),

            summary_period_ms: self.summary_ms,
            max_duration: self
                .duration_seconds
                .map(|seconds| Some(Duration::from_secs(seconds))),
            spike_threshold_ns,

            irq_latency: self.irq_latency.then_some(true),
            irqs: (!self.irqs.is_empty()).then_some(self.irqs),
            hwmon: self.hwmon.then_some(true),
            cpu_freq: self.cpu_freq.then_some(true),
            faults: self.faults.then_some(true),
            block_io: self.block_io.then_some(true),
            stat_wait: self.stat_wait.then_some(true),
            runtime_slices: self.runtime_slices.then_some(true),
            runtime_slices_max_tasks: self.runtime_slices_max_tasks,
            run_name: self
                .record
                .then(|| Some(self.run_name.unwrap_or_else(|| "remote-run".to_owned()))),
            focus_source,
            foreground_window: (self.foreground_window
                || focus_source.is_some_and(|source| source != FocusSource::Heuristic))
            .then_some(true),
            foreground_source,
            foreground_poll_ms: self.foreground_poll_ms,
            foreground_max_stale_ms: self.foreground_max_stale_ms,
            foreground_include_title: self.foreground_include_title.then_some(true),

            ..MonitorConfigLayer::default()
        })
    }
}

fn focus_source_label(source: FocusSource) -> String {
    match source {
        FocusSource::Heuristic => "heuristic",
        FocusSource::Foreground => "foreground",
        FocusSource::Hybrid => "hybrid",
    }
    .to_owned()
}

fn foreground_source_label(source: ForegroundSource) -> String {
    match source {
        ForegroundSource::Auto => "auto",
        ForegroundSource::Sway => "sway",
        ForegroundSource::Hyprland => "hyprland",
        ForegroundSource::X11 => "x11",
    }
    .to_owned()
}

pub fn request_from_monitor_config(config: &MonitorConfig) -> anyhow::Result<RemoteMonitorRequest> {
    Ok(RemoteMonitorRequest {
        target_pids: config.target.target_pids.clone(),
        tree_pids: config.target.tree_pids.clone(),
        exclude_tree_pids: config.target.exclude_tree_pids.clone(),
        duration_seconds: config.timing.max_duration.map(|d| d.as_secs()),
        spike_us: Some(config.timing.spike_threshold_ns / 1000),
        summary_ms: Some(config.timing.summary_period_ms),
        include_comm: config.target.include_comm.clone(),
        exclude_comm: config.target.exclude_comm.clone(),
        hwmon: config.probes.hwmon,
        cpu_freq: config.probes.cpu_freq,
        faults: config.probes.faults,
        stat_wait: config.probes.stat_wait,
        block_io: config.probes.block_io,
        runtime_slices: config.probes.runtime_slices,
        runtime_slices_max_tasks: Some(config.runtime_slices.max_tasks),
        irq_latency: config.probes.irq_latency,
        irqs: config.probes.irqs.clone(),
        foreground_window: config.focus.foreground_window,
        focus_source: Some(focus_source_label(config.focus.focus_source)),
        foreground_source: Some(foreground_source_label(config.focus.foreground_source)),
        foreground_poll_ms: Some(config.focus.foreground_poll_ms),
        foreground_max_stale_ms: Some(config.focus.foreground_max_stale_ms),
        foreground_include_title: config.focus.foreground_include_title,
        record: config.recording.output_dir.is_some() || config.recording.run_name.is_some(),
        run_name: config.recording.run_name.clone(),
    })
}
