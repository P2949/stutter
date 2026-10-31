//! Candidate dry-run execution helpers.

use serde::{Deserialize, Serialize};

use super::candidate::CandidateAction;
use crate::actions::{ActionState, ActionWarning, SafetyClass, TuningAction};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateDryRunRecord {
    pub candidate_name: String,
    pub affected_tasks: usize,
    pub warnings: Vec<ActionWarning>,
    pub safety_class: SafetyClass,
    pub eligible: bool,
    pub reason: Option<String>,
}

pub trait CandidateDryRunner {
    fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord;
}

#[derive(Default)]
pub struct RealCandidateDryRunner;

/// Installed controllers preview using the same privileged authority that will apply the plan.
/// An unprivileged preview must not mistake a root-owned destination for an unusable operation.
pub(crate) struct WorkerCandidateDryRunner<'a> {
    pub service: &'a dyn crate::daemon::privilege::PrivilegedActionService,
    pub policy: &'a crate::daemon_policy::DaemonPolicy,
    pub now_unix_nanos: u128,
}

impl CandidateDryRunner for WorkerCandidateDryRunner<'_> {
    fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
        use crate::daemon::privilege::{CandidateApplyRequest, CandidatePlanRequest};
        self.service
            .dry_run_candidate(CandidateApplyRequest {
                plan: CandidatePlanRequest::from_candidate(candidate.clone(), self.now_unix_nanos),
                policy: self.policy.clone(),
                context: Default::default(),
                max_plan_age_nanos: std::time::Duration::from_secs(30).as_nanos(),
            })
            .unwrap_or_else(|error| CandidateDryRunRecord {
                candidate_name: candidate.candidate_name().to_owned(),
                affected_tasks: 0,
                warnings: vec![],
                safety_class: candidate.safety_class(),
                eligible: false,
                reason: Some(format!("privileged preview refused: {error:#}")),
            })
    }
}

impl CandidateDryRunner for RealCandidateDryRunner {
    fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
        dry_run_candidate(candidate)
    }
}

pub fn dry_run_candidates(candidates: &[CandidateAction]) -> Vec<CandidateDryRunRecord> {
    let mut runner = RealCandidateDryRunner;
    dry_run_candidates_with_runner(candidates, &mut runner)
}

pub fn dry_run_candidates_with_runner<R: CandidateDryRunner>(
    candidates: &[CandidateAction],
    runner: &mut R,
) -> Vec<CandidateDryRunRecord> {
    candidates
        .iter()
        .map(|candidate| runner.dry_run(candidate))
        .collect()
}

pub fn dry_run_record_from_action_state(
    candidate_name: String,
    safety_class: SafetyClass,
    state: ActionState,
) -> CandidateDryRunRecord {
    let affected_tasks = state.affected_tasks;
    CandidateDryRunRecord {
        candidate_name,
        affected_tasks,
        warnings: state.warnings,
        safety_class,
        eligible: affected_tasks > 0,
        reason: if affected_tasks == 0 {
            Some("dry-run matched zero affected tasks".to_owned())
        } else {
            None
        },
    }
}

pub fn dry_run_candidate(candidate: &CandidateAction) -> CandidateDryRunRecord {
    let candidate_name = candidate.candidate_name().to_owned();
    let safety_class = candidate.safety_class();

    match crate::actions::default_action_factory_registry().build(candidate) {
        Ok(action) => dry_run_planned_action(candidate_name, safety_class, &action),
        Err(err) => CandidateDryRunRecord {
            candidate_name,
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class,
            eligible: false,
            reason: Some(format!("dry-run action build failed: {err}")),
        },
    }
}

fn dry_run_planned_action<A: TuningAction>(
    candidate_name: String,
    safety_class: SafetyClass,
    action: &A,
) -> CandidateDryRunRecord {
    match action.dry_run() {
        Ok(state) => dry_run_record_from_action_state(candidate_name, safety_class, state),
        Err(err) => CandidateDryRunRecord {
            candidate_name,
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class,
            eligible: false,
            reason: Some(format!("dry-run failed: {err:#}")),
        },
    }
}

#[cfg(test)]
mod worker_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::daemon::privilege::*;

    #[derive(Debug, Default)]
    struct RefusingWorker(AtomicUsize);
    impl PrivilegedActionService for RefusingWorker {
        fn dry_run_candidate(
            &self,
            request: CandidateApplyRequest,
        ) -> anyhow::Result<CandidateDryRunRecord> {
            self.0.fetch_add(1, Ordering::Relaxed);
            assert_eq!(
                request.plan.operation,
                request.plan.candidate.operation_kind()
            );
            anyhow::bail!("test_root_policy_denied")
        }
        fn apply_candidate(&self, _: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
            panic!("preview must never apply")
        }
        fn rollback(&self, _: RollbackRequest) -> anyhow::Result<RollbackResult> {
            panic!("preview must never restore")
        }
    }

    #[test]
    fn service_preview_uses_worker_authority_and_refusal_has_no_local_fallback() {
        let service = RefusingWorker::default();
        let policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        let candidate = CandidateAction::cpu_affinity_profile_for(
            crate::profiles::Profile {
                name: "worker-preview".into(),
                rules: vec![],
            },
            std::process::id(),
            crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
        );
        let record = WorkerCandidateDryRunner {
            service: &service,
            policy: &policy,
            now_unix_nanos: crate::audit::unix_nanos_now(),
        }
        .dry_run(&candidate);
        assert!(!record.eligible);
        assert_eq!(record.affected_tasks, 0);
        assert!(record.reason.unwrap().contains("test_root_policy_denied"));
        assert_eq!(service.0.load(Ordering::Relaxed), 1);
    }
}
