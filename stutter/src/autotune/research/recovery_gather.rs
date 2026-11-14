//! Collects the independent observations `recovery_plan` decides on. Reads of durable records
//! that fail are errors (the caller treats them as unsafe); a witness that cannot answer yields
//! `None`, which the decision reads as unsettled. Nothing here derives authority from the
//! attempt's own claims.

use std::fs::File;

use super::{
    dispatch_context::{DispatchContextV1, launch_marker_present},
    launch_record::{RecordedOutcome, read_launch_record, request_settled},
    launch_ticket::has_live_claim,
    launch_txn::UnitState,
    recovery_plan::{AttemptState, Observations},
    steam_port::SettlementWitness,
    user_launcher::launch_unit_name,
};

/// Independent witnesses, each queried at most once. They are closures so the production
/// procfs, user-manager and worker sources can be supplied without this module depending on them.
pub struct Witnesses<'a> {
    pub unit_state: &'a mut dyn FnMut(&str) -> anyhow::Result<UnitState>,
    pub process_tree_settled: &'a mut dyn FnMut() -> anyhow::Result<bool>,
    pub outstanding_worker_tokens: &'a mut dyn FnMut() -> anyhow::Result<usize>,
    pub transactions_restored: &'a mut dyn FnMut() -> anyhow::Result<bool>,
}

pub fn gather(
    attempt_dir: &File,
    uid: u32,
    attempt: AttemptState,
    same_boot: bool,
    expected_unit: &str,
    witnesses: Witnesses<'_>,
) -> anyhow::Result<Observations> {
    let record = read_launch_record(attempt_dir, uid)?;
    // The records must name exactly this attempt's launch unit, never another request's.
    anyhow::ensure!(
        record
            .unit_name
            .as_deref()
            .is_none_or(|unit| unit == expected_unit),
        "launch record names a different unit"
    );
    let marker = launch_marker_present(attempt_dir, uid)?;
    let ticket_claimed = has_live_claim(attempt_dir, uid)?;
    let launch_requested = record.sent || marker || ticket_claimed;
    let launch_request_settled = if marker && !record.sent {
        // A request marker without a durable transaction record means a launch path that left no
        // proof of what was sent (for example a direct child spawn): never assume it is settled.
        None
    } else {
        let current = match record.unit_name.as_deref() {
            Some(unit) if record.sent => (witnesses.unit_state)(unit).ok(),
            _ => None,
        };
        Some(request_settled(&record, current))
    };
    Ok(Observations {
        attempt,
        same_boot,
        launch_requested,
        ticket_claimed,
        launch_request_settled,
        process_tree_settled: (witnesses.process_tree_settled)().ok(),
        outstanding_worker_tokens: (witnesses.outstanding_worker_tokens)().ok(),
        transactions_restored: (witnesses.transactions_restored)().ok(),
    })
}

type UnitStateWitness = Box<dyn FnMut(&str) -> anyhow::Result<UnitState> + Send>;
type TreeWitness = Box<dyn FnMut(&File, &DispatchContextV1) -> anyhow::Result<bool> + Send>;
type TokenWitness = Box<dyn FnMut() -> anyhow::Result<usize> + Send>;
type RestoredWitness = Box<dyn FnMut() -> anyhow::Result<bool> + Send>;

/// The completion witness the Steam port consults: the same durable records and independent
/// witnesses recovery uses, so a finished attempt and a recovered one settle by one rule.
pub struct CompletionWitness {
    pub unit_state: UnitStateWitness,
    pub process_tree_settled: TreeWitness,
    pub outstanding_worker_tokens: TokenWitness,
    pub transactions_restored: RestoredWitness,
}

