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
    /// Whether settlement and restoration are proven when this run ends.
    pub ownership: Ownership,
    /// The confirmation rule a qualified pilot froze; `None` for every other study.
    pub rule: Option<super::pilot_rule::FrozenConfirmationRule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    Settled,
    Unresolved,
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
    /// Stop an in-flight study, returning its durable interrupted decision if it created artifacts.
    fn abort_study(&mut self, _study: &ChildStudy) -> Result<Option<StudyRun>, &'static str> {
        self.abort()?;
        Ok(None)
    }
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
    /// A process, worker transaction or edited settings file may still be owned.
    OwnershipUnresolved(&'static str),
    /// Every planned study has a recorded decision.
    Done,
}

pub struct StageRunner {
    ledger: ChildLedger,
    max_attempts: u32,
    /// The study being polled. Not persisted: after a restart the first undecided study is chosen
    /// again and the executor adopts its journal.
    in_flight: Option<ChildStudy>,
    /// Retain the settled decision across a failed ledger write; never release it silently.
    pending_abort: Option<StudyRun>,
    unresolved: Option<&'static str>,
}

impl StageRunner {
    /// Settle the actual in-flight study and record any durable interrupted outcome.
    pub fn abort(
        &mut self,
        executor: &mut dyn StudyExecutor,
    ) -> anyhow::Result<Result<(), &'static str>> {
        if let Some(reason) = self.unresolved {
            return Ok(Err(reason));
        }
        let Some(study) = self.in_flight.as_ref() else {
            return Ok(executor.abort());
        };
        if self.pending_abort.is_none() {
            self.pending_abort = match executor.abort_study(study) {
                Ok(outcome) => outcome,
                Err(reason) => {
                    self.unresolved = Some(reason);
                    return Ok(Err(reason));
                }
            };
        }
        if let Some(run) = &self.pending_abort {
            self.ledger
                .record_run(study, run.result, run.reason, run.attempts_used)?;
        }
        self.pending_abort = None;
        self.in_flight = None;
        Ok(Ok(()))
    }
    pub fn new(ledger: ChildLedger, max_attempts: u32) -> Self {
        Self {
            ledger,
            max_attempts,
            in_flight: None,
            pending_abort: None,
            unresolved: None,
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
    /// `OwnershipUnresolved` requires the caller to keep the campaign's ownership.
    pub fn step_fenced(
        &mut self,
        executor: &mut dyn StudyExecutor,
        blocker: &dyn Fn(&ChildStudy) -> Option<&'static str>,
        health_blocked: &mut dyn FnMut() -> bool,
        fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<RunnerStep> {
        if let Some(reason) = self.unresolved {
            return Ok(RunnerStep::OwnershipUnresolved(reason));
        }
        if health_blocked() {
            // A health stop is not permission to leave a profile applied or a game unsettled: the
            // running study is settled and its settings restored first, and that proof is what the
            // halt reports when it fails.
            return Ok(match self.abort(executor)? {
                Ok(()) => RunnerStep::Halted("BlockedGpuStability"),
                Err(reason) => RunnerStep::OwnershipUnresolved(reason),
            });
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
                // Consumption and ownership are unknown. Do not fabricate an observed count;
                // retain the study and prevent any further execution until recovery resolves it.
                log::error!("child study executor failed: {error:#}");
                self.in_flight = Some(study);
                self.unresolved = Some("ExecutorError");
                return Ok(RunnerStep::OwnershipUnresolved("ExecutorError"));
            }
        };
        if run.ownership == Ownership::Unresolved {
            self.in_flight = Some(study);
            self.unresolved = Some(run.reason);
            return Ok(RunnerStep::OwnershipUnresolved(run.reason));
        }
        // Qualified evidence needs at least the study's own attempts.
        let minimum = if matches!(study.stage, super::child_study::Stage::Pilot) {
            (2 * super::pilot_rule::MIN_PILOT_RUNS) as u32
        } else {
            study.min_attempts
        };
        let (result, reason) =
            if run.result == StudyResult::Qualified && run.attempts_used < minimum {
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
        if run.halt || run.ownership == Ownership::Unresolved {
            let code = if reason.is_empty() { "Halted" } else { reason };
            return Ok(match run.ownership {
                Ownership::Settled => RunnerStep::Halted(code),
                Ownership::Unresolved => RunnerStep::OwnershipUnresolved(code),
            });
        }
        Ok(RunnerStep::Ran { study, result })
    }
}

#[cfg(test)]
#[path = "child_runner_tests.rs"]
mod tests;
