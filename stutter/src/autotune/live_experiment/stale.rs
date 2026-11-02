//! A candidate abandoned before its first write because a target exited: a stale candidate to
//! look at again, not a fault.
use super::{
    executor::CleanPrecommitAbort,
    model::{LiveExperimentEvent, LiveExperimentHistoryContext, LiveExperimentOutcome},
};
use crate::{
    autotune::{
        controller::ControllerRuntimeState,
        experiment::{ExperimentId, WindowScore},
        observation::AutotuneObservation,
        planning::candidate::CandidateAction,
        state::ControllerPhase,
    },
    daemon_policy::DaemonMode,
};

/// `Some` when `error` says the candidate was abandoned before any write with the journal and the
/// root token both clean. Nothing was changed and nothing is outstanding, so the controller looks
/// again, does not retry the same action for a moment, and stays alive.
pub(super) fn abandoned_before_mutation(
    controller_state: &mut ControllerRuntimeState,
    observation: &AutotuneObservation,
    error: &anyhow::Error,
    candidate: &CandidateAction,
    (mode, experiment_id, baseline): (DaemonMode, &ExperimentId, &WindowScore),
) -> Option<LiveExperimentOutcome> {
    error.downcast_ref::<CleanPrecommitAbort>()?;
    let retry_after = observation
        .now_unix_nanos
        .saturating_add(std::time::Duration::from_secs(15).as_nanos());
    controller_state.record_candidate_attempt(candidate, observation, None, Some(retry_after));
    controller_state.phase = ControllerPhase::Observing;
    log::warn!(
        "autotune_candidate_abandoned_stale_target candidate={} reason=\"{error:#}\"",
        candidate.profile_name()
    );
    Some(LiveExperimentOutcome {
        event: LiveExperimentEvent::CooldownEntered,
        history_context: Some(LiveExperimentHistoryContext {
            experiment_id: experiment_id.clone(),
            action_id: candidate.action_id(),
            candidate_name: candidate.profile_name().to_owned(),
            action_kind: candidate.action_kind().to_owned(),
            mode,
            safety_class: candidate.safety_class(),
            score_before: Some(baseline.clone()),
            score_after: None,
            rollback_performed: false,
            rollback_policy: "aborted-before-mutation".to_owned(),
            cooldown_until_unix_nanos: Some(retry_after),
            manual_restore_command: None,
            objective_evidence: None,
        }),
        clear_measurement_window: true,
    })
}
