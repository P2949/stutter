//! The resident child-study stage runner (PLAN section 6.3). It walks the frozen funnel through the
//! durable `ChildLedger`: each call decides what cannot run (budget, failed prerequisites, a named
//! blocker), polls the next ready study through a `StudyExecutor`, and records the sealed result with
//! the attempts it consumed, so a restart resumes at the first undecided study with the right
//! remaining budget. A health stop is checked before every call and ends the whole funnel without
//! deciding the studies it prevented. The executor is the only part that touches a game; this
//! module owns ordering, budget, qualification gating and restart safety.
//!
//! The resident controller ticks about once a second and must never block for the hours a study
//! takes, so execution is polled: a study that is still running returns `Pending` and is polled
//! again on the next tick.

use super::{
    child_ledger::ChildLedger,
    child_study::{ChildStudy, StudyResult},
};

/// What an executor reports for one finished child study.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudyRun {
    pub result: StudyResult,
    /// Whole attempts the study consumed.
    pub attempts_used: u32,
    /// Machine-readable cause, empty for a clean qualified result.
    pub reason: &'static str,
    /// A health stop ended the study: nothing further may run in this campaign.
    pub halt: bool,
    /// The confirmation rule a qualified pilot froze; `None` for every other study.
    pub rule: Option<super::pilot_rule::FrozenConfirmationRule>,
}

pub trait StudyExecutor {
    /// Advance one ready study within `remaining_attempts`. `Ok(None)` means it is still running.
    /// The same study is polled again until it finishes, including after a restart (the executor
    /// adopts the study's own durable journal; it never replays an attempt).
    fn poll(
        &mut self,
        study: &ChildStudy,
        remaining_attempts: u32,
    ) -> anyhow::Result<Option<StudyRun>>;
    /// As `poll`, with the controller's launch fence: called before a launch is armed, so the
    /// campaign lease can durably record its launching phase first. Executors that never launch a
    /// game ignore it.
    fn poll_fenced(
        &mut self,
        study: &ChildStudy,
        remaining_attempts: u32,
        _fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<StudyRun>> {
        self.poll(study, remaining_attempts)
    }
    /// The journalled driver's step from the last poll, taken once, so the controller can advance
    /// the lease phases exactly as it does for a whole campaign.
    fn take_step(&mut self) -> Option<super::driver::Step> {
        None
    }
    /// Stop the running study (health stop, cancellation, deadline or shutdown): settle its
    /// attempt and prove no owner remains, then restore whatever it changed. `Err` means that is
    /// unproven, which ends the campaign.
    fn abort(&mut self) -> Result<(), &'static str>;
}

/// Whether a halt code says something may still be running or left edited: a process, worker
/// transaction or settings file whose settlement or restoration could not be proven, or an executor
/// that failed after possibly starting a game. A halt like this is not a settled stop: the campaign
/// keeps its lease and mutation slot, and nothing else may run, until independent recovery proves it.
pub fn leaves_ownership_unresolved(code: &str) -> bool {
    matches!(
        code,
        "SettlementUnproven" | "RecoveryUnproven" | "RestoreUnproven" | "ExecutorError"
    )
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunnerStep {
    /// A study is running; nothing is decided yet.
    Pending,
    /// A study finished and its result is recorded.
    Ran {
        study: ChildStudy,
        result: StudyResult,
    },
    /// The funnel stopped without deciding the rest (health). Nothing was recorded for them.
    Halted(&'static str),
    /// Every planned study has a recorded decision.
    Done,
}

pub struct StageRunner {
    ledger: ChildLedger,
    max_attempts: u32,
    /// The study being polled. Not persisted: after a restart the first undecided study is chosen
    /// again and the executor adopts its journal.
    in_flight: Option<ChildStudy>,
}

impl StageRunner {
    pub fn new(ledger: ChildLedger, max_attempts: u32) -> Self {
        Self {
            ledger,
            max_attempts,
            in_flight: None,
        }
    }

    pub fn remaining_attempts(&self) -> u32 {
        self.max_attempts
            .saturating_sub(self.ledger.spent_attempts())
    }

    pub fn ledger(&self) -> &ChildLedger {
        &self.ledger
    }

    /// The rule a confirmation runs under: the one its qualified pilot froze and the ledger recorded.
    /// `None` for any other study, or when the pilot is not qualified.
    pub fn rule_for(
        &self,
        study: &ChildStudy,
    ) -> Option<super::pilot_rule::FrozenConfirmationRule> {
        if !matches!(
            study.stage,
            super::child_study::Stage::Confirm | super::child_study::Stage::Holdout
        ) {
            return None;
        }
        study
            .depends_on
            .iter()
            .find_map(|digest| self.ledger.rule_of(digest))
            .cloned()
    }

    pub fn step(
        &mut self,
        executor: &mut dyn StudyExecutor,
        blocker: &dyn Fn(&ChildStudy) -> Option<&'static str>,
        health_blocked: &mut dyn FnMut() -> bool,
    ) -> anyhow::Result<RunnerStep> {
        self.step_fenced(executor, blocker, health_blocked, &mut || Ok(()))
    }

    /// As `step`, passing the controller's launch fence to the executor.
    ///
    /// A `Halted` code that `leaves_ownership_unresolved` means a game, a worker transaction or an
    /// edited settings file is not proven settled: the caller must keep the campaign's ownership.
    pub fn step_fenced(
        &mut self,
        executor: &mut dyn StudyExecutor,
        blocker: &dyn Fn(&ChildStudy) -> Option<&'static str>,
        health_blocked: &mut dyn FnMut() -> bool,
        fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<RunnerStep> {
        if health_blocked() {
            // A health stop is not permission to leave a profile applied or a game unsettled: the
            // running study is settled and its settings restored first, and that proof is what the
            // halt reports when it fails.
            self.in_flight = None;
            return Ok(RunnerStep::Halted(match executor.abort() {
                Ok(()) => "BlockedGpuStability",
                Err(reason) => reason,
            }));
        }
        let remaining = self.remaining_attempts();
        let study = match self.in_flight.take() {
            Some(study) => study,
            None => match self.ledger.next_runnable(remaining, blocker)? {
                Some(study) => study,
                None => return Ok(RunnerStep::Done),
            },
        };
        let run = match executor.poll_fenced(&study, remaining, fence) {
            Ok(Some(run)) => run,
            Ok(None) => {
                self.in_flight = Some(study);
                return Ok(RunnerStep::Pending);
            }
            Err(error) => {
                // The study may have started games: it is charged its minimum, recorded failed
                // and the funnel stops; it is never replayed or silently dropped.
                log::error!("child study executor failed: {error:#}");
                self.ledger.record_run(
                    &study,
                    StudyResult::Failed,
                    "ExecutorError",
                    study.min_attempts,
                )?;
                return Ok(RunnerStep::Halted("ExecutorError"));
            }
        };
        // Qualified evidence needs at least the study's own attempts.
        let (result, reason) =
            if run.result == StudyResult::Qualified && run.attempts_used < study.min_attempts {
                (StudyResult::Unqualified, "InsufficientAttempts")
            } else {
                (run.result, run.reason)
            };
        self.ledger.record_run_with_rule(
            &study,
            result,
            reason,
            run.attempts_used,
            run.rule.filter(|_| result == StudyResult::Qualified),
        )?;
        if run.halt {
            return Ok(RunnerStep::Halted(if reason.is_empty() {
                "Halted"
            } else {
                reason
            }));
        }
        Ok(RunnerStep::Ran { study, result })
    }
}

#[cfg(test)]
#[path = "child_runner_tests.rs"]
mod tests;
