//! The producer side of a CP-LONG session (PLAN section 7): the lifecycle a driver must follow, the
//! evidence record a completed session leaves, and the verifier that decides whether that record
//! stands for the work unit that was authorized. The native benchmark is a short fixed scene and its
//! FPS is never a long-session result; a long session is a route played inside the game for the
//! work unit's whole duration, with the controller observing, diagnosing, acting and settling.
//!
//! No physical driver exists: nothing in this repository can play a route in the game. The
//! `UnavailableDriver` says so explicitly, so a long study is blocked with a typed reason rather than
//! silently run on the benchmark. A real driver implements `LongSessionDriver`, and whatever it
//! produces is accepted only through `verify`, which does not trust the driver's own claims.

use super::long_session::LongWorkUnit;

/// Phases of a session must be accounted for to at least this share of its elapsed time, so a
/// session cannot claim its duration while most of it was unobserved.
const MIN_PHASE_COVERAGE_PERCENT: u64 = 90;

/// One observed phase of the route (for example loading, travel, combat), inside the session.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Phase {
    pub name: String,
    pub started_unix: u64,
    pub ended_unix: u64,
}

/// The game process the session ran in, as the controller attested it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionProcess {
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
}

/// What a completed session leaves.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LongSessionRecord {
    pub unit_id: String,
    /// The work unit's definition digest, as the driver says it played it.
    pub definition_sha256: String,
    pub started_unix: u64,
    pub ended_unix: u64,
    pub process: SessionProcess,
    pub phases: Vec<Phase>,
    /// The game process tree is proven settled after the session.
    pub settled: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LongProgress {
    Running,
    Completed(Box<LongSessionRecord>),
    /// The session ended without completing the route; the reason is typed.
    Failed(&'static str),
}

/// The lifecycle a producer follows: start the session for a unit, poll it to its end, and abort it
/// (the abort must prove the game settled, or say it cannot).
pub trait LongSessionDriver {
    fn start(&mut self, unit: &LongWorkUnit) -> Result<(), &'static str>;
    fn poll(&mut self) -> LongProgress;
    fn abort(&mut self) -> anyhow::Result<()>;
}

/// The driver this build has: it cannot play a route, and says so.
pub struct UnavailableDriver;

impl LongSessionDriver for UnavailableDriver {
    fn start(&mut self, _unit: &LongWorkUnit) -> Result<(), &'static str> {
        Err("BlockedNoLongSessionDriver")
    }

    fn poll(&mut self) -> LongProgress {
        LongProgress::Failed("BlockedNoLongSessionDriver")
    }

    fn abort(&mut self) -> anyhow::Result<()> {
        // Nothing was ever started, so there is nothing to settle.
        Ok(())
    }
}

/// Whether a record stands for the authorized session of `unit` within `authorized_seconds`.
pub fn verify(
    record: &LongSessionRecord,
    unit: &LongWorkUnit,
    authorized_seconds: u32,
) -> Result<(), &'static str> {
    if record.unit_id != unit.id || record.definition_sha256 != unit.definition_sha256 {
        return Err("LongSessionNotTheAuthorizedUnit");
    }
    let Some(elapsed) = record.ended_unix.checked_sub(record.started_unix) else {
        return Err("LongSessionTimeReversed");
    };
    // The whole route must have been played, and never past the authorized window.
    if elapsed < u64::from(unit.duration_seconds) {
        return Err("LongSessionShorterThanTheUnit");
    }
    if elapsed > u64::from(authorized_seconds) {
        return Err("LongSessionOutlastedTheAuthorizedWindow");
    }
    if record.process.pid.as_u32() == 0 || record.process.start_ticks == 0 {
        return Err("LongSessionProcessNotIdentified");
    }
    if !record.settled {
        return Err("LongSessionNotSettled");
    }
    // Phases lie inside the session, do not overlap, and account for almost all of it.
    let mut covered = 0u64;
    let mut previous_end = record.started_unix;
    for phase in &record.phases {
        if phase.started_unix < previous_end
            || phase.ended_unix < phase.started_unix
            || phase.ended_unix > record.ended_unix
        {
            return Err("LongSessionPhasesInconsistent");
        }
        covered += phase.ended_unix - phase.started_unix;
        previous_end = phase.ended_unix;
    }
    if covered * 100 < elapsed * MIN_PHASE_COVERAGE_PERCENT {
        return Err("LongSessionMostlyUnobserved");
    }
    Ok(())
}

#[cfg(test)]
#[path = "long_receipt_tests.rs"]
mod tests;
