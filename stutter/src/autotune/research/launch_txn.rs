//! Generic bounded launch transaction over a user-manager port (PLAN P1). The phase is persisted
//! before the request is sent; a lost acknowledgement is reconciled by observing the unique unit,
//! never by resending. Anything not provably started or provably never going to start is
//! `Ambiguous`, which the caller must treat as `UnsafeToResume`.

use super::user_launcher::UserLaunchRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitState {
    /// The unique unit exists and is running or starting.
    Active,
    /// The unit exists and has finished or failed; it will not start again.
    Finished,
    /// No such unit is currently known.
    Absent,
}

#[derive(Debug)]
pub enum SendError {
    /// The manager rejected the request; nothing was started.
    Rejected,
    /// The request may or may not have been delivered or acted on.
    AckLost,
}

pub trait UserManagerPort {
    fn send_start(&mut self, request: &UserLaunchRequest) -> Result<(), SendError>;
    fn unit_state(&mut self, unit_name: &str) -> anyhow::Result<UnitState>;
    /// Authoritative, durable manager-side proof that the request was rejected or can no longer
    /// execute (for example the job queue is empty and the request identity is fenced). A
    /// snapshot of "no such unit" is never this proof.
    fn proves_never_executable(&mut self, unit_name: &str) -> anyhow::Result<bool>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchPhase {
    Prepared,
    Sent,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LaunchOutcome {
    Running,
    /// Provably nothing was or will be started for this unit.
    NeverStarted,
    /// The unit ran and is gone; the caller must still settle the game tree independently.
    FinishedAlready,
    Ambiguous(&'static str),
}

/// Absent snapshots taken after a lost acknowledgement. They only look for a late-appearing unit;
/// they never prove nothing will start, so they cannot yield `NeverStarted` on their own.
pub const ABSENT_CONFIRMATIONS: u32 = 3;

pub fn run(
    port: &mut dyn UserManagerPort,
    request: &UserLaunchRequest,
    persist: &mut dyn FnMut(LaunchPhase) -> anyhow::Result<()>,
) -> anyhow::Result<LaunchOutcome> {
    // Durable before the first byte can leave: a crash after this must be recoverable as Sent.
    persist(LaunchPhase::Prepared)?;
    persist(LaunchPhase::Sent)?;
    match port.send_start(request) {
        Ok(()) => reconcile(port, &request.unit_name, false),
        Err(SendError::Rejected) => Ok(LaunchOutcome::NeverStarted),
        Err(SendError::AckLost) => reconcile(port, &request.unit_name, true),
    }
}

/// Observe the unique unit; an acknowledged start must be visible, a lost one may legitimately
/// be absent but only stays "never started" after repeated absence.
fn reconcile(
    port: &mut dyn UserManagerPort,
    unit: &str,
    ack_lost: bool,
) -> anyhow::Result<LaunchOutcome> {
    let mut absent = 0;
    for _ in 0..ABSENT_CONFIRMATIONS {
        match port.unit_state(unit)? {
            UnitState::Active => return Ok(LaunchOutcome::Running),
            UnitState::Finished => return Ok(LaunchOutcome::FinishedAlready),
            UnitState::Absent => absent += 1,
        }
    }
    if !ack_lost {
        return Ok(LaunchOutcome::Ambiguous(
            "an acknowledged start left no unit",
        ));
    }
    debug_assert_eq!(absent, ABSENT_CONFIRMATIONS);
    // A delayed job may still start, and a short-lived transient unit may already have been
    // collected: absence is only `NeverStarted` with durable manager-side proof.
    Ok(if port.proves_never_executable(unit)? {
        LaunchOutcome::NeverStarted
    } else {
        LaunchOutcome::Ambiguous("lost acknowledgement and no proof the request cannot execute")
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct Fake {
        send: Option<Result<(), SendError>>,
        states: VecDeque<UnitState>,
        sends: u32,
        proof: bool,
    }

    impl UserManagerPort for Fake {
        fn send_start(&mut self, _: &UserLaunchRequest) -> Result<(), SendError> {
            self.sends += 1;
            self.send.take().unwrap_or(Err(SendError::AckLost))
        }
        fn unit_state(&mut self, _: &str) -> anyhow::Result<UnitState> {
            Ok(self.states.pop_front().unwrap_or(UnitState::Absent))
        }
        fn proves_never_executable(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(self.proof)
        }
    }

    fn request() -> UserLaunchRequest {
        UserLaunchRequest {
            unit_name: "stutter-research-1.service".into(),
            body: Vec::new(),
        }
    }

    fn go(
        send: Result<(), SendError>,
        states: &[UnitState],
    ) -> (LaunchOutcome, Vec<LaunchPhase>, u32) {
        let mut port = Fake {
            send: Some(send),
            states: states.iter().copied().collect(),
            sends: 0,
            proof: false,
        };
        let mut phases = Vec::new();
        let outcome = run(&mut port, &request(), &mut |p| {
            phases.push(p);
            Ok(())
        })
        .unwrap();
        (outcome, phases, port.sends)
    }

    #[test]
    fn the_phase_is_persisted_before_sending_and_a_request_is_sent_once() {
        let (outcome, phases, sends) = go(Ok(()), &[UnitState::Active]);
        assert_eq!(outcome, LaunchOutcome::Running);
        assert_eq!(phases, [LaunchPhase::Prepared, LaunchPhase::Sent]);
        assert_eq!(sends, 1);
    }

    #[test]
    fn a_lost_ack_is_reconciled_by_observation_never_by_resending() {
        let (outcome, _, sends) = go(
            Err(SendError::AckLost),
            &[UnitState::Absent, UnitState::Active],
        );
        assert_eq!(outcome, LaunchOutcome::Running);
        assert_eq!(sends, 1);
        let (outcome, ..) = go(Err(SendError::AckLost), &[UnitState::Finished]);
        assert_eq!(outcome, LaunchOutcome::FinishedAlready);
    }

    #[test]
    fn repeated_absence_after_a_lost_ack_is_ambiguous_without_manager_proof() {
        // A delayed start, or a short-lived unit already collected, leaves absence proving nothing.
        let (outcome, ..) = go(Err(SendError::AckLost), &[]);
        assert!(matches!(outcome, LaunchOutcome::Ambiguous(_)));
        // An acknowledged start that leaves no unit is contradictory.
        let (outcome, ..) = go(Ok(()), &[]);
        assert!(matches!(outcome, LaunchOutcome::Ambiguous(_)));
    }

    #[test]
    fn a_late_start_after_absent_snapshots_is_still_observed() {
        let (outcome, _, sends) = go(
            Err(SendError::AckLost),
            &[UnitState::Absent, UnitState::Absent, UnitState::Active],
        );
        assert_eq!(outcome, LaunchOutcome::Running);
        assert_eq!(sends, 1);
    }

    #[test]
    fn only_durable_manager_proof_or_a_rejection_means_never_started() {
        let mut port = Fake {
            send: Some(Err(SendError::AckLost)),
            states: VecDeque::new(),
            sends: 0,
            proof: true,
        };
        let outcome = run(&mut port, &request(), &mut |_| Ok(())).unwrap();
        assert_eq!(outcome, LaunchOutcome::NeverStarted);
        assert_eq!(port.sends, 1);
        let (outcome, ..) = go(Err(SendError::Rejected), &[]);
        assert_eq!(outcome, LaunchOutcome::NeverStarted);
    }

    #[test]
    fn a_failed_persist_prevents_any_send() {
        let mut port = Fake {
            send: Some(Ok(())),
            states: VecDeque::new(),
            sends: 0,
            proof: false,
        };
        let result = run(&mut port, &request(), &mut |_| {
            Err(anyhow::anyhow!("disk full"))
        });
        assert!(result.is_err());
        assert_eq!(port.sends, 0);
    }
}
