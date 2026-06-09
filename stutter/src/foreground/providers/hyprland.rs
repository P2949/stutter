//! Hyprland foreground provider.
//!
//! Owns `hyprctl activewindow -j` sampling and Hyprland active-window JSON conversion. Does not own
//! provider auto-selection, generic resolver stale handling, or other compositor parsers.

use serde::Deserialize;

use crate::foreground::{
    command::{resolve_trusted_foreground_helper, trusted_foreground_command},
    model::{
        CONFIDENCE_HIGH, CONFIDENCE_LOW, CONFIDENCE_MEDIUM, CONFIDENCE_ZERO, ForegroundDecision,
        ForegroundProviderStatus, ForegroundReason, ForegroundSource, ForegroundTarget,
        ForegroundWindowSnapshot,
    },
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
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!("failed to parse hyprctl activewindow JSON: {err}"),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            };
        }
    };

    let class = active_window.class.or(active_window.initial_class);
    let confidence = if active_window.pid.is_some() {
        CONFIDENCE_HIGH
    } else if class.is_some() {
        CONFIDENCE_MEDIUM
    } else if active_window.title.is_some() || active_window.address.is_some() {
        CONFIDENCE_LOW
    } else {
        CONFIDENCE_ZERO
    };

    ForegroundWindowSnapshot {
        elapsed_ms,
        source: Some(ForegroundSource::Hyprland),
        status: ForegroundProviderStatus::Available,
        decision: ForegroundDecision {
            target: Some(ForegroundTarget {
                pid: active_window.pid,
                app_id: None,
                class,
                title: active_window.title,
                window_id: active_window.address,
                workspace: active_window.workspace.and_then(|workspace| workspace.name),
            }),
            confidence,
            reasons: vec![ForegroundReason {
                reason: "active Hyprland window from hyprctl activewindow".to_owned(),
            }],
            rejected_candidates: Vec::new(),
        },
        stale_ms: None,
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
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason { reason: "HYPRLAND_INSTANCE_SIGNATURE is not set; Hyprland foreground provider is unavailable".to_owned() }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            };
        }

        let Some(hyprctl) = resolve_trusted_foreground_helper(&self.hyprctl) else {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Unavailable,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!(
                            "{} was not found in trusted foreground helper paths; Hyprland foreground provider is unavailable",
                            self.hyprctl
                        ),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
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
                    decision: ForegroundDecision {
                        target: None,
                        confidence: CONFIDENCE_ZERO,
                        reasons: vec![ForegroundReason {
                            reason: format!(
                                "failed to run {} activewindow -j: {err}",
                                hyprctl.display()
                            ),
                        }],
                        rejected_candidates: Vec::new(),
                    },
                    stale_ms: None,
                };
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!(
                            "{} activewindow -j exited with status {}; stderr={}",
                            hyprctl.display(),
                            output.status,
                            stderr.trim()
                        ),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            };
        }

        match String::from_utf8(output.stdout) {
            Ok(stdout) => hyprland_snapshot_from_activewindow_json(elapsed_ms, &stdout),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Hyprland),
                status: ForegroundProviderStatus::Error,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!(
                            "hyprctl activewindow JSON output was not valid UTF-8: {err}"
                        ),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            },
        }
    }
}
