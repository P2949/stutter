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

/// What the caller could independently observe. `None` means "could not be observed", which is
/// never read as settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observations {
    pub attempt: AttemptState,
    pub same_boot: bool,
    /// The durable launch record says a start request was sent (or a launch marker exists).
    pub launch_requested: bool,
    /// A dispatcher claim that was not a revocation exists for this attempt.
    pub ticket_claimed: bool,
    /// The durable record plus a fresh unit observation show the request can start nothing.
    pub launch_request_settled: Option<bool>,
    /// The independent process witness found no recorded dispatcher, game or foreign Steam.
    pub process_tree_settled: Option<bool>,
    /// The independent worker witness count, when available.
    pub outstanding_worker_tokens: Option<usize>,
    /// The attempt's own transactions are restored or never started.
    pub transactions_restored: Option<bool>,
}

impl Observations {
    /// Whether a launched attempt may be completed: the launch request, the process tree, the
    /// worker transactions and the restoration are all independently shown settled. Any
    /// missing observation, or a non-zero token count, is unsettled.
    pub fn settled_for_completion(&self) -> bool {
        self.launch_request_settled == Some(true)
            && self.process_tree_settled == Some(true)
            && self.outstanding_worker_tokens == Some(0)
            && self.transactions_restored == Some(true)
    }
}

/// Map observations to decision facts, treating every unobserved item as unsettled.
pub fn facts_from(o: &Observations) -> RecoveryFacts {
    RecoveryFacts {
        attempt: o.attempt,
        same_boot: o.same_boot,
        launch_requested: o.launch_requested,
        ticket_claimed: o.ticket_claimed,
        launch_request_settled: o.launch_request_settled == Some(true),
        process_tree_settled: o.process_tree_settled == Some(true),
        worker_tokens_zero: o.outstanding_worker_tokens == Some(0),
        transactions_restored: o.transactions_restored == Some(true),
    }
}

