//! KDE Plasma Wayland foreground provider.
//!
//! This provider intentionally does not inject KWin scripts. It only consumes
//! JSON from an explicit trusted helper.

use crate::foreground::{
    command::{resolve_trusted_foreground_helper, trusted_foreground_command},
    model::{
        CONFIDENCE_ZERO, ForegroundDecision, ForegroundProviderStatus, ForegroundReason,
        ForegroundSource, ForegroundWindowSnapshot,
    },
    parse::compositor_json::snapshot_from_compositor_helper_json,
    provider::ForegroundProvider,
    providers::desktop::{desktop_looks_like_kde, wayland_session_detected},
};

const KDE_HELPER: &str = "stutter-kde-foreground";
const KDE_UNSAFE_REASON: &str = "KDE Wayland foreground provider requires stutter-kde-foreground; KWin script injection is intentionally not used";

#[derive(Debug, Clone)]
pub struct KdeForegroundProvider {
    helper: String,
}

impl Default for KdeForegroundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl KdeForegroundProvider {
    pub fn new() -> Self {
        Self {
            helper: KDE_HELPER.to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_helper(mut self, helper: impl Into<String>) -> Self {
        self.helper = helper.into();
        self
    }

    pub fn is_detected() -> bool {
        wayland_session_detected() && desktop_looks_like_kde()
    }

    pub fn sample_from_helper_json(&self, elapsed_ms: u64, json: &str) -> ForegroundWindowSnapshot {
        snapshot_from_compositor_helper_json(elapsed_ms, ForegroundSource::Kde, "KDE", json)
    }

    fn unavailable(elapsed_ms: u64, reason: impl Into<String>) -> ForegroundWindowSnapshot {
        ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(ForegroundSource::Kde),
            status: ForegroundProviderStatus::Unavailable,
            decision: ForegroundDecision {
                target: None,
                confidence: CONFIDENCE_ZERO,
                reasons: vec![ForegroundReason {
                    reason: reason.into(),
                }],
                rejected_candidates: Vec::new(),
            },
            stale_ms: None,
        }
    }

    fn error(elapsed_ms: u64, reason: impl Into<String>) -> ForegroundWindowSnapshot {
        ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(ForegroundSource::Kde),
            status: ForegroundProviderStatus::Error,
            decision: ForegroundDecision {
                target: None,
                confidence: CONFIDENCE_ZERO,
                reasons: vec![ForegroundReason {
                    reason: reason.into(),
                }],
                rejected_candidates: Vec::new(),
            },
            stale_ms: None,
        }
    }
}

impl ForegroundProvider for KdeForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::Kde
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        if !Self::is_detected() {
            return Self::unavailable(
                elapsed_ms,
                "KDE Wayland session was not detected; KDE foreground provider is unavailable",
            );
        }

        let Some(helper) = resolve_trusted_foreground_helper(&self.helper) else {
            return Self::unavailable(
                elapsed_ms,
                format!(
                    "{} was not found in trusted foreground helper paths; {KDE_UNSAFE_REASON}",
                    self.helper
                ),
            );
        };

        let output = match trusted_foreground_command(&helper).arg("--json").output() {
            Ok(output) => output,
            Err(err) => {
                return Self::error(
                    elapsed_ms,
                    format!("failed to run {} --json: {err}", helper.display()),
                );
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Self::error(
                elapsed_ms,
                format!(
                    "{} --json exited with status {}; stderr={}",
                    helper.display(),
                    output.status,
                    stderr.trim()
                ),
            );
        }

        match String::from_utf8(output.stdout) {
            Ok(stdout) => self.sample_from_helper_json(elapsed_ms, &stdout),
            Err(err) => Self::error(
                elapsed_ms,
                format!("KDE foreground helper output was not valid UTF-8: {err}"),
            ),
        }
    }
}
