//! Serialized campaign types. Internal state is typed, never arbitrary JSON.
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CampaignMode {
    Development,
    Collector,
    ReleaseCandidate,
    Complete,
    Invalidated,
}

/// Coarse position in the fixed campaign order; every task belongs to one phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum CampaignPhase {
    Preflight,
    Collection,
    Integration,
    Policy,
    ReleaseCandidate,
    Release,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TaskStatus {
    Pending,
    Ready,
    Running,
    Passed,
    Failed,
    BlockedResource,
    BlockedPrivilege,
    WaitingForReboot,
    Invalidated,
    /// Valid only when the manifest says the task is not required; never PASS evidence.
    SkippedByPolicy,
}

impl TaskStatus {
    /// Only `Passed` satisfies a dependency.
    pub fn satisfies_dependents(self) -> bool {
        self == Self::Passed
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum FailureClass {
    /// Environment hiccup worth another bounded attempt.
    Retryable,
    BlockedResource,
    BlockedPrivilege,
    /// A genuine Stutter/harness defect: stop collection, fix through a PR.
    SoftwareDefect,
    /// The implementation works but empirical acceptance failed; never rewritten to pass.
    ValidationFailure,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BlockReason {
    pub class: FailureClass,
    /// Short, command-line-free reason (status JSON never exposes private command lines).
    pub detail: String,
}

/// What a result was produced from. A passed result whose fingerprint differs from the
/// current one is historical evidence, not current completion.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskFingerprint {
    pub task: String,
    pub evidence_commit: String,
    pub manifest_sha256: String,
    pub stutter_sha256: Option<String>,
    pub xtask_sha256: Option<String>,
    pub workload_sha256: Option<String>,
    pub scenario_sha256: Option<String>,
    pub kernel_release: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AttemptOutcome {
    Passed,
    Failed {
        class: FailureClass,
        stage: String,
        summary: String,
    },
    Blocked(BlockReason),
    WaitingForReboot,
}

/// One append-only attempt; a retry adds another record and never erases this one.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskAttempt {
    pub number: u32,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    pub outcome: AttemptOutcome,
    pub fingerprint: TaskFingerprint,
    /// Campaign-relative artifact directory (raw run or bundle).
    pub artifacts: Option<String>,
    /// Campaign-relative receipt path when the task passed.
    pub receipt: Option<String>,
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaskState {
    pub id: String,
    pub status: TaskStatus,
    pub fingerprint: Option<TaskFingerprint>,
    pub attempts: Vec<TaskAttempt>,
    pub block: Option<BlockReason>,
}

impl TaskState {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            status: TaskStatus::Pending,
            fingerprint: None,
            attempts: Vec::new(),
            block: None,
        }
    }
    pub fn failed_attempts(&self) -> usize {
        self.attempts
            .iter()
            .filter(|attempt| matches!(attempt.outcome, AttemptOutcome::Failed { .. }))
            .count()
    }
}

/// A recorded policy decision (e.g. a conservative retain), kept with its limitations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CampaignDecision {
    pub task: String,
    pub decision: String,
    pub limitations: Vec<String>,
    pub unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ValidationGate {
    /// Host-independent identity of the gate (for status, strict verification and tag readiness).
    pub name: String,
    pub title: String,
    pub tasks: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CampaignState {
    pub schema_version: u32,
    pub campaign_id: String,
    pub generation: u32,
    pub mode: CampaignMode,
    pub repository: PathBuf,
    pub evidence_base: String,
    pub manifest_sha256: String,
    pub cargo_lock_sha256: String,
    pub xtask_sha256: String,
    pub stutter_sha256: String,
    pub uid: u32,
    pub gid: u32,
    pub created_unix_ms: u64,
    pub tasks: BTreeMap<String, TaskState>,
    pub decisions: Vec<CampaignDecision>,
    /// Parameters frozen before counted evidence begins (workload counts, selected IRQ, ...).
    pub frozen: BTreeMap<String, String>,
    /// Set once the release candidate is chosen.
    pub release_candidate: Option<String>,
    /// Digests of the binaries built at the release candidate; the evidence-base digests above
    /// stay untouched so collection results remain current.
    #[serde(default)]
    pub rc_xtask_sha256: Option<String>,
    #[serde(default)]
    pub rc_stutter_sha256: Option<String>,
}

/// How a `continue` run ended.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Boundary {
    Complete,
    WaitingForReboot,
    BlockedResource,
    BlockedPrivilege,
    FailureRequiresSourceChange,
    /// Stopped by `--max-tasks`, or nothing runnable remains yet nothing is blocked.
    Idle,
}
