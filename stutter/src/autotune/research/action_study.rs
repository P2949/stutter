//! G5 outcome classification for the real CP-LONG action study. The action itself is only ever
//! taken by the normal controller and root worker; this module judges the recorded facts and
//! refuses to count external comparators or an unverified transaction as a Stutter action.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionStudyFacts {
    pub controller_decided: bool,
    pub passed_autonomous_gate: bool,
    pub worker_transaction_committed: bool,
    pub kernel_readback_matches: bool,
    pub affected_tids: u64,
    pub baseline_runnable_events: u64,
    pub candidate_runnable_events: u64,
    pub guardrails_held: bool,
    pub kept: bool,
    pub restored_or_released: bool,
    pub outstanding_root_tokens: u64,
    /// The comparison used a taskset/systemd-run style external action.
    pub external_actor: bool,
    pub pressure_present: bool,
    pub physical_blocker: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionOutcome {
    /// Applied, physically verified, and kept or reverted per the objective.
    QualifiedApplied { kept: bool },
    /// No pressure and no action: the correct result for a clean scene.
    CorrectNoOp,
    /// A source-qualified denial under a documented physical blocker.
    DeniedUnderBlocker(&'static str),
    /// Evidence cannot support any claim; the reason is typed.
    NotQualified(&'static str),
}

pub fn classify(f: &ActionStudyFacts) -> ActionOutcome {
    use super::quality::MIN_RUNNABLE_EVENTS;
    if f.external_actor {
        return ActionOutcome::NotQualified("external actor is not a Stutter action");
    }
    if f.outstanding_root_tokens != 0 {
        return ActionOutcome::NotQualified("root tokens outstanding");
    }
    if !f.controller_decided || !f.worker_transaction_committed {
        if !f.pressure_present && !f.worker_transaction_committed {
            return ActionOutcome::CorrectNoOp;
        }
        return match f.physical_blocker {
            Some(blocker) => ActionOutcome::DeniedUnderBlocker(blocker),
            None => ActionOutcome::NotQualified("no committed transaction and no blocker"),
        };
    }
    if !f.passed_autonomous_gate {
        return ActionOutcome::NotQualified("transaction bypassed the autonomous gate");
    }
    if f.affected_tids == 0 || !f.kernel_readback_matches {
        return ActionOutcome::NotQualified("physical readback does not confirm the change");
    }
    if f.baseline_runnable_events < MIN_RUNNABLE_EVENTS
        || f.candidate_runnable_events < MIN_RUNNABLE_EVENTS
    {
        return ActionOutcome::NotQualified("below the runnable sample floor");
    }
    if !f.restored_or_released && !f.kept {
        return ActionOutcome::NotQualified("action neither kept nor restored");
    }
    // A keep requires the guardrails; a lower runnable P99 alone never justifies one.
    ActionOutcome::QualifiedApplied {
        kept: f.kept && f.guardrails_held,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied() -> ActionStudyFacts {
        ActionStudyFacts {
            controller_decided: true,
            passed_autonomous_gate: true,
            worker_transaction_committed: true,
            kernel_readback_matches: true,
            affected_tids: 12,
            baseline_runnable_events: 1_000,
            candidate_runnable_events: 1_000,
            guardrails_held: true,
            kept: true,
            restored_or_released: true,
            outstanding_root_tokens: 0,
            external_actor: false,
            pressure_present: true,
            physical_blocker: None,
        }
    }

    #[test]
    fn full_transaction_qualifies_and_guardrail_failure_never_keeps() {
        assert_eq!(
            classify(&applied()),
            ActionOutcome::QualifiedApplied { kept: true }
        );
        let mut f = applied();
        f.guardrails_held = false;
        assert_eq!(
            classify(&f),
            ActionOutcome::QualifiedApplied { kept: false }
        );
    }

    #[test]
    fn external_actor_tokens_and_zero_tids_are_refused() {
        let mut f = applied();
        f.external_actor = true;
        assert!(matches!(classify(&f), ActionOutcome::NotQualified(_)));
        let mut f = applied();
        f.outstanding_root_tokens = 1;
        assert!(matches!(classify(&f), ActionOutcome::NotQualified(_)));
        let mut f = applied();
        f.affected_tids = 0;
        assert!(matches!(classify(&f), ActionOutcome::NotQualified(_)));
        let mut f = applied();
        f.passed_autonomous_gate = false;
        assert!(matches!(classify(&f), ActionOutcome::NotQualified(_)));
        let mut f = applied();
        f.candidate_runnable_events = 999;
        assert!(matches!(classify(&f), ActionOutcome::NotQualified(_)));
    }

    #[test]
    fn clean_scene_noop_and_blocker_denial_are_valid_outcomes() {
        let mut f = applied();
        f.controller_decided = false;
        f.worker_transaction_committed = false;
        f.pressure_present = false;
        assert_eq!(classify(&f), ActionOutcome::CorrectNoOp);
        f.pressure_present = true;
        assert!(matches!(classify(&f), ActionOutcome::NotQualified(_)));
        f.physical_blocker = Some("no configured game cgroup");
        assert_eq!(
            classify(&f),
            ActionOutcome::DeniedUnderBlocker("no configured game cgroup")
        );
    }
}
