//! Foreground-related monitor argument normalization and validation.

use super::*;

pub(super) fn normalize_foreground_monitor_args(args: &mut MonitorArgs) {
    if args.focus_source != FocusSource::Heuristic {
        args.foreground_window = true;
    }
}

pub(super) fn validate_foreground_title_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
    let foreground_focus_requested = args.auto_focus
        && matches!(
            args.focus_source,
            FocusSource::Foreground | FocusSource::Hybrid
        );

    if args.foreground_include_title && !args.foreground_window && !foreground_focus_requested {
        anyhow::bail!(
            "--foreground-include-title requires --foreground-window or --auto-focus with --focus-source foreground or hybrid"
        );
    }

    Ok(())
}

pub(super) fn validate_foreground_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
    if args.foreground_poll_ms < 100 {
        anyhow::bail!("--foreground-poll-ms must be >= 100");
    }

    if args.foreground_max_stale_ms < args.foreground_poll_ms {
        eprintln!(
            "warning: foreground max stale is lower than poll interval; provider errors may clear focus quickly"
        );
    }

    Ok(())
}
