//! The orchestration of an owned-pressure control, apart from the world it runs in. The control counts the game-stand-in's
//! runnable latency before a load the campaign owns, starts that load, applies and restores one action while the
//! latency is counted under it, stops and joins the load, and leaves the raw record the receipt is derived from
//! (`pressure_evidence`). Everything that touches the machine (counting a population, applying the action, asking
//! whether the campaign was halted) is a `PressureStage`, so the order of events, the halt checks and the cleanup are
//! proven here with a stage that does nothing to the machine; scheduling it into a campaign is supplying a real stage.
//!
//! A halt is asked before anything starts and again before each step; a load that is running when a step fails is always
//! stopped and joined (`OwnedLoad` stops on drop), and no record is written for a control that did not finish.

use super::{
    owned_load::OwnedLoad,
    pressure_evidence::{ActionRecord, Population, PressureControlEvidence, reconstruct},
};

/// What the control needs from the world it runs in.
pub trait PressureStage {
    /// The current unix second.
    fn now(&mut self) -> u64;
    /// Whether the campaign must not run this control now (a GPU incident, a pause, a cancellation).
    fn halted(&mut self) -> bool;
    /// Count the stand-in's runnable latency over the next `seconds`, with no load running.
    fn baseline(&mut self, seconds: u64) -> Result<Population, &'static str>;
    /// While the load runs: apply the action, hold it and restore it, counting the runnable latency over the next
    /// `seconds`; returns that population and what the gate and the worker recorded of the action.
    fn under_load(&mut self, seconds: u64) -> Result<(Population, ActionRecord), &'static str>;
}

/// The size of one control.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub workers: usize,
    /// How long each counted window is.
    pub window_seconds: u64,
}

/// Run one control and return the raw record the receipt derives from, or why it did not finish. The record is checked
/// to support a receipt before it is returned.
pub fn run(
    stage: &mut impl PressureStage,
    binary_sha256: &str,
    plan: Plan,
) -> Result<PressureControlEvidence, &'static str> {
    if stage.halted() {
        return Err("PressureControlHalted");
    }
    let baseline = stage.baseline(plan.window_seconds)?;
    if stage.halted() {
        return Err("PressureControlHalted");
    }
    // The load stops by itself after a bounded time even if this function never gets to stop it.
    let started = stage.now();
    let load = OwnedLoad::start(
        plan.workers,
        plan.window_seconds.saturating_mul(3).max(1),
        started,
    )?;
    // Any early return below drops the load, which stops and joins it.
    if stage.halted() {
        return Err("PressureControlHalted");
    }
    let (loaded, action) = stage.under_load(plan.window_seconds)?;
    let record = load.stop(stage.now());
    let evidence = PressureControlEvidence {
        schema: 1,
        binary_sha256: binary_sha256.to_owned(),
        captured_unix: stage.now(),
        load: record,
        baseline,
        loaded,
        action,
    };
    reconstruct(&evidence)?;
    Ok(evidence)
}

/// Write the finished control's raw record into the campaign directory (never over an existing one).
pub fn record(dir: &std::fs::File, evidence: &PressureControlEvidence) -> anyhow::Result<()> {
    crate::autotune::research::report_store::write_new(
        dir,
        super::pressure_evidence::EVIDENCE_FILE,
        &serde_json::to_vec_pretty(evidence)?,
    )
}

#[cfg(test)]
#[path = "pressure_runner_tests.rs"]
mod tests;
