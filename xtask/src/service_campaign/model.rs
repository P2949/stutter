//! Service campaign provenance and attempts reuse the Core Validation evidence vocabulary.
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::validation_campaign::model::TaskState;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Active,
    Qualified,
    Invalidated,
    Cancelled,
    /// A timing rehearsal ran its stages; it is never evidence and never becomes Qualified.
    RehearsalComplete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preflight,
    Install,
    Discovery,
    Recovery,
    Reboots,
    Soak,
    Evidence,
}
impl Stage {
    pub const ALL: [Self; 7] = [
        Self::Preflight,
        Self::Install,
        Self::Discovery,
        Self::Recovery,
        Self::Reboots,
        Self::Soak,
        Self::Evidence,
    ];
    pub const fn id(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Install => "install",
            Self::Discovery => "discovery",
            Self::Recovery => "recovery",
            Self::Reboots => "reboots",
            Self::Soak => "soak",
            Self::Evidence => "evidence",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct State {
    pub schema_version: u32,
    pub campaign_id: String,
    pub mode: Mode,
    pub repository: PathBuf,
    pub source_commit: String,
    pub manifest_sha256: String,
    pub cargo_lock_sha256: String,
    pub stutter_sha256: String,
    pub xtask_sha256: String,
    pub uid: u32,
    pub gid: u32,
    pub created_unix_ms: u64,
    pub kernel_release: String,
    /// The NixOS system closure the campaign was started on. A `switch` mid-campaign changes the
    /// host under the evidence even when the kernel does not, so integrity fails if it moves.
    #[serde(default)]
    pub host_system: String,
    /// A non-publishable timing rehearsal: one full service cycle, no reboots, never evidence.
    #[serde(default)]
    pub rehearsal: bool,
    pub required_reboots: u32,
    pub boot_ids: Vec<String>,
    pub reboot_pending: bool,
    pub tasks: BTreeMap<String, TaskState>,
}

impl State {
    /// The campaign-owned copies of the two binaries it qualifies. Cargo's `target/debug` output
    /// is a working directory that any build may relink; these bytes never change.
    pub fn pinned_dir(&self) -> std::path::PathBuf {
        self.repository
            .join(super::store::ROOT)
            .join(&self.campaign_id)
            .join("pinned")
    }
    pub fn pinned_stutter(&self) -> std::path::PathBuf {
        self.pinned_dir().join("stutter")
    }
    pub fn pinned_xtask(&self) -> std::path::PathBuf {
        self.pinned_dir().join("xtask")
    }
}
