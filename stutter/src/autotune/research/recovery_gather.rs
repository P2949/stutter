//! Collects the independent observations `recovery_plan` decides on. Reads of durable records
//! that fail are errors (the caller treats them as unsafe); a witness that cannot answer yields
//! `None`, which the decision reads as unsettled. Nothing here derives authority from the
//! attempt's own claims.

use std::fs::File;

use super::{
    dispatch_context::launch_marker_present,
    launch_record::{read_launch_record, request_settled},
    launch_ticket::has_live_claim,
    launch_txn::UnitState,
    recovery_plan::{AttemptState, Observations},
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
    witnesses: Witnesses<'_>,
) -> anyhow::Result<Observations> {
    let record = read_launch_record(attempt_dir, uid)?;
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
}
