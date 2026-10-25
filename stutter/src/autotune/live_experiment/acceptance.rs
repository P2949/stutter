//! Acceptance-only executor: the real low-risk apply path, optionally decorated
//! to fail verification (and rollback) *after* real kernel mutation succeeded.
//! Only the owned disposable workload of the live recovery suite may use it.
use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Context;

use super::{executor::RuntimeLiveExperimentActionExecutor, *};
use crate::{
    actions::{ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction},
    autotune::{
        apply_low_risk::action_from_candidate,
        crash_acceptance::publish_acceptance_handshake,
        observation::{ActiveConfigSnapshot, AutotuneObservation},
        planning::candidate::CandidateAction,
    },
};

/// Handshake file names shared with the xtask harness.
pub(crate) const VERIFY_FAILURE_READY: &str = "verify-failure-ready.json";
pub(crate) const VERIFY_FAILURE_ACK: &str = "verify-failure-ack";

struct VerifyFailureAction<'a> {
    inner: Box<dyn TuningAction>,
    out: &'a Path,
    rollback_fails: bool,
}
impl TuningAction for VerifyFailureAction<'_> {
    fn id(&self) -> ActionId {
        self.inner.id()
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn safety_class(&self) -> SafetyClass {
        self.inner.safety_class()
    }
    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        self.inner.descriptor()
    }
    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.inner.preflight()
    }
    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.inner.dry_run()
    }
    fn apply(&self) -> crate::actions::ApplyResult {
        self.inner.apply()
    }
    fn prepared_before_mutation(&self) -> bool {
        self.inner.prepared_before_mutation()
    }
    fn apply_with_prepared_rollback(
        &self,
        callback: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> crate::actions::ApplyResult {
        self.inner.apply_with_prepared_rollback(callback)
    }
    fn verify(&self) -> anyhow::Result<ActionState> {
        let state = self.inner.verify()?;
        anyhow::ensure!(
            state.applied && state.checked_tasks == 5 && state.pending_changes == 0,
            "real kernel verification did not cover all five tasks"
        );
        // Let the harness independently observe the physical mutation first.
        publish_acceptance_handshake(
            self.out,
            VERIFY_FAILURE_READY,
            serde_json::json!({"real_apply_verified_tasks":state.checked_tasks,"rollback_fails":self.rollback_fails}),
        )?;
        let deadline = Instant::now() + Duration::from_secs(30);
        while !self.out.join(VERIFY_FAILURE_ACK).exists() {
            anyhow::ensure!(Instant::now() < deadline, "harness did not acknowledge");
            std::thread::sleep(Duration::from_millis(20));
        }
        anyhow::bail!("acceptance-injected verification failure after real apply")
    }
    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.rollback_fails,
            "acceptance-injected rollback failure; restore state retained"
        );
        self.inner.rollback(token)
    }
}

#[derive(Default)]
pub(crate) struct FaultInjectingExecutor {
    /// `Some(rollback_fails)` fails verification after the real apply.
    pub(crate) verify_failure: Option<bool>,
    pub(crate) apply_calls: usize,
}

impl LiveExperimentActionExecutor for FaultInjectingExecutor {
    fn apply_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        candidate: &CandidateAction,
        experiment_id: &str,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        self.apply_calls += 1;
        let journal_path = super::journal::controller_journal_path(input);
        let action: Box<dyn TuningAction> = if candidate.operation_kind()
            == crate::autotune::operation::MutationOperationKind::CpuAffinityProfile
        {
            let (_, mut action) = action_from_candidate(candidate.clone())?;
            action.restore_path = Some(super::journal::reserve_experiment_restore_path(
                &journal_path,
            )?);
            Box::new(action)
        } else {
            crate::actions::default_action_factory_registry()
                .build(candidate)
                .map_err(anyhow::Error::new)?
        };
        let out = journal_path
            .parent()
            .context("journal path has no directory")?
            .to_owned();
        let audit_path = input
            .audit_path
            .clone()
            .unwrap_or_else(|| out.join("actions.jsonl"));
        let source = crate::daemon_policy::ActionSource::AutotuneRuntime;
        let mut policy =
            crate::actions::runner::ActionRunPolicy::for_action(&action, false, source);
        if candidate.safety_class() > SafetyClass::ReversibleLowRisk {
            // The explicit medium-risk unlock the validation harness is granted.
            policy.policy = crate::daemon::DaemonPolicy::apply_medium_risk(source);
        }
        let journal_input = crate::autotune::action_transaction::JournaledActionInput {
            journal_path: &journal_path,
            audit_path: &audit_path,
            experiment_id,
            metadata: super::journal::controller_journal_metadata_for_candidate(
                input,
                candidate,
                observation,
                None,
                "pending_apply",
            ),
        };
        let outcome = match self.verify_failure {
            Some(rollback_fails) => crate::autotune::action_transaction::apply_journaled_action(
                &VerifyFailureAction {
                    inner: action,
                    out: &out,
                    rollback_fails,
                },
                policy,
                journal_input,
            )?,
            None => crate::autotune::action_transaction::apply_journaled_action(
                &action,
                policy,
                journal_input,
            )?,
        };
        outcome
            .rollback
            .context("journaled apply returned without rollback token")
    }

    fn rollback_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<Option<ActiveConfigSnapshot>> {
        RuntimeLiveExperimentActionExecutor.rollback_candidate(input, experiment, observation)
    }
}
