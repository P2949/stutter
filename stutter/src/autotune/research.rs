//! Unprivileged, bounded research request data. A digest is an identity, not authorization.
//!
//! Admission must additionally authenticate the caller and check the live session, policy,
//! installation, source identities, and exclusive resident lease before any launch or mutation.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod action_bridge;
pub(crate) mod action_child;
pub(crate) mod action_coverage;
pub(crate) mod action_gate;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "judges the real CP-LONG action study once it can run"
    )
)]
pub(crate) mod action_study;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "P5 analysis is consumed once the scheduler drives real runs"
    )
)]
pub(crate) mod analysis;
pub(crate) mod arm_selection;
pub(crate) mod cache_state;
pub(crate) mod campaign_budget;
#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod child_intent;
pub(crate) mod child_ledger;
#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod child_runner;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "drives the scheduler once child-study execution is wired"
    )
)]
pub(crate) mod child_study;
pub(crate) mod child_verify;
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
pub(crate) mod deploy_receipt;
#[cfg(feature = "wayland-probe")]
pub(crate) mod desktop_idle;
pub(crate) mod dispatch_context;
pub(crate) mod dispatch_select;
pub(crate) mod dispatcher;
pub(crate) mod driver;
#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod driver_attempts;
pub(crate) mod entitlement;
pub(crate) mod fault_matrix;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "arm environments are consumed by the real Steam host once it is qualified"
    )
)]
pub(crate) mod features;
pub(crate) mod final_bundle;
pub(crate) mod functional_probe;
pub(crate) mod funnel_driver;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "composed into the production host once the live launch is qualified"
    )
)]
pub(crate) mod game_facts;
pub(crate) mod gate_verdict;
pub(crate) mod gpu_health;
pub(super) mod host;
pub(crate) mod host_baseline;
pub(crate) mod host_hardware;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed by the resident capture once a live game source is qualified"
    )
)]
pub(crate) mod join;
mod journal;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "read by recovery once the launch transaction is wired into the driver"
    )
)]
pub(crate) mod launch_record;
pub(crate) mod launch_ticket;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "composed with the verified user manager once the live launch is qualified"
    )
)]
pub(crate) mod launch_txn;
mod lease;
pub(crate) mod lease_inspect;
pub(crate) mod loaded_runtime;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "staged: no physical long-session driver exists to produce a record"
    )
)]
pub(crate) mod long_receipt;
pub(crate) mod long_session;
mod mounts;
pub(crate) mod native_context;
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
        reason = "supplied with the production fact provider once the live launch is qualified"
    )
)]
pub(crate) mod native_source;
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
        reason = "staged: cases need a live controller diagnosis for an injected cause"
    )
)]
pub(crate) mod oracle_cases;
pub(crate) mod owned_cleanup;
pub(crate) mod owned_domain;
pub(crate) mod pilot_rule;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "staged: the supervised initialization launch is a live action, not yet run"
    )
)]
pub(crate) mod prefix_init;
pub(crate) mod profile_mappings;
pub(crate) mod protected_state;
pub(crate) mod replication;
pub(crate) mod telemetry_observer;
pub(crate) mod treatment;
pub use treatment::{Treatment, TreatmentSpec};
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "P3 prefix proof is consumed only after the Steam dispatcher is qualified"
    )
)]
mod prefix;
#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod prefix_identity;
pub(crate) mod production_host;
#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod profile_applier;
#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod profile_hook;
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
pub(crate) mod raw_verify;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "called by RecoveryJob once launch history recovery is enabled"
    )
)]
pub(crate) mod recovery_gather;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed by post-launch recovery once its witnesses are wired"
    )
)]
pub(crate) mod recovery_plan;
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
pub(crate) mod session_host;
pub(crate) mod settlement;
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
#[cfg_attr(not(test), expect(dead_code, reason = "staged"))]
pub(crate) mod study_executor;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "written beside report.json once the final report is generated"
    )
)]
pub(crate) mod summary;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the driver calls it once the transient launch replaces the child spawn"
    )
)]
pub(crate) mod transient_launch;
mod unix_diag;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "user-session handoff is not enabled until its ownership and recovery gates are connected"
    )
)]
pub(crate) mod user_launcher;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "user-manager identity is not used until launch settlement and recovery are connected"
    )
)]
pub(crate) mod user_manager;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the physical launch stays gated by UnqualifiedSteamHost until qualification"
    )
)]
pub(crate) mod user_manager_port;
mod vdf;
pub(crate) mod verdict;
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

impl ProtonSource {
    /// The private per-arm directory name (the serialized name): every arm gets its own compatibility
    /// prefix, so no arm inherits another arm's prefix state, settings or results.
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::ValveStable => "valve_stable",
            Self::ValveExperimental => "valve_experimental",
            Self::CachyOs => "cachy_os",
            Self::CachyOsV3 => "cachy_os_v3",
            Self::Ge => "ge",
            Self::Em => "em",
        }
    }
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
    /// Set only on the intent of one child study: the digest of that study in the frozen funnel.
    /// It makes the child's schedule, journal and report distinct from its siblings'. Absent on a
    /// whole-campaign intent, where it is not serialized, so existing digests are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_study_sha256: Option<String>,
    /// Only on a confirmation study's intent: the rule its pilot froze, digest-bound before any run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_rule: Option<pilot_rule::FrozenConfirmationRule>,
    /// Only on a feature or action study's intent: the factor or action it switches off and on over
    /// one fixed Proton build (see `treatment`). Absent otherwise, so existing digests are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub treatment: Option<TreatmentSpec>,
    /// The longest one attempt of a long-profile study may run (CP-LONG, PLAN section 7), authorized
    /// separately from `max_attempt_seconds`, which stays the short benchmark limit for every other
    /// attempt. Absent unless the operator authorized a long window, so existing digests are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_attempt_seconds: Option<u32>,
    /// Only on a child study's intent: whether its arms ran over cold or warm prefixes, observed before
    /// the study was created and sealed with it (a sidecar label could be edited after the fact).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_cohort: Option<String>,
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
        if let Some(rule) = &self.confirmation_rule {
            anyhow::ensure!(
                self.child_study_sha256.is_some() && rule.is_consistent(),
                "invalid confirmation rule"
            );
        }
        if let Some(spec) = self.treatment {
            let authorized = match spec {
                TreatmentSpec::Factor(factor) => self.factors.contains(&factor),
                TreatmentSpec::Action(action) => self.actions.contains(&action),
            };
            anyhow::ensure!(
                self.child_study_sha256.is_some()
                    && self.confirmation_rule.is_none()
                    && self.tools.len() == 1
                    && authorized,
                "invalid treatment"
            );
        }
        if let Some(child) = &self.child_study_sha256 {
            anyhow::ensure!(
                child.len() == 64
                    && child
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid child study digest"
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
            long_session::attempt_limits_valid(self),
            "invalid attempt time limits"
        );
        anyhow::ensure!(
            self.max_owned_pressure_seconds_per_study <= 600
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
#[path = "research_tests.rs"]
mod tests;
