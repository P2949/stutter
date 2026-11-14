//! Unprivileged, bounded research request data. A digest is an identity, not authorization.
//!
//! Admission must additionally authenticate the caller and check the live session, policy,
//! installation, source identities, and exclusive resident lease before any launch or mutation.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "P5 analysis is consumed once the scheduler drives real runs"
    )
)]
pub(crate) mod analysis;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "emitted by the final report once the campaign runs"
    )
)]
pub(crate) mod crosswalk;
#[expect(
    dead_code,
    reason = "P2's unprivileged verifier remains inactive until the owned Steam launcher is qualified"
)]
pub(crate) mod cyberpunk;
#[cfg(feature = "wayland-probe")]
pub(crate) mod desktop_idle;
pub(crate) mod dispatch_context;
pub(crate) mod dispatcher;
pub(crate) mod driver;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "arm environments are consumed by the real Steam host once it is qualified"
    )
)]
pub(crate) mod features;
pub(super) mod host;
mod journal;
pub(crate) mod launch_ticket;
mod lease;
mod mounts;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "requires a qualified resident game observation source"
    )
)]
pub(crate) mod native_probe;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "scores the live diagnosis once real pre-registered cases exist"
    )
)]
pub(crate) mod oracle;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "P3 prefix proof is consumed only after the Steam dispatcher is qualified"
    )
)]
mod prefix;
mod protocol;
#[path = "research/proton/mod.rs"]
pub(crate) mod proton;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed by the resident capture join once a live game source is qualified"
    )
)]
pub(crate) mod quality;
pub(crate) mod report;
pub(crate) mod report_store;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "P5 scheduling is consumed once the launcher is qualified"
    )
)]
pub(crate) mod scheduler;
mod session_env;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the production SteamPort is qualified against a live Steam first"
    )
)]
pub(crate) mod steam_host;
mod steam_inventory;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into the resident host after live Steam qualification"
    )
)]
pub(crate) mod steam_port;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "kernel peer diagnostics are not yet connected to the session handoff"
    )
)]
mod unix_diag;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "user-session handoff is not enabled until its ownership and recovery gates are connected"
    )
)]
pub(crate) mod user_launcher;
mod vdf;
pub use journal::{AttemptOutcome, AttemptRecord, AttemptSlot, ResearchJournal};
pub use lease::{CampaignBlockReason, CampaignFacts, CampaignLease};
pub(crate) use protocol::read_frame as read_control_frame;
pub use protocol::{AuthenticatedRequest, ResearchRequest, receive_request};
pub(crate) use proton::{SteamClientIdentity, SteamProcessIdentity, inspect_proton_inventory};
pub use steam_inventory::{
    CyberpunkInstallation, SteamUpdateStatus, inspect_cyberpunk_installation,
};
pub(crate) use steam_inventory::{discover_cyberpunk_installation, discover_steam_libraries};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchGame {
    Cyberpunk2077,
}

/// Identity of the independently checked private study root, without exposing its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudyRootIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtonSource {
    ValveStable,
    ValveExperimental,
    CachyOs,
    CachyOsV3,
    Ge,
    Em,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchFactor {
    WineWayland,
    Vkd3dLowLatency,
    NtSync,
    Fsr4,
    FrameGeneration,
    AntiLag2,
    Gamescope,
    AudioBackend,
    InputBackend,
    Hdr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchAction {
    CpuAffinityProfile,
    CgroupPlacement,
}

/// Campaign progress only. State does not itself authorize any process or mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchPhase {
    Requested,
    Preflight,
    Prepared,
    AwaitSafeWindow,
    Paused,
    Launching,
    Attesting,
    Warmup,
    Measuring,
    Sealing,
    Analysing,
    NextArm,
    Diagnosing,
    EligibleAction,
    Confirming,
    Restoring,
    Completed,
    Blocked,
    Aborted,
}