impl SettlementWitness for CompletionWitness {
    fn settled(&mut self, dispatch: &File, context: &DispatchContextV1) -> anyhow::Result<bool> {
        let unit = launch_unit_name(&context.intent.digest()?, context.slot.index)?;
        // Completion is for an attempt that was actually launched. Recovery may conclude that an
        // untouched attempt launched nothing, but an absent or unreconciled transaction here is
        // never completion: it needs a valid sent request with a reconciled outcome.
        let record = read_launch_record(dispatch, context.intent.uid)?;
        if !record.sent
            || !matches!(
                record.outcome,
                Some(RecordedOutcome::Running | RecordedOutcome::FinishedAlready)
            )
        {
            return Ok(false);
        }
        let Self {
            unit_state,
            process_tree_settled,
            outstanding_worker_tokens,
            transactions_restored,
        } = self;
        let observations = gather(
            dispatch,
            context.intent.uid,
            AttemptState::StartedUnsealed,
            true,
            &unit,
            Witnesses {
                unit_state: &mut **unit_state,
                process_tree_settled: &mut || process_tree_settled(dispatch, context),
                outstanding_worker_tokens: &mut **outstanding_worker_tokens,
                transactions_restored: &mut **transactions_restored,
            },
        )?;
        Ok(observations.settled_for_completion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::research::{
        launch_record::{record_outcome, record_phase},
        launch_txn::{LaunchOutcome, LaunchPhase},
        recovery_plan::{RecoveryDecision, decide, facts_from},
    };

    const UNIT: &str = "stutter-research-0123456789abcdef-0000.service";

    fn fresh() -> (tempfile::TempDir, File) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            tmp.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let file = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        (tmp, file)
    }

    fn uid() -> u32 {
        crate::syscall::geteuid()
    }

    fn unavailable<T>() -> anyhow::Result<T> {
        Err(anyhow::anyhow!("witness unavailable"))
    }

    fn run(
        dir: &File,
        unit: UnitState,
        process: Option<bool>,
        tokens: Option<usize>,
        restored: Option<bool>,
    ) -> Observations {
        gather(
            dir,
            uid(),
            AttemptState::StartedUnsealed,
            true,
            UNIT,
            Witnesses {
                unit_state: &mut |_| Ok(unit),
                process_tree_settled: &mut || process.map_or_else(unavailable, Ok),
                outstanding_worker_tokens: &mut || tokens.map_or_else(unavailable, Ok),
                transactions_restored: &mut || restored.map_or_else(unavailable, Ok),
            },
        )
        .unwrap()
    }

    #[test]
    fn an_untouched_attempt_retires_as_never_dispatched() {
        let (_tmp, dir) = fresh();
        let o = run(&dir, UnitState::Absent, Some(true), Some(0), Some(true));
        assert!(!o.launch_requested && !o.ticket_claimed);
        assert_eq!(
            decide(&facts_from(&o)),
            RecoveryDecision::RetireAndSealFailed("InterruptedBeforeDispatch")
        );
    }

    #[test]
    fn a_sent_ambiguous_launch_with_no_unit_is_unsafe() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir, LaunchPhase::Sent, UNIT).unwrap();
        record_outcome(&dir, UNIT, &LaunchOutcome::Ambiguous("lost")).unwrap();
        let o = run(&dir, UnitState::Absent, Some(true), Some(0), Some(true));
        assert!(o.launch_requested);
        assert_eq!(o.launch_request_settled, Some(false));
        assert!(matches!(
            decide(&facts_from(&o)),
            RecoveryDecision::UnsafeToResume(_)
        ));
    }

    #[test]
    fn a_terminal_unit_with_a_settled_tree_and_worker_retires_without_replay() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir, LaunchPhase::Sent, UNIT).unwrap();
        record_outcome(&dir, UNIT, &LaunchOutcome::Running).unwrap();
        let o = run(&dir, UnitState::Finished, Some(true), Some(0), Some(true));
        assert_eq!(o.launch_request_settled, Some(true));
        assert_eq!(
            decide(&facts_from(&o)),
            RecoveryDecision::RetireAndSealFailed("InterruptedBeforeDispatch")
        );
    }

    #[test]
    fn a_marker_without_a_transaction_record_is_never_settled() {
        let (_tmp, dir) = fresh();
        crate::autotune::research::dispatch_context::mark_launch_requested(&dir).unwrap();
        let o = run(&dir, UnitState::Finished, Some(true), Some(0), Some(true));
        assert!(o.launch_requested);
        assert_eq!(o.launch_request_settled, None);
        assert!(matches!(
            decide(&facts_from(&o)),
            RecoveryDecision::UnsafeToResume(_)
        ));
    }

    #[test]
    fn a_record_naming_another_requests_unit_is_refused() {
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, "other-request.service").unwrap();
        let result = gather(
            &dir,
            uid(),
            AttemptState::StartedUnsealed,
            true,
            UNIT,
            Witnesses {
                unit_state: &mut |_| Ok(UnitState::Finished),
                process_tree_settled: &mut || Ok(true),
                outstanding_worker_tokens: &mut || Ok(0),
                transactions_restored: &mut || Ok(true),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn an_unavailable_witness_or_live_claim_never_settles() {
        let (_tmp, dir) = fresh();
        for o in [
            run(&dir, UnitState::Absent, Some(true), None, Some(true)),
            run(&dir, UnitState::Absent, Some(true), Some(0), None),
            run(&dir, UnitState::Absent, Some(true), Some(2), Some(true)),
        ] {
            assert!(matches!(
                decide(&facts_from(&o)),
                RecoveryDecision::UnsafeToResume(_)
            ));
        }
        // Once a launch was sent, an unavailable process witness is unsettled.
        let (_tmp3, dir3) = fresh();
        record_phase(&dir3, LaunchPhase::Prepared, UNIT).unwrap();
        record_phase(&dir3, LaunchPhase::Sent, UNIT).unwrap();
        let o = run(&dir3, UnitState::Finished, None, Some(0), Some(true));
        assert!(matches!(
            decide(&facts_from(&o)),
            RecoveryDecision::UnsafeToResume(_)
        ));
        // A dispatcher claim that is not a revocation is a live claim.
        crate::autotune::research::report_store::write_new(
            &dir,
            &format!("used-{}", "a".repeat(64)),
            b"deadbeef",
        )
        .unwrap();
        let o = run(&dir, UnitState::Absent, Some(true), Some(0), Some(true));
        assert!(o.ticket_claimed && o.launch_requested);
        // A revoked claim is not.
        let (_tmp2, dir2) = fresh();
        crate::autotune::research::report_store::write_new(
            &dir2,
            &format!("used-{}", "b".repeat(64)),
            b"revoked-v1",
        )
        .unwrap();
        assert!(!run(&dir2, UnitState::Absent, Some(true), Some(0), Some(true)).ticket_claimed);
    }

    fn completion_witness(
        unit: UnitState,
        process: bool,
        tokens: usize,
        restored: bool,
    ) -> CompletionWitness {
        CompletionWitness {
            unit_state: Box::new(move |_| Ok(unit)),
            process_tree_settled: Box::new(move |_, _| Ok(process)),
            outstanding_worker_tokens: Box::new(move || Ok(tokens)),
            transactions_restored: Box::new(move || Ok(restored)),
        }
    }

    fn context() -> DispatchContextV1 {
        use crate::autotune::research::{AttemptSlot, dispatch_context::ArmSpec, tests::intent};
        let intent = intent();
        let slot = AttemptSlot {
            study_sha256: intent.digest().unwrap(),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: intent.tools[0].source,
        };
        DispatchContextV1 {
            schema_version: 1,
            intent,
            slot,
            tickets: Default::default(),
            spec: ArmSpec {
                tool_dir: "/t".into(),
                study_compat_data: "/s".into(),
                arm_env: Default::default(),
                steamapps: "/a".into(),
                proton_manifest: "/m".into(),
            },
        }
    }

    #[test]
    fn completion_needs_a_real_sent_and_reconciled_transaction() {
        let context = context();
        let unit = launch_unit_name(&context.intent.digest().unwrap(), 0).unwrap();
        let everything_settled = || completion_witness(UnitState::Finished, true, 0, true);
        // An empty attempt directory is never completion, however settled the witnesses are.
        let (_empty, dir) = fresh();
        assert!(!everything_settled().settled(&dir, &context).unwrap());
        // Prepared only: nothing was sent.
        let (_prepared, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, &unit).unwrap();
        assert!(!everything_settled().settled(&dir, &context).unwrap());
        // Sent but never reconciled, or reconciled as ambiguous or never started.
        let (_sent, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, &unit).unwrap();
        record_phase(&dir, LaunchPhase::Sent, &unit).unwrap();
        assert!(!everything_settled().settled(&dir, &context).unwrap());
        for outcome in [
            LaunchOutcome::Ambiguous("lost"),
            LaunchOutcome::NeverStarted,
        ] {
            let (_t, dir) = fresh();
            record_phase(&dir, LaunchPhase::Prepared, &unit).unwrap();
            record_phase(&dir, LaunchPhase::Sent, &unit).unwrap();
            record_outcome(&dir, &unit, &outcome).unwrap();
            assert!(!everything_settled().settled(&dir, &context).unwrap());
        }
        // Another request's records and torn (out-of-order) records are errors, not completion.
        let (_other, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, "other.service").unwrap();
        record_phase(&dir, LaunchPhase::Sent, "other.service").unwrap();
        record_outcome(&dir, "other.service", &LaunchOutcome::Running).unwrap();
        assert!(everything_settled().settled(&dir, &context).is_err());
        let (_torn, dir) = fresh();
        record_phase(&dir, LaunchPhase::Sent, &unit).unwrap();
        assert!(everything_settled().settled(&dir, &context).is_err());
    }

    #[test]
    fn the_completion_witness_settles_only_when_every_independent_source_agrees() {
        let context = context();
        let unit = launch_unit_name(&context.intent.digest().unwrap(), 0).unwrap();
        let (_tmp, dir) = fresh();
        record_phase(&dir, LaunchPhase::Prepared, &unit).unwrap();
        record_phase(&dir, LaunchPhase::Sent, &unit).unwrap();
        record_outcome(&dir, &unit, &LaunchOutcome::Running).unwrap();
        let settled = |w: &mut CompletionWitness| w.settled(&dir, &context).unwrap();
        assert!(settled(&mut completion_witness(
            UnitState::Finished,
            true,
            0,
            true
        )));
        for mut witness in [
            completion_witness(UnitState::Active, true, 0, true),
            completion_witness(UnitState::Absent, true, 0, true),
            completion_witness(UnitState::Finished, false, 0, true),
            completion_witness(UnitState::Finished, true, 1, true),
            completion_witness(UnitState::Finished, true, 0, false),
        ] {
            assert!(!settled(&mut witness));
        }
        // A witness that cannot answer is unsettled, never settled.
        let mut broken = completion_witness(UnitState::Finished, true, 0, true);
        broken.outstanding_worker_tokens = Box::new(|| Err(anyhow::anyhow!("worker unavailable")));
        assert!(!settled(&mut broken));
    }
}
