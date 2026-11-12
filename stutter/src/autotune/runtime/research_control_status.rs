//! Read-only, explicitly incomplete campaign status.
#[cfg(feature = "wayland-probe")]
use std::time::Duration;

use super::{ResearchCoordinator, response};

impl ResearchCoordinator {
    /// Retrying an identical authenticated intent only returns existing state; it cannot create
    /// another lease or broaden authority. This lets a client resolve a lost acknowledgement.
    pub(super) fn retry_submission_status(
        &self,
        intent: &crate::autotune::research::ResearchIntentV1,
        peer_uid: u32,
    ) -> String {
        if intent.uid != peer_uid {
            return response("denied", "WrongUser");
        }
        let Ok(digest) = intent.digest() else {
            return response("denied", "InvalidIntent");
        };
        let existing = self
            .active
            .as_ref()
            .map(|active| active.intent_sha256.as_str())
            .or_else(|| {
                self.terminal
                    .as_ref()
                    .map(|terminal| terminal.intent_sha256.as_str())
            });
        if existing == Some(digest.as_str()) {
            self.status()
        } else {
            response("denied", "SingleCampaignLimit")
        }
    }

    pub(super) fn status(&self) -> String {
        if let Some(active) = &self.active {
            #[cfg(feature = "wayland-probe")]
            let desktop_idle =
                serde_json::to_value(if self.desktop_idle.0.elapsed() <= Duration::from_secs(3) {
                    self.desktop_idle.1
                } else {
                    crate::autotune::research::desktop_idle::DesktopIdleObservation::Unknown
                })
                .unwrap_or(serde_json::Value::Null);
            #[cfg(not(feature = "wayland-probe"))]
            let desktop_idle = serde_json::json!("unavailable");
            return serde_json::json!({
                "status": "ok",
                "phase": active.lease.phase(),
                "intent_sha256": active.intent_sha256,
                "execution": "BlockedSteamAutomation",
                "steam_authentication": "Unverified",
                "game_entitlement": "Unverified",
                "desktop_idle_observation": desktop_idle,
            })
            .to_string()
                + "\n";
        }
        if let Some(terminal) = &self.terminal {
            return serde_json::json!({
                "status": "ok",
                "phase": terminal.phase,
                "intent_sha256": terminal.intent_sha256,
                "code": terminal.code,
            })
            .to_string()
                + "\n";
        }
        if self.submitted {
            return response("blocked", "CampaignTerminal");
        }
        if let Some(code) = self.storage_blocker {
            return response("blocked", code);
        }
        if let Some(facts) = &self.facts {
            if facts.validation_override_present || facts.mutable_action_present {
                return response("blocked", "BlockedMutationSlot");
            }
            return serde_json::json!({
                "status": "idle",
                "code": "ReadyForIntent",
                "execution": "BlockedSteamAutomation",
                "steam_authentication": "Unverified",
                "game_entitlement": "Unverified",
                "facts": facts,
            })
            .to_string()
                + "\n";
        }
        response("idle", self.context_blocker)
    }
}
