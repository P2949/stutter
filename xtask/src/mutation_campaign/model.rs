//! Serialized mutation-campaign state. Task bookkeeping reuses the Core Validation types
//! (attempts are append-only, fingerprints bind results to their source), so both campaigns read
//! the same way to a script.
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::validation_campaign::model::TaskState;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MutationMode {
    Active,
    /// Every stage ran and the tier decision was recorded.
    Complete,
    Cancelled,
    /// Source or binaries changed; the evidence is historical and a successor must be started.
    Invalidated,
}

/// What a graduation attempt concluded. Never forced: `Supported` needs every stage to pass.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Disposition {
    Supported,
    DeferredEvidence,
    ExcludedFromAutonomy,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TierDecision {
    pub disposition: Disposition,
    pub reasons: Vec<String>,
    pub decided_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MutationState {
    pub schema_version: u32,
    pub campaign_id: String,
    pub generation: u32,
    pub mode: MutationMode,
    pub repository: PathBuf,
    pub evidence_base: String,
    pub manifest_sha256: String,
    pub cargo_lock_sha256: String,
    pub xtask_sha256: String,
    pub stutter_sha256: String,
    pub uid: u32,
    pub gid: u32,
    pub created_unix_ms: u64,
    pub operation: String,
    pub objective: String,
    /// The Core Validation campaign that anchors the privileged runner's authorization.
    pub companion_campaign: String,
    /// Stage id -> state; skipped stages are absent and listed in `skipped`.
    pub tasks: BTreeMap<String, TaskState>,
    /// Explicitly skipped stages with their manifest reasons.
    pub skipped: BTreeMap<String, String>,
    pub decision: Option<TierDecision>,
    /// Parameters frozen before counted evidence begins.
    pub frozen: BTreeMap<String, String>,
}
