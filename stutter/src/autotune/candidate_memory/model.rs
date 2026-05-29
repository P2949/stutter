use serde::{Deserialize, Deserializer, Serialize};
use stutter_core::ids::EmptyStringIdError;

use super::key::{CandidateContextHashInput, CandidateIdentitySummary, CandidateMemoryKey};
use crate::{
    actions::ActionId,
    autotune::{
        objective::ObjectiveKind, planning::candidate::CandidateAction, state::SituationKind,
    },
    process_tree::TaskClass,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateMemoryResult {
    Tried,
    Kept,
    Reverted,
    Faulted,
    Inconclusive,
    Rejected,
}

impl CandidateMemoryResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tried => "tried",
            Self::Kept => "kept",
            Self::Reverted => "reverted",
            Self::Faulted => "faulted",
            Self::Inconclusive => "inconclusive",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateClassCount {
    pub class: TaskClass,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateMemoryRecord {
    pub action_id: ActionId,
    pub candidate_name: String,
    pub last_tried_unix_nanos: u128,
    pub result: CandidateMemoryResult,
    pub context_hash: CandidateMemoryKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_summary: Option<CandidateIdentitySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
    /// Raw score-total delta retained for diagnostics/history only.
    ///
    /// This value is duration/sample-count sensitive and must not be used as the
    /// primary A/B keep/revert metric. Active experiment decisions use normalized
    /// `WindowScore` comparison instead.
    pub score_delta: i64,
    pub rollback_reason: Option<String>,
    pub cooldown_expires_unix_nanos: Option<u128>,
}

impl CandidateMemoryRecord {
    pub fn validate_identity_strings(&self) -> Result<(), EmptyStringIdError> {
        self.action_id.validate_non_empty()
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded_reason.is_some()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkloadActionMemory {
    pub workload_hash: String,
    pub action_id: ActionId,
    pub action_kind: String,
    pub objective: ObjectiveKind,
    pub situation: SituationKind,
    pub last_result: CandidateMemoryResult,
    /// Raw score-total delta retained as a coarse historical hint.
    ///
    /// This is not normalized by sample count or duration. It may inform
    /// diagnostics/profile confidence, but keep/revert decisions must use
    /// normalized `WindowScore` comparison.
    pub score_delta: Option<f64>,
    pub last_seen_unix_nanos: u128,
    pub cooldown_until_unix_nanos: Option<u128>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub cgroup_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_summary: Option<CandidateIdentitySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
}

impl WorkloadActionMemory {
    pub fn validate_identity_strings(&self) -> Result<(), EmptyStringIdError> {
        self.action_id.validate_non_empty()
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded_reason.is_some()
    }

    pub fn matches_context(&self, context: &CandidateContextHashInput) -> bool {
        if self.is_degraded() {
            return false;
        }

        let normalized = context.clone().normalized();
        let identity_summary_matches = self.identity_summary.as_ref().is_none_or(|summary| {
            normalized
                .workload_identity_summary()
                .as_ref()
                .is_none_or(|current| current == summary)
        });

        normalized
            .workload_identity
            .as_ref()
            .is_some_and(|identity| identity.as_str() == self.workload_hash)
            && identity_summary_matches
            && normalized
                .target_executable
                .as_ref()
                .map(|fingerprint| fingerprint.dev())
                == self.exe_dev
            && normalized
                .target_executable
                .as_ref()
                .map(|fingerprint| fingerprint.ino())
                == self.exe_ino
            && normalized.cgroup_path == self.cgroup_path
            && normalized.situation == self.situation
    }
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct CandidateMemory {
    pub records: Vec<CandidateMemoryRecord>,
    #[serde(default)]
    pub workload_actions: Vec<WorkloadActionMemory>,
}

impl CandidateMemory {
    pub fn validate_identity_strings(&self) -> Result<(), EmptyStringIdError> {
        for record in &self.records {
            record.validate_identity_strings()?;
        }
        for workload_action in &self.workload_actions {
            workload_action.validate_identity_strings()?;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for CandidateMemory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct CandidateMemoryWire {
            #[serde(default)]
            records: Vec<CandidateMemoryRecord>,
            #[serde(default)]
            workload_actions: Vec<WorkloadActionMemory>,
        }

        let wire = CandidateMemoryWire::deserialize(deserializer)?;
        let mut memory = Self {
            records: wire.records,
            workload_actions: wire.workload_actions,
        };

        memory
            .validate_identity_strings()
            .map_err(serde::de::Error::custom)?;

        super::collision::mark_loaded_identity_collisions(&mut memory);
        Ok(memory)
    }
}

pub struct CandidateResultRecordInput<'a> {
    pub candidate: &'a CandidateAction,
    pub context: &'a CandidateContextHashInput,
    pub now_unix_nanos: u128,
    pub result: CandidateMemoryResult,
    pub diagnostic_baseline_raw_score_total: Option<u64>,
    pub diagnostic_current_raw_score_total: Option<u64>,
    pub rollback_reason: Option<String>,
    pub cooldown_expires_unix_nanos: Option<u128>,
}