impl ResearchPhase {
    /// Check a transition; persistence and side effects are the resident controller's duty.
    pub fn permits(self, next: Self) -> bool {
        use ResearchPhase::*;
        matches!(
            (self, next),
            (Requested, Preflight)
                | (Preflight, Prepared)
                | (Prepared, AwaitSafeWindow)
                | (AwaitSafeWindow, Paused)
                | (Paused, AwaitSafeWindow)
                | (AwaitSafeWindow, Launching)
                | (Launching, Attesting)
                | (Attesting, Warmup)
                | (Warmup, Measuring)
                | (Launching | Attesting | Warmup | Measuring, Sealing)
                | (Sealing, Analysing)
                | (Analysing, NextArm)
                | (NextArm, AwaitSafeWindow)
                | (Analysing, Diagnosing)
                | (Diagnosing, EligibleAction)
                | (EligibleAction, Confirming)
                | (Confirming, Sealing)
                | (Restoring, Completed | Blocked | Aborted)
        ) || (!matches!(self, Completed | Blocked | Aborted | Restoring) && next == Restoring)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchTool {
    pub source: ProtonSource,
    /// Digest of the frozen tool identity; actual loaded code still needs attestation.
    pub identity_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchIntentV1 {
    pub schema_version: u32,
    pub game: ResearchGame,
    pub app_id: u32,
    pub uid: u32,
    pub boot_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub study_root: Option<StudyRootIdentity>,
    /// The user's selected Steam Client Experimental branch and exact installed package identity.
    pub steam_client: SteamClientIdentity,
    /// Digest of the declared full Git revision and running binary SHA-256. Build provenance is
    /// independently qualified at deployment; this digest alone does not prove a clean tree.
    pub source_sha256: String,
    pub policy_sha256: String,
    pub game_executable_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_executable_sha256: Option<String>,
    pub build_id: u64,
    pub tools: Vec<ResearchTool>,
    pub factors: Vec<ResearchFactor>,
    pub actions: Vec<ResearchAction>,
    pub issued_unix_seconds: u64,
    pub deadline_unix_seconds: u64,
    pub max_active_seconds: u32,
    pub max_attempt_seconds: u32,
    pub max_owned_pressure_seconds_per_study: u32,
    pub max_attempts: u32,
    pub max_evidence_bytes: u64,
    pub desktop_research_consented: bool,
}

impl ResearchIntentV1 {
    /// Validate the closed contract. This alone never grants execution authority.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.schema_version == 1, "unsupported intent schema");
        anyhow::ensure!(
            self.app_id == 1_091_500 && self.build_id != 0,
            "invalid game identity"
        );
        self.steam_client.validate()?;
        anyhow::ensure!(
            self.study_root.is_none_or(|root| root.inode != 0),
            "invalid study root identity"
        );
        anyhow::ensure!(
            self.steam_client.branch == "publicbeta",
            "Cyberpunk research requires the installed Steam Client Experimental branch"
        );
        anyhow::ensure!(
            self.steam_client
                .process
                .as_ref()
                .is_some_and(|process| process.logged_in),
            "Steam Client Experimental has no nonzero account hint"
        );
        anyhow::ensure!(
            self.uid != 0 && self.desktop_research_consented,
            "missing user consent"
        );
        for value in [&self.boot_id, &self.session_id] {
            anyhow::ensure!(
                !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "invalid session identity"
            );
        }
        for digest in [
            &self.source_sha256,
            &self.policy_sha256,
            &self.game_executable_sha256,
        ]
        .into_iter()
        .chain(self.launcher_executable_sha256.iter())
        .chain(self.tools.iter().map(|tool| &tool.identity_sha256))
        {
            anyhow::ensure!(
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid sha256 identity"
            );
        }
        anyhow::ensure!(
            !self.tools.is_empty() && self.tools.len() <= 6,
            "invalid tool count"
        );
        anyhow::ensure!(
            self.tools
                .iter()
                .map(|t| t.source)
                .collect::<BTreeSet<_>>()
                .len()
                == self.tools.len(),
            "duplicate tool source"
        );
        anyhow::ensure!(
            self.tools
                .iter()
                .map(|t| &t.identity_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                == self.tools.len(),
            "duplicate tool identity"
        );
        anyhow::ensure!(
            self.factors.iter().collect::<BTreeSet<_>>().len() == self.factors.len(),
            "duplicate factor"
        );
        anyhow::ensure!(
            self.actions.iter().collect::<BTreeSet<_>>().len() == self.actions.len(),
            "duplicate action"
        );
        let wall = self
            .deadline_unix_seconds
            .saturating_sub(self.issued_unix_seconds);
        anyhow::ensure!((1..=43_200).contains(&wall), "invalid wall deadline");
        anyhow::ensure!(
            self.max_active_seconds > 0
                && self.max_active_seconds <= 28_800
                && u64::from(self.max_active_seconds) <= wall,
            "invalid active budget"
        );
        anyhow::ensure!(
            (1..=720).contains(&self.max_attempt_seconds)
                && self.max_attempt_seconds <= self.max_active_seconds
                && self.max_owned_pressure_seconds_per_study <= 600
                && self.max_owned_pressure_seconds_per_study <= self.max_active_seconds
                && self.max_attempts > 0
                && self.max_attempts <= 1_000,
            "invalid attempt budget"
        );
        anyhow::ensure!(
            self.max_evidence_bytes > 0 && self.max_evidence_bytes <= 20 * 1024 * 1024 * 1024,
            "invalid evidence budget"
        );
        Ok(())
    }

    pub fn digest(&self) -> anyhow::Result<String> {
        self.validate()?;
        Ok(stutter_core::hash::sha256_hex(&serde_json::to_vec(self)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn intent() -> ResearchIntentV1 {
        ResearchIntentV1 {
            schema_version: 1,
            game: ResearchGame::Cyberpunk2077,
            app_id: 1_091_500,
            uid: 1000,
            boot_id: "boot-1".into(),
            session_id: "11".into(),
            study_root: None,
            steam_client: SteamClientIdentity {
                branch: "publicbeta".into(),
                steamrt64_build_id: 1,
                steamrt64_manifest_sha256: "e".repeat(64),
                installed_index_sha256: "f".repeat(64),
                identity_sha256: "1".repeat(64),
                process: Some(SteamProcessIdentity {
                    steam_pid: 10,
                    steam_start_ticks: 1,
                    webhelper_pid: 11,
                    webhelper_start_ticks: 2,
                    logged_in: true,
                }),
            },
            source_sha256: "a".repeat(64),
            policy_sha256: "b".repeat(64),
            game_executable_sha256: "c".repeat(64),
            launcher_executable_sha256: None,
            build_id: 1,
            tools: vec![ResearchTool {
                source: ProtonSource::Ge,
                identity_sha256: "d".repeat(64),
            }],
            factors: vec![],
            actions: vec![],
            issued_unix_seconds: 10,
            deadline_unix_seconds: 3610,
            max_active_seconds: 3600,
            max_attempt_seconds: 720,
            max_owned_pressure_seconds_per_study: 600,
            max_attempts: 10,
            max_evidence_bytes: 1024,
            desktop_research_consented: true,
        }
    }

    #[test]
    fn contract_roundtrip_and_digest_are_stable() {
        let value = intent();
        let encoded = serde_json::to_vec(&value).unwrap();
        let decoded: ResearchIntentV1 = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(value, decoded);
        assert_eq!(value.digest().unwrap(), decoded.digest().unwrap());
        let mut changed = decoded;
        changed.max_attempts += 1;
        assert_ne!(value.digest().unwrap(), changed.digest().unwrap());
    }

    #[test]
    fn malformed_or_unbounded_requests_are_rejected() {
        let mut value = intent();
        value.tools.push(value.tools[0].clone());
        assert!(value.validate().is_err());
        value = intent();
        value.policy_sha256 = "A".repeat(64);
        assert!(value.validate().is_err());
        value = intent();
        value.steam_client.branch = "public".into();
        assert!(value.validate().is_err());
        value = intent();
        value.steam_client.process.as_mut().unwrap().logged_in = false;
        assert!(value.validate().is_err());
        value = intent();
        value.deadline_unix_seconds = 50_000;
        assert!(value.validate().is_err());
        value = intent();
        value.desktop_research_consented = false;
        assert!(value.validate().is_err());
        value = intent();
        value.actions = vec![ResearchAction::CpuAffinityProfile; 2];
        assert!(value.validate().is_err());
        let mut json = serde_json::to_value(intent()).unwrap();
        json["command"] = "arbitrary".into();
        assert!(serde_json::from_value::<ResearchIntentV1>(json).is_err());
        let mut json = serde_json::to_value(intent()).unwrap();
        json["game"] = "unknown".into();
        assert!(serde_json::from_value::<ResearchIntentV1>(json).is_err());
    }

    #[test]
    fn terminal_states_require_restoration_and_cannot_restart() {
        use ResearchPhase::*;
        assert!(Requested.permits(Preflight));
        assert!(Measuring.permits(Restoring));
        assert!(!Measuring.permits(Completed));
        assert!(Restoring.permits(Blocked));
        assert!(!Blocked.permits(Launching));
        assert!(!Sealing.permits(Launching));
    }
}
