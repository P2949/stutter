//! Read-only, explicitly incomplete campaign status.
#[cfg(feature = "wayland-probe")]
use std::time::Duration;

use super::{ResearchCoordinator, response};

impl ResearchCoordinator {
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
