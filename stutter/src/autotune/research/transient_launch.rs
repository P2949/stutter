//! The one call the driver makes to start an attempt through the user manager: run the launch
//! transaction with every phase made durable before the request can leave, then record the
//! outcome. It never retries and never interprets ambiguity as "not started".

use std::fs::File;

use super::{
    launch_record::{record_outcome, record_phase},
    launch_txn::{LaunchOutcome, UserManagerPort, run},
    user_launcher::UserLaunchRequest,
};

/// What the driver must do next.
#[derive(Debug, PartialEq, Eq)]
pub enum LaunchDecision {
    /// The unit is running; continue to observe the attempt.
    Proceed,
    /// Nothing was or will be started: the attempt may be sealed as a refused launch.
    Refused,
    /// The unit already ran and ended; the detached game tree must still be settled.
    NeedsSettlement,
    /// The effect of the request is unknown: no seal, no retry, independent recovery only.
    NeedsRecovery(&'static str),
}

/// `attempt_dir` is the attempt's private directory. A failed durable write before sending
/// aborts without sending; a failed outcome write leaves the already-durable `Sent` record,
/// which recovery reads as an unsettled launch.
pub fn start_attempt(
    port: &mut dyn UserManagerPort,
    attempt_dir: &File,
    request: &UserLaunchRequest,
) -> anyhow::Result<LaunchDecision> {
    let unit = request.unit_name.clone();
    let outcome = run(port, request, &mut |phase| {
        record_phase(attempt_dir, phase, &unit)
    })?;
    record_outcome(attempt_dir, &unit, &outcome)?;
    Ok(match outcome {
        LaunchOutcome::Running => LaunchDecision::Proceed,
        LaunchOutcome::NeverStarted => LaunchDecision::Refused,
        LaunchOutcome::FinishedAlready => LaunchDecision::NeedsSettlement,
        LaunchOutcome::Ambiguous(reason) => LaunchDecision::NeedsRecovery(reason),
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use super::*;
    use crate::autotune::research::{
        launch_record::{RecordedOutcome, read_launch_record, request_settled},
        launch_txn::{SendError, UnitState},
    };

    const UNIT: &str = "stutter-research-0123456789abcdef-0000.service";

    struct Fake {
        send: Option<Result<(), SendError>>,
        states: VecDeque<UnitState>,
        sent_when: Rc<RefCell<Option<bool>>>,
        dir: File,
    }

    impl UserManagerPort for Fake {
        fn send_start(&mut self, _: &UserLaunchRequest) -> Result<(), SendError> {
            // The Sent record must already be durable at the moment the request leaves.
            let record = read_launch_record(&self.dir, crate::syscall::geteuid()).unwrap();
            *self.sent_when.borrow_mut() = Some(record.sent);
            self.send.take().unwrap_or(Err(SendError::AckLost))
        }
        fn unit_state(&mut self, _: &str) -> anyhow::Result<UnitState> {
            Ok(self.states.pop_front().unwrap_or(UnitState::Absent))
        }
        fn proves_never_executable(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    fn request() -> UserLaunchRequest {
        UserLaunchRequest {
            unit_name: UNIT.into(),
            body: Vec::new(),
        }
    }

    fn attempt(
        send: Result<(), SendError>,
        states: &[UnitState],
    ) -> (LaunchDecision, File, Option<bool>) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            tmp.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let dir = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        let seen = Rc::new(RefCell::new(None));
        let mut port = Fake {
            send: Some(send),
            states: states.iter().copied().collect(),
            sent_when: seen.clone(),
            dir: dir.try_clone().unwrap(),
        };
        let decision = start_attempt(&mut port, &dir, &request()).unwrap();
        // Keep the tempdir alive until the caller is done with the directory handle.
        std::mem::forget(tmp);
        let sent_when = *seen.borrow();
        (decision, dir, sent_when)
    }

    #[test]
    fn the_sent_record_is_durable_before_the_request_leaves() {
        let (decision, dir, sent_when) = attempt(Ok(()), &[UnitState::Active]);
        assert_eq!(decision, LaunchDecision::Proceed);
        assert_eq!(sent_when, Some(true));
        let record = read_launch_record(&dir, crate::syscall::geteuid()).unwrap();
        assert_eq!(record.outcome, Some(RecordedOutcome::Running));
        // A running unit is not settled.
        assert!(!request_settled(&record, Some(UnitState::Active)));
    }

    #[test]
    fn a_lost_ack_with_no_unit_needs_recovery_and_stays_unsettled() {
        let (decision, dir, _) = attempt(Err(SendError::AckLost), &[]);
        assert!(matches!(decision, LaunchDecision::NeedsRecovery(_)));
        let record = read_launch_record(&dir, crate::syscall::geteuid()).unwrap();
        assert_eq!(record.outcome, Some(RecordedOutcome::Ambiguous));
        assert!(!request_settled(&record, Some(UnitState::Absent)));
    }

    #[test]
    fn a_rejected_request_is_refused_and_settled() {
        let (decision, dir, _) = attempt(Err(SendError::Rejected), &[]);
        assert_eq!(decision, LaunchDecision::Refused);
        let record = read_launch_record(&dir, crate::syscall::geteuid()).unwrap();
        assert!(request_settled(&record, None));
    }

    #[test]
    fn a_finished_unit_still_needs_the_game_tree_settled() {
        let (decision, ..) = attempt(Err(SendError::AckLost), &[UnitState::Finished]);
        assert_eq!(decision, LaunchDecision::NeedsSettlement);
    }
}
