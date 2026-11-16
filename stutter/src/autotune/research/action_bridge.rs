//! Sequencing of one research action child through the existing worker boundary (PLAN section 7).
//! `ActionChildScope` decides whether a request may be made; this module performs the sequence
//! against a worker interface: authorize, ask the worker to apply, take its kernel readback, let
//! the child decide keep or revert, restore, and accept the restoration only when the worker both
//! confirms the original state and holds no outstanding token. The worker interface is the existing
//! controller and root-worker pair (nothing new is added to the privileged surface); here it is a
//! trait so every failure path is tested with a fake.

use super::{
    ResearchAction,
    action_child::{ActionChildScope, ActionRequest, ChildTarget, Decision, Refusal},
};

/// What the worker reports after applying an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerApply {
    /// Applied; `readback_ok` is whether the kernel readback matched the intended state.
    Applied { readback_ok: bool },
    /// The worker refused (policy, ownership, stale target). Nothing was changed.
    Refused,
    /// The outcome is unknown (a lost reply). The action may have been applied.
    Ambiguous,
}

pub trait ActionWorker {
    fn apply(&mut self, action: ResearchAction, target: ChildTarget) -> WorkerApply;
    /// Undo the action; `true` only when the kernel readback shows the original state again.
    fn restore(&mut self, action: ResearchAction, target: ChildTarget) -> bool;
    /// Worker transactions still outstanding (zero when everything it applied is restored).
    /// `None` when the worker cannot say, which never counts as zero.
    fn outstanding_tokens(&mut self) -> Option<u32>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    /// The scope refused the request; the worker was never asked.
    Refused(Refusal),
    /// The worker refused; nothing was applied or needs restoring.
    WorkerRefused,
    /// Applied, then undone (by decision, or forced by a failed or unknown readback) and proven.
    Reverted { forced: bool },
    /// Applied and kept; it stays outstanding until `settle_kept`.
    Kept,
    /// The restoration could not be proven: the action stays outstanding and nothing else may run.
    RestoreUnproven,
}

fn prove_restored(
    scope: &mut ActionChildScope,
    worker: &mut dyn ActionWorker,
    action: ResearchAction,
    target: ChildTarget,
) -> bool {
    let kernel_ok = worker.restore(action, target);
    // The worker's own confirmation is not enough: it must also hold no token for what it applied.
    let proven = kernel_ok && worker.outstanding_tokens() == Some(0);
    scope.restored(proven)
}

/// Run one action: the child's `decide` is called only for an applied, read-back action.
pub fn run_action(
    scope: &mut ActionChildScope,
    worker: &mut dyn ActionWorker,
    request: ActionRequest,
    decide: &mut dyn FnMut() -> Decision,
) -> ActionOutcome {
    let grant = match scope.authorize(request) {
        Ok(grant) => grant,
        Err(refusal) => return ActionOutcome::Refused(refusal),
    };
    let (action, target) = (request.action, request.target);
    match worker.apply(action, target) {
        WorkerApply::Refused => {
            scope.not_applied(grant);
            ActionOutcome::WorkerRefused
        }
        WorkerApply::Applied { readback_ok: true } => {
            scope.applied(grant, true);
            match decide() {
                Decision::Keep => {
                    scope.decide(Decision::Keep);
                    ActionOutcome::Kept
                }
                Decision::Revert => {
                    scope.decide(Decision::Revert);
                    if prove_restored(scope, worker, action, target) {
                        ActionOutcome::Reverted { forced: false }
                    } else {
                        ActionOutcome::RestoreUnproven
                    }
                }
            }
        }
        // A failed readback, or a reply that never came, may have changed the system: it can only
        // be undone, and the undo must be proven.
        WorkerApply::Applied { readback_ok: false } | WorkerApply::Ambiguous => {
            scope.applied(grant, false);
            if prove_restored(scope, worker, action, target) {
                ActionOutcome::Reverted { forced: true }
            } else {
                ActionOutcome::RestoreUnproven
            }
        }
    }
}

/// End of the child: restore a kept action. Returns whether the scope is clean afterwards.
pub fn settle_kept(
    scope: &mut ActionChildScope,
    worker: &mut dyn ActionWorker,
    action: ResearchAction,
    target: ChildTarget,
) -> bool {
    if scope.is_clean() {
        return worker.outstanding_tokens() == Some(0);
    }
    prove_restored(scope, worker, action, target) && scope.is_clean()
}

#[cfg(test)]
#[path = "action_bridge_tests.rs"]
mod tests;