pub fn decide(f: &RecoveryFacts) -> RecoveryDecision {
    use RecoveryDecision::{ResumeAtBoundary, RetireAndSealFailed, UnsafeToResume};
    // A claimed ticket implies a durable request; anything else is contradictory evidence.
    if f.ticket_claimed && !f.launch_requested {
        return UnsafeToResume("a ticket was claimed without a recorded launch request");
    }
    if !f.worker_tokens_zero || !f.transactions_restored {
        return UnsafeToResume("privileged state is not provably settled");
    }
    if f.attempt == AttemptState::Sealed {
        // A sealed record is not proof that nothing is still pending or running.
        // Any requested launch needs independent process settlement, claimed ticket or not: a
        // launched Steam may hand off to the game without producing a dispatcher claim.
        return if f.same_boot
            && (!f.launch_requested || (f.launch_request_settled && f.process_tree_settled))
        {
            ResumeAtBoundary
        } else {
            UnsafeToResume("a sealed attempt still has unsettled launch or process state")
        };
    }
    if !f.same_boot {
        // In-flight timing, tickets and process identity are not comparable across a reboot.
        return UnsafeToResume("boot changed with an unsealed attempt");
    }
    if f.ticket_claimed {
        return if f.process_tree_settled && f.launch_request_settled {
            RetireAndSealFailed("InterruptedAfterDispatch")
        } else {
            UnsafeToResume("a claimed launch has no settlement proof")
        };
    }
    if f.launch_requested && !f.process_tree_settled {
        // The launcher hands off to Steam and may exit at once, so a finished launch unit does
        // not show that Steam will not still dispatch or that nothing was started.
        return UnsafeToResume("a requested launch has no process settlement proof");
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
        assert_eq!(decide(&f), RecoveryDecision::ResumeAtBoundary);
    }

    #[test]
    fn a_sealed_attempt_still_needs_independent_settlement_and_consistent_facts() {
        let sealed = || {
            let mut f = settled();
            f.attempt = AttemptState::Sealed;
            f
        };
        let cases: [fn(&mut RecoveryFacts); 6] = [
            |f| f.process_tree_settled = false,
            |f| f.launch_request_settled = false,
            |f| f.worker_tokens_zero = false,
            |f| f.transactions_restored = false,
            |f| f.same_boot = false,
            |f| f.launch_requested = false,
        ];
        for mutate in cases {
            let mut f = sealed();
            mutate(&mut f);
            assert!(matches!(decide(&f), RecoveryDecision::UnsafeToResume(_)));
        }
    }

    fn observed() -> Observations {
        Observations {
            attempt: AttemptState::StartedUnsealed,
            same_boot: true,
            launch_requested: true,
            ticket_claimed: true,
            launch_request_settled: Some(true),
            process_tree_settled: Some(true),
            outstanding_worker_tokens: Some(0),
            transactions_restored: Some(true),
        }
    }

    #[test]
    fn complete_observations_decide_and_any_unobserved_item_stays_unsafe() {
        assert_eq!(
            decide(&facts_from(&observed())),
            RecoveryDecision::RetireAndSealFailed("InterruptedAfterDispatch")
        );
        let unknowns: [fn(&mut Observations); 4] = [
            |o| o.launch_request_settled = None,
            |o| o.process_tree_settled = None,
            |o| o.outstanding_worker_tokens = None,
            |o| o.transactions_restored = None,
        ];
        for unknown in unknowns {
            let mut o = observed();
            unknown(&mut o);
            assert!(matches!(
                decide(&facts_from(&o)),
                RecoveryDecision::UnsafeToResume(_)
            ));
        }
        // A non-zero worker count is unsettled, not merely unknown.
        let mut o = observed();
        o.outstanding_worker_tokens = Some(1);
        assert!(matches!(
            decide(&facts_from(&o)),
            RecoveryDecision::UnsafeToResume(_)
        ));
    }

    #[test]
    fn completion_needs_every_witness_and_zero_tokens() {
        assert!(observed().settled_for_completion());
        let cases: [fn(&mut Observations); 5] = [
            |o| o.launch_request_settled = None,
            |o| o.process_tree_settled = Some(false),
            |o| o.outstanding_worker_tokens = None,
            |o| o.outstanding_worker_tokens = Some(1),
            |o| o.transactions_restored = Some(false),
        ];
        for mutate in cases {
            let mut o = observed();
            mutate(&mut o);
            assert!(!o.settled_for_completion());
        }
    }

    #[test]
    fn a_sealed_requested_launch_without_a_claim_still_needs_the_process_witness() {
        let facts = RecoveryFacts {
            attempt: AttemptState::Sealed,
            same_boot: true,
            launch_requested: true,
            ticket_claimed: false,
            launch_request_settled: true,
            process_tree_settled: false,
            worker_tokens_zero: true,
            transactions_restored: true,
        };
        assert!(matches!(
            decide(&facts),
            RecoveryDecision::UnsafeToResume(_)
        ));
        let mut settled = facts;
        settled.process_tree_settled = true;
        assert_eq!(decide(&settled), RecoveryDecision::ResumeAtBoundary);
        // A sealed attempt that never requested a launch has nothing to settle.
        let mut untouched = facts;
        untouched.launch_requested = false;
        untouched.launch_request_settled = false;
        assert_eq!(decide(&untouched), RecoveryDecision::ResumeAtBoundary);
    }

    #[test]
    fn a_claimed_ticket_cannot_retire_before_the_launch_request_is_settled() {
        let mut f = settled();
        f.launch_request_settled = false;
        assert!(matches!(decide(&f), RecoveryDecision::UnsafeToResume(_)));
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
    fn only_an_attempt_that_never_requested_a_launch_retires_without_a_process_witness() {
        let mut f = settled();
        f.ticket_claimed = false;
        f.process_tree_settled = false;
        // A requested launch needs the process witness even with no claim and a finished unit.
        assert!(matches!(decide(&f), RecoveryDecision::UnsafeToResume(_)));
        f.launch_requested = false;
        f.launch_request_settled = false;
        assert_eq!(
            decide(&f),
            RecoveryDecision::RetireAndSealFailed("InterruptedBeforeDispatch")
        );
    }
}
