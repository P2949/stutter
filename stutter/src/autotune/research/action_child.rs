//! Narrow authorization for one research action child (PLAN section 7). Research normally suppresses
//! every autonomous mutation; this is the only door back open, and it is deliberately small: only
//! the supported game actions the campaign's intent already names, against the one frozen game
//! process, inside the campaign window, one at a time, each with kernel readback and a restoration
//! that must be proven before the next can start or the child can finish. It grants permission to
//! *ask* the existing Stutter controller and root worker; it never executes anything itself and is
//! no second optimizer.

use std::collections::BTreeSet;

use super::ResearchAction;

/// The frozen game process an action may target (a pid is only valid with its start time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChildTarget {
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionRequest {
    pub action: ResearchAction,
    pub target: ChildTarget,
    pub now_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The intent did not name this action.
    ActionNotAuthorized,
    /// Not the frozen game process (pid, start time or owner differ: a reused pid or a foreign task).
    WrongTarget,
    /// The authorization window is closed.
    Expired,
    /// A previous action is not yet restored (one live action at a time, no overlapping owners).
    ActionOutstanding,
    /// This action was already tried once in the child; it is never retried.
    AlreadyAttempted,
}

/// What the child decided about an applied action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Keep,
    Revert,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Permission given; the worker has not reported yet.
    Granted(ResearchAction),
    /// Applied and the kernel readback matched; the child has not decided.
    Applied(ResearchAction),
    /// Must be undone (reverted by decision, or the readback failed) and the undo proven.
    Reverting(ResearchAction),
    /// Kept; it stays outstanding until the campaign's restoration is proven.
    Kept(ResearchAction),
}

/// Permission for exactly one action; handed back to report what the worker did.
#[derive(Debug, PartialEq, Eq)]
pub struct ActionGrant {
    action: ResearchAction,
}

#[derive(Debug)]
pub struct ActionChildScope {
    actions: BTreeSet<ResearchAction>,
    target: ChildTarget,
    not_after_unix: u64,
    phase: Phase,
    attempted: BTreeSet<ResearchAction>,
    /// Actions whose restoration was proven, with how they ended.
    settled: Vec<(ResearchAction, Decision)>,
}

impl ActionChildScope {
    pub fn new(
        actions: impl IntoIterator<Item = ResearchAction>,
        target: ChildTarget,
        not_after_unix: u64,
    ) -> Self {
        Self {
            actions: actions.into_iter().collect(),
            target,
            not_after_unix,
            phase: Phase::Idle,
            attempted: BTreeSet::new(),
            settled: Vec::new(),
        }
    }

    /// Permission to ask the controller for one action. Every refusal leaves the scope unchanged.
    pub fn authorize(&mut self, request: ActionRequest) -> Result<ActionGrant, Refusal> {
        if !matches!(self.phase, Phase::Idle) {
            return Err(Refusal::ActionOutstanding);
        }
        if !self.actions.contains(&request.action) {
            return Err(Refusal::ActionNotAuthorized);
        }
        if request.target != self.target {
            return Err(Refusal::WrongTarget);
        }
        if request.now_unix > self.not_after_unix {
            return Err(Refusal::Expired);
        }
        if self.attempted.contains(&request.action) {
            return Err(Refusal::AlreadyAttempted);
        }
        self.attempted.insert(request.action);
        self.phase = Phase::Granted(request.action);
        Ok(ActionGrant {
            action: request.action,
        })
    }

    /// The worker refused or never applied the action: nothing changed, nothing to restore. The
    /// action still counts as attempted.
    pub fn not_applied(&mut self, grant: ActionGrant) -> bool {
        match self.phase {
            Phase::Granted(action) if action == grant.action => {
                self.phase = Phase::Idle;
                true
            }
            _ => false,
        }
    }

    /// The worker applied the action; `readback_ok` is whether the kernel readback matched. A
    /// failed readback can only be undone.
    pub fn applied(&mut self, grant: ActionGrant, readback_ok: bool) -> bool {
        match self.phase {
            Phase::Granted(action) if action == grant.action => {
                self.phase = if readback_ok {
                    Phase::Applied(action)
                } else {
                    Phase::Reverting(action)
                };
                true
            }
            _ => false,
        }
    }

    /// Keep or revert an applied (and read back) action.
    pub fn decide(&mut self, decision: Decision) -> bool {
        match (self.phase, decision) {
            (Phase::Applied(action), Decision::Keep) => {
                self.phase = Phase::Kept(action);
                true
            }
            (Phase::Applied(action), Decision::Revert) => {
                self.phase = Phase::Reverting(action);
                true
            }
            _ => false,
        }
    }

    /// Physical restoration was independently proven (`proven`). Only then does the scope free the
    /// slot; an unproven restoration leaves the action outstanding.
    pub fn restored(&mut self, proven: bool) -> bool {
        let (action, decision) = match self.phase {
            Phase::Reverting(action) => (action, Decision::Revert),
            Phase::Kept(action) => (action, Decision::Keep),
            _ => return false,
        };
        if !proven {
            return false;
        }
        self.settled.push((action, decision));
        self.phase = Phase::Idle;
        true
    }

    /// Nothing is applied or pending: every granted action was restored (or never applied).
    /// A fresh scope for the next attempt's game process: the same actions and window, nothing attempted
    /// yet. Only a scope with nothing outstanding can hand over.
    pub fn retargeted(&self, target: ChildTarget) -> Option<Self> {
        self.is_clean()
            .then(|| Self::new(self.actions.iter().copied(), target, self.not_after_unix))
    }

    pub fn is_clean(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    pub fn settled(&self) -> &[(ResearchAction, Decision)] {
        &self.settled
    }
}

#[cfg(test)]
#[path = "action_child_tests.rs"]
mod tests;
