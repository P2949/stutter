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
    pub required_reboots: u32,
    pub boot_ids: Vec<String>,
    pub reboot_pending: bool,
    pub tasks: BTreeMap<String, TaskState>,
}
