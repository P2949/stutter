//! Apply-low-risk public data transfer models.

use super::*;

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct ApplyLowRiskPlan {
    pub tree_pid: u32,
    pub profiles_path: PathBuf,
    pub candidate: CandidateAction,
    pub dry_run_record: CandidateDryRunRecord,
    pub duration: Duration,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct ApplyLowRiskOutcome {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: usize,
    pub safety_class: SafetyClass,
    pub rollback_performed: bool,
}

#[derive(Clone, Debug)]
pub struct AuditedCandidateApplyOutcome {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: usize,
    pub safety_class: SafetyClass,
    pub state: ActionState,
    pub rollback: RollbackToken,
}
