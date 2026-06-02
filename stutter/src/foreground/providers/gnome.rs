//! GNOME Wayland foreground provider.
//!
//! This provider intentionally does not use org.gnome.Shell Eval or private
//! shell introspection. It only consumes JSON from an explicit trusted helper.

use crate::foreground::{
    command::{resolve_trusted_foreground_helper, trusted_foreground_command},
    model::{
        CONFIDENCE_ZERO, ForegroundDecision, ForegroundProviderStatus, ForegroundReason,
        ForegroundSource, ForegroundWindowSnapshot,
    },
    parse::compositor_json::snapshot_from_compositor_helper_json,
    provider::ForegroundProvider,
    providers::desktop::{desktop_looks_like_gnome, wayland_session_detected},
};

const GNOME_HELPER: &str = "stutter-gnome-foreground";
const GNOME_UNSAFE_REASON: &str = "GNOME Wayland foreground provider requires stutter-gnome-foreground; unsafe org.gnome.Shell Eval is intentionally not used";

#[derive(Debug, Clone)]
pub struct GnomeForegroundProvider {
    helper: String,
}

impl Default for GnomeForegroundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GnomeForegroundProvider {
    pub fn new() -> Self {
        Self {
            helper: GNOME_HELPER.to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_helper(mut self, helper: impl Into<String>) -> Self {
        self.helper = helper.into();
        self
    }

    pub fn is_detected() -> bool {
        wayland_session_detected() && desktop_looks_like_gnome()
    }

    pub fn sample_from_helper_json(&self, elapsed_ms: u64, json: &str) -> ForegroundWindowSnapshot {
        snapshot_from_compositor_helper_json(elapsed_ms, ForegroundSource::Gnome, "GNOME", json)
    }

    fn unavailable(elapsed_ms: u64, reason: impl Into<String>) -> ForegroundWindowSnapshot {
        ForegroundWindowSnapshot {
            elapsed_ms,
            source: Some(ForegroundSource::Gnome),
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
            source: Some(ForegroundSource::Gnome),
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

impl ForegroundProvider for GnomeForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::Gnome
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        if !Self::is_detected() {
            return Self::unavailable(
                elapsed_ms,
                "GNOME Wayland session was not detected; GNOME foreground provider is unavailable",
            );
        }

        let Some(helper) = resolve_trusted_foreground_helper(&self.helper) else {
            return Self::unavailable(
                elapsed_ms,
                format!(
                    "{} was not found in trusted foreground helper paths; {GNOME_UNSAFE_REASON}",
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
                format!("GNOME foreground helper output was not valid UTF-8: {err}"),
            ),
        }
    }
}
