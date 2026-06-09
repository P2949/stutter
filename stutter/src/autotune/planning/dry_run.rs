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
