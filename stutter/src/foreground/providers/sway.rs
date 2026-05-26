//! Sway foreground provider.
//!
//! Owns `swaymsg get_tree` process execution and conversion of parsed Sway trees into foreground
//! snapshots. Does not own Sway tree data-model traversal details or resolver stale handling.

use crate::foreground::{
    command::{resolve_trusted_foreground_helper, trusted_foreground_command},
    model::{
        CONFIDENCE_ZERO, ForegroundDecision, ForegroundProviderStatus, ForegroundReason,
        ForegroundSource, ForegroundWindowSnapshot,
    },
    parse::sway::{SwayNode, focused_sway_snapshot_from_tree},
    provider::ForegroundProvider,
};

#[derive(Debug, Clone)]
pub struct SwayForegroundProvider {
    swaymsg: String,
}

impl Default for SwayForegroundProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SwayForegroundProvider {
    pub fn new() -> Self {
        Self {
            swaymsg: String::new(),
        }
        .with_swaymsg("swaymsg")
    }

    pub fn with_swaymsg(mut self, swaymsg: impl Into<String>) -> Self {
        self.swaymsg = swaymsg.into();
        self
    }

    pub fn is_detected() -> bool {
        std::env::var("SWAYSOCK").is_ok()
    }

    pub fn sample_from_tree_json(
        &self,
        elapsed_ms: u64,
        tree_json: &str,
    ) -> ForegroundWindowSnapshot {
        match serde_json::from_str::<SwayNode>(tree_json) {
            Ok(root) => focused_sway_snapshot_from_tree(elapsed_ms, &root),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Error,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!("failed to parse swaymsg get_tree JSON: {err}"),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            },
        }
    }
}

impl ForegroundProvider for SwayForegroundProvider {
    fn source(&self) -> ForegroundSource {
        ForegroundSource::Sway
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        if !Self::is_detected() {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Unavailable,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: "SWAYSOCK is not set; Sway foreground provider is unavailable"
                            .to_owned(),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            };
        }

        let Some(swaymsg) = resolve_trusted_foreground_helper(&self.swaymsg) else {
            return ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Unavailable,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!(
                            "{} was not found in trusted foreground helper paths; Sway foreground provider is unavailable",
                            self.swaymsg
                        ),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            };
        };

        let output = match trusted_foreground_command(&swaymsg)
            .args(["-t", "get_tree", "-r"])
            .output()
        {
            Ok(output) => output,
            Err(err) => {
                return ForegroundWindowSnapshot {
                    elapsed_ms,
                    source: Some(ForegroundSource::Sway),
                    status: ForegroundProviderStatus::Error,
                    decision: ForegroundDecision {
                        target: None,
                        confidence: CONFIDENCE_ZERO,
                        reasons: vec![ForegroundReason {
                            reason: format!(
                                "failed to run {} -t get_tree -r: {err}",
                                swaymsg.display()
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
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Error,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!(
                            "{} -t get_tree -r exited with status {}; stderr={}",
                            swaymsg.display(),
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
            Ok(stdout) => self.sample_from_tree_json(elapsed_ms, &stdout),
            Err(err) => ForegroundWindowSnapshot {
                elapsed_ms,
                source: Some(ForegroundSource::Sway),
                status: ForegroundProviderStatus::Error,
                decision: ForegroundDecision {
                    target: None,
                    confidence: CONFIDENCE_ZERO,
                    reasons: vec![ForegroundReason {
                        reason: format!("swaymsg get_tree output was not valid UTF-8: {err}"),
                    }],
                    rejected_candidates: Vec::new(),
                },
                stale_ms: None,
            },
        }
    }
}
