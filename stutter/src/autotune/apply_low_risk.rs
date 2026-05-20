use crate::{
    actions::cpu_affinity::CpuAffinityProfileAction, autotune::candidate::CandidateAction,
};

mod audit;
#[cfg(test)]
mod command;
#[cfg(test)]
mod executor;
#[cfg(test)]
mod experiment;
mod model;
#[cfg(test)]
mod target;

pub use audit::action_from_candidate;
#[cfg(test)]
pub(crate) use audit::{
    AuditedRollbackGuard, append_low_risk_history_event,
    apply_cpu_affinity_candidate_with_audit_hooks,
    apply_cpu_affinity_candidate_with_audit_path_for_tests,
    controller_journal_hooks_for_low_risk_action,
    controller_journal_metadata_for_cpu_affinity_action,
};

#[cfg(test)]
pub(crate) use crate::autotune::washout::WashoutWindowConfig;

#[cfg(test)]
pub fn ensure_low_risk_action_allowed(
    action_kind: &str,
    safety_class: &crate::actions::SafetyClass,
) -> anyhow::Result<()> {
    audit::ensure_low_risk_action_allowed(action_kind, safety_class)
}
#[cfg(test)]
pub use command::apply_low_risk_command;
#[cfg(test)]
pub use executor::{
    CpuAffinityLowRiskExecutor, LowRiskActionExecutor, executor_for_low_risk_candidate,
    run_apply_low_risk_candidate, run_apply_low_risk_with_executor,
};
#[cfg(test)]
pub use experiment::{
    compare_low_risk_experiment, ensure_baseline_ready_for_apply,
    ensure_candidate_measurement_ready_for_decision, plan_apply_low_risk_from_profiles,
    resolve_low_risk_experiment, resolve_low_risk_experiment_with_active_profile_state,
    select_first_eligible_low_risk_candidate,
};
pub type AuditedCandidateApplyOutcome = model::AuditedCandidateApplyOutcome;
#[cfg(test)]
pub use model::{ApplyLowRiskOutcome, ApplyLowRiskPlan};
#[cfg(test)]
pub use target::{resolve_one_target_tree_pid, resolve_one_target_tree_pid_at};

pub fn apply_cpu_affinity_candidate_with_audit(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    audit::apply_cpu_affinity_candidate_with_audit(candidate_name, action)
}

pub fn apply_candidate_with_audit(
    candidate: CandidateAction,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    let (candidate_name, action) = action_from_candidate(candidate)?;
    apply_cpu_affinity_candidate_with_audit(candidate_name, &action)
}

fn unsupported_low_risk_candidate<T>(candidate: &CandidateAction) -> anyhow::Result<T> {
    anyhow::bail!(
        "apply-low-risk supports CPU-affinity profile actions only; candidate '{}' action_kind={} safety={:?} required_mode={}",
        candidate.candidate_name(),
        candidate.action_kind(),
        candidate.safety_class(),
        crate::daemon_policy::DaemonMode::ApplyMediumRisk
    )
}

#[cfg(test)]
#[path = "apply_low_risk_tests/mod.rs"]
mod tests;
