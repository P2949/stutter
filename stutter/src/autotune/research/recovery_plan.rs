//! Pure decision for an interrupted attempt after a controller restart (PLAN P0). It only maps
//! independently gathered facts to a safe next step; it never replays an attempt, and anything
//! it cannot prove is `UnsafeToResume`. Gathering the facts (procfs, worker, tickets) stays with
//! the caller and its independent witnesses.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttemptState {
    /// The journal holds a sealed result for this attempt.
    Sealed,
    /// Started in the journal but never sealed.
    StartedUnsealed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryFacts {
    pub attempt: AttemptState,
    pub same_boot: bool,
    /// Stutter durably requested a Steam launch for this attempt.
    pub launch_requested: bool,
    /// At least one launch ticket was consumed (claimed) by a dispatcher.
    pub ticket_claimed: bool,
    /// An independent witness shows the requested Steam launch can no longer start anything.
    pub launch_request_settled: bool,
    /// An independent witness shows the claimed game tree and dispatchers are gone.
    pub process_tree_settled: bool,
    /// The worker reports no outstanding issued tokens.
    pub worker_tokens_zero: bool,
    /// The attempt's own transactions are physically restored or never started.
    pub transactions_restored: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryDecision {
    /// Nothing to reconcile; continue at the next sealed boundary.
    ResumeAtBoundary,
    /// Retire the unused tickets and seal the attempt as failed with this code; never replay.
    RetireAndSealFailed(&'static str),
    UnsafeToResume(&'static str),
}

pub fn decide(f: &RecoveryFacts) -> RecoveryDecision {
    use RecoveryDecision::{ResumeAtBoundary, RetireAndSealFailed, UnsafeToResume};
    if f.attempt == AttemptState::Sealed {
        return ResumeAtBoundary;
    }
    if !f.same_boot {
        // In-flight timing, tickets and process identity are not comparable across a reboot.
        return UnsafeToResume("boot changed with an unsealed attempt");
    }
    if !f.worker_tokens_zero || !f.transactions_restored {
        return UnsafeToResume("privileged state is not provably settled");
    }
    if f.ticket_claimed {
        return if f.process_tree_settled {
            RetireAndSealFailed("InterruptedAfterDispatch")
        } else {
            UnsafeToResume("a claimed launch has no settlement proof")
        };
    }
    if f.launch_requested && !f.launch_request_settled {
        // Steam may still start the invocation: not provably unarmed.
        return UnsafeToResume("a requested launch is not settled");
    }
    RetireAndSealFailed("InterruptedBeforeDispatch")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled() -> RecoveryFacts {
        RecoveryFacts {
            attempt: AttemptState::StartedUnsealed,
            same_boot: true,
            launch_requested: true,
            ticket_claimed: true,
            launch_request_settled: true,
            process_tree_settled: true,
            worker_tokens_zero: true,
            transactions_restored: true,
        }
    }

    #[test]
    fn only_provably_settled_attempts_are_sealed_and_never_replayed() {
        assert_eq!(
            decide(&settled()),
            RecoveryDecision::RetireAndSealFailed("InterruptedAfterDispatch")
        );
        let mut f = settled();
        f.attempt = AttemptState::Sealed;
        f.process_tree_settled = false;
        assert_eq!(decide(&f), RecoveryDecision::ResumeAtBoundary);
    }

    #[test]
    fn unprovable_states_stay_unsafe() {
        let cases: [fn(&mut RecoveryFacts); 5] = [
            |f| f.same_boot = false,
            |f| f.worker_tokens_zero = false,
            |f| f.transactions_restored = false,
            |f| f.process_tree_settled = false,
            |f| {
                f.ticket_claimed = false;
                f.launch_request_settled = false;
            },
        ];
        for mutate in cases {
            let mut f = settled();
            mutate(&mut f);
            assert!(matches!(decide(&f), RecoveryDecision::UnsafeToResume(_)));
        }
    }

    #[test]
    fn a_provably_unarmed_attempt_is_retired_without_a_process_witness() {
        let mut f = settled();
        f.ticket_claimed = false;
        f.process_tree_settled = false;
        assert_eq!(
            decide(&f),
            RecoveryDecision::RetireAndSealFailed("InterruptedBeforeDispatch")
        );
        f.launch_requested = false;
        f.launch_request_settled = false;
        assert_eq!(
            decide(&f),
            RecoveryDecision::RetireAndSealFailed("InterruptedBeforeDispatch")
        );
    }
}
