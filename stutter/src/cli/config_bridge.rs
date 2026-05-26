use std::{sync::Arc, time::Duration};

use super::monitor::{
    MonitorArgPresence, MonitorArgs, RecordingMode, monitor_config_from_monitor_args_with_presence,
};
use crate::config::{FocusSource, model::MonitorConfig};

pub(crate) fn autotune_monitor_config(
    input: &crate::autotune::commands::live::AutotuneCommandInput,
) -> anyhow::Result<Arc<MonitorConfig>> {
    let has_target = input.tree_pid.is_some() || input.watch_process.is_some();
    if !has_target && !input.auto_focus {
        anyhow::bail!("autotune requires --tree-pid, --watch-process, or --auto-focus");
    }

    let mut monitor = MonitorArgs {
        watch_process: input.watch_process.clone(),
        tree_pids: input.tree_pid.map_or(Vec::new(), |pid| vec![pid]),
        persistent: input.watch_process.is_some(),
        summary_period_ms: Some(input.summary_ms),
        preset: Some(input.preset.clone()),
        hwmon: input.hwmon,
        no_hwmon: !input.hwmon,
        mangohud_log: input.mangohud_log.clone(),
        no_record: true,
        run_name: Some("autotune-observe".to_owned()),
        auto_focus: input.auto_focus,
        focus_source: input.focus_source,
        foreground_window: input.foreground_window
            || input.auto_focus
            || matches!(
                input.focus_source,
                FocusSource::Foreground | FocusSource::Hybrid
            ),
        foreground_source: input.foreground_source,
        foreground_poll_ms: input.foreground_poll_ms,
        foreground_max_stale_ms: input.foreground_max_stale_ms,
        foreground_include_title: false,
        auto_focus_min_confidence: 0.70,
        auto_focus_required_polls: 3,
        auto_focus_switch_cooldown_ms: 5_000,
        auto_focus_switch_margin: 0.20,
        auto_focus_max_roots: 1,
        ..Default::default()
    };

    monitor.no_record = true;

    Ok(Arc::new(monitor_config_from_monitor_args_with_presence(
        monitor,
        RecordingMode::ForceRecording {
            max_duration: input.duration_seconds.map(Duration::from_secs),
        },
        MonitorArgPresence::autotune_monitor_defaults(),
    )?))
}
