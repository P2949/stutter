//! Hyprland foreground provider.
//!
//! Owns `hyprctl activewindow -j` sampling and Hyprland active-window JSON conversion. Does not own
//! provider auto-selection, generic resolver stale handling, or other compositor parsers.

use serde::Deserialize;

use crate::foreground::{
    command::{resolve_trusted_foreground_helper, trusted_foreground_command},
    model::{ForegroundProviderStatus, ForegroundSource, ForegroundWindowSnapshot},
    provider::ForegroundProvider,
};

#[derive(Debug, Deserialize)]
struct HyprlandActiveWindow {
    address: Option<String>,
    class: Option<String>,
    #[serde(rename = "initialClass")]
    initial_class: Option<String>,
    title: Option<String>,
    pid: Option<u32>,
    workspace: Option<HyprlandWorkspace>,
}

#[derive(Debug, Deserialize)]
struct HyprlandWorkspace {
    name: Option<String>,
}

pub(crate) fn hyprland_snapshot_from_activewindow_json(
    elapsed_ms: u64,
    active_window_json: &str,
) -> ForegroundWindowSnapshot {
    let active_window = match serde_json::from_str::<HyprlandActiveWindow>(active_window_json) {
        Ok(active_window) => active_window,
        Err(err) => {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!("failed to parse hyprctl activewindow JSON: {err}"),
                ..ForegroundWindowSnapshot::default()
            };
        }
    };

    let class = active_window.class.or(active_window.initial_class);
    let confidence = if active_window.pid.is_some() {
        0.95
    } else if class.is_some() {
        0.65
    } else if active_window.title.is_some() || active_window.address.is_some() {
        0.35
    } else {
        0.0
    };

    ForegroundWindowSnapshot {
        elapsed_ms,
        source: Some(ForegroundSource::Hyprland),
        status: ForegroundProviderStatus::Available,
        pid: active_window.pid,
        app_id: None,
        class,
        title: active_window.title,
        window_id: active_window.address,
        workspace: active_window.workspace.and_then(|workspace| workspace.name),
        confidence,
        stale_ms: None,
        reason: "active Hyprland window from hyprctl activewindow".to_owned(),
    }
}

#[derive(Debug, Clone)]
pub struct HyprlandForegroundProvider {
    hyprctl: String,
}

impl Default for HyprlandForegroundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl HyprlandForegroundProvider {
    pub fn new() -> Self {
        Self {
            hyprctl: "hyprctl".to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_hyprctl(mut self, hyprctl: impl Into<String>) -> Self {
        self.hyprctl = hyprctl.into();
        self
    }

    pub fn is_detected() -> bool {
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok()
    }
}

impl ForegroundProvider for HyprlandForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::Hyprland
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        if !Self::is_detected() {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: "HYPRLAND_INSTANCE_SIGNATURE is not set; Hyprland foreground provider is unavailable".to_owned(),
                ..ForegroundWindowSnapshot::default()
            };
        }

        let Some(hyprctl) = resolve_trusted_foreground_helper(&self.hyprctl) else {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Unavailable,
                confidence: 0.0,
                reason: format!(
                    "{} was not found in trusted foreground helper paths; Hyprland foreground provider is unavailable",
                    self.hyprctl
                ),
                ..ForegroundWindowSnapshot::default()
            };
        };

        let output = match trusted_foreground_command(&hyprctl)
            .args(["activewindow", "-j"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return ForegroundWindowSnapshot {
                    elapsed_ms,
                    source: Some(ForegroundSource::Hyprland),
                    status: ForegroundProviderStatus::Error,
                    confidence: 0.0,
                    reason: format!("failed to run {} activewindow -j: {err}", hyprctl.display()),
                    ..ForegroundWindowSnapshot::default()
                };
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!(
                    "{} activewindow -j exited with status {}; stderr={}",
                    hyprctl.display(),
                    output.status,
                    stderr.trim()
                ),
                ..ForegroundWindowSnapshot::default()
            };
        }

        match String::from_utf8(output.stdout) {
            Ok(stdout) => hyprland_snapshot_from_activewindow_json(elapsed_ms, &stdout),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                confidence: 0.0,
                reason: format!("hyprctl activewindow JSON output was not valid UTF-8: {err}"),
                ..ForegroundWindowSnapshot::default()
            },
        }
    }
}
