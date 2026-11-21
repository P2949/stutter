//! The production `LongSessionDriver` (PLAN section 7): it plays a route plan phase by phase through a
//! `RouteInput`, on the clock, while watching the host-attested game process, and produces the session
//! record from what actually happened (real phase boundaries, the real process identity). It never claims
//! the game is settled: `LongAttemptHost` stops the game and attests that before verifying the record.
//!
//! Everything but the input backend is real and tested here. The one thing this repository cannot supply
//! is the backend that actually sends the route's inputs to the game, so the build's input is
//! `UnavailableInput`: starting a session then fails with the typed `BlockedNoLongSessionDriver`, exactly as
//! the plain `UnavailableDriver` does, and a physical backend is the only thing left to add.

use super::{
    long_receipt::{LongProgress, LongSessionDriver, LongSessionRecord, Phase, SessionProcess},
    long_session::LongWorkUnit,
};

/// One phase of the route: what the player is doing and for how long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteStep {
    pub phase: String,
    pub seconds: u32,
}

/// Sends a route's inputs to the game. The physical implementation does not exist in this repository.
pub trait RouteInput: Send {
    /// Whether this backend can actually drive the game.
    fn available(&self) -> bool;
    /// The route enters a phase: start sending that phase's inputs.
    fn begin_phase(&mut self, phase: &str) -> Result<(), &'static str>;
    /// The route is over or abandoned: stop sending anything.
    fn stop(&mut self);
}

/// The build's input: it cannot drive the game, and says so.
pub struct UnavailableInput;

impl RouteInput for UnavailableInput {
    fn available(&self) -> bool {
        false
    }
    fn begin_phase(&mut self, _phase: &str) -> Result<(), &'static str> {
        Err("BlockedNoLongSessionDriver")
    }
    fn stop(&mut self) {}
}

struct Running {
    unit_id: String,
    definition_sha256: String,
    started: Option<u64>,
    process: Option<SessionProcess>,
    cursor: usize,
    phase_started: u64,
    phases: Vec<Phase>,
}

pub struct RouteDriver<I: RouteInput> {
    input: I,
    route: Vec<RouteStep>,
    clock: Box<dyn FnMut() -> u64 + Send>,
    observed: Option<SessionProcess>,
    state: Option<Running>,
}

impl<I: RouteInput> RouteDriver<I> {
    pub fn new(input: I, route: Vec<RouteStep>, clock: Box<dyn FnMut() -> u64 + Send>) -> Self {
        Self {
            input,
            route,
            clock,
            observed: None,
            state: None,
        }
    }

    fn total(&self) -> u64 {
        self.route.iter().map(|step| u64::from(step.seconds)).sum()
    }

    fn fail(&mut self, code: &'static str) -> LongProgress {
        self.input.stop();
        self.state = None;
        LongProgress::Failed(code)
    }
}

impl<I: RouteInput> LongSessionDriver for RouteDriver<I> {
    fn observe_process(&mut self, process: Option<SessionProcess>) {
        self.observed = process;
    }

    fn start(&mut self, unit: &LongWorkUnit) -> Result<(), &'static str> {
        if !self.input.available() {
            return Err("BlockedNoLongSessionDriver");
        }
        // The route must be exactly the unit's authorized length, never a shorter one passed off as it.
        if self.route.is_empty() || self.total() != u64::from(unit.duration_seconds) {
            return Err("BlockedRouteDoesNotMatchTheUnit");
        }
        self.state = Some(Running {
            unit_id: unit.id.clone(),
            definition_sha256: unit.definition_sha256.clone(),
            started: None,
            process: None,
            cursor: 0,
            phase_started: 0,
            phases: Vec::new(),
        });
        Ok(())
    }

    fn poll(&mut self) -> LongProgress {
        let now = (self.clock)();
        let observed = self.observed;
        let Some(state) = self.state.as_mut() else {
            return LongProgress::Failed("LongSessionNotStarted");
        };
        // The route begins once the host has attested the game process, and the game must stay that
        // very process (pid and start time) for the whole session.
        match (state.process, observed) {
            (None, None) => return LongProgress::Running,
            (None, Some(process)) => {
                state.process = Some(process);
                state.started = Some(now);
                state.phase_started = now;
                let first = self.route[0].phase.clone();
                if let Err(code) = self.input.begin_phase(&first) {
                    return self.fail(code);
                }
                return LongProgress::Running;
            }
            (Some(was), Some(is)) if was == is => {}
            _ => return self.fail("LongSessionProcessLost"),
        }
        // Close every phase whose time has come (a late poll closes several, at their true boundaries).
        loop {
            let step = &self.route[state.cursor];
            let end = state.phase_started + u64::from(step.seconds);
            if now < end {
                return LongProgress::Running;
            }
            state.phases.push(Phase {
                name: step.phase.clone(),
                started_unix: state.phase_started,
                ended_unix: end,
            });
            state.phase_started = end;
            state.cursor += 1;
            if state.cursor == self.route.len() {
                break;
            }
            let next = self.route[state.cursor].phase.clone();
            if let Err(code) = self.input.begin_phase(&next) {
                return self.fail(code);
            }
        }
        self.input.stop();
        let (Some(started), Some(process)) = (state.started, state.process) else {
            return self.fail("LongSessionNotStarted");
        };
        let record = LongSessionRecord {
            unit_id: state.unit_id.clone(),
            definition_sha256: state.definition_sha256.clone(),
            started_unix: started,
            ended_unix: state.phase_started,
            process,
            phases: std::mem::take(&mut state.phases),
            // Never the driver's claim: the host attests settlement before the record is verified.
            settled: false,
        };
        self.state = None;
        LongProgress::Completed(Box::new(record))
    }

    fn abort(&mut self) -> anyhow::Result<()> {
        self.input.stop();
        self.state = None;
        Ok(())
    }
}

#[cfg(test)]
#[path = "route_driver_tests.rs"]
mod tests;
