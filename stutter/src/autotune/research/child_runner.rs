//! The resident child-study stage runner (PLAN section 6.3). It walks the frozen funnel through the
//! durable `ChildLedger`: each call decides what cannot run (budget, failed prerequisites, a named
//! blocker), runs the next ready study through a `StudyExecutor`, and records the sealed result with
//! the attempts it consumed, so a restart resumes at the first undecided study with the right
//! remaining budget. A health stop is checked before every study and ends the whole funnel without
//! deciding the studies it prevented. The executor is the only part that touches a game; this
//! module owns ordering, budget, qualification gating and restart safety.

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
}

pub trait StudyExecutor {
    /// Run one ready study within `remaining_attempts`.
    fn execute(&mut self, study: &ChildStudy, remaining_attempts: u32) -> anyhow::Result<StudyRun>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum RunnerStep {
    /// A study ran and its result is recorded.
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
}

impl StageRunner {
    pub fn new(ledger: ChildLedger, max_attempts: u32) -> Self {
        Self {
            ledger,
            max_attempts,
        }
    }

    pub fn remaining_attempts(&self) -> u32 {
        self.max_attempts
            .saturating_sub(self.ledger.spent_attempts())
    }

    pub fn ledger(&self) -> &ChildLedger {
        &self.ledger
    }

    pub fn step(
        &mut self,
        executor: &mut dyn StudyExecutor,
        blocker: &dyn Fn(&ChildStudy) -> Option<&'static str>,
        health_blocked: &mut dyn FnMut() -> bool,
    ) -> anyhow::Result<RunnerStep> {
        if health_blocked() {
            return Ok(RunnerStep::Halted("BlockedGpuStability"));
        }
        let remaining = self.remaining_attempts();
        let Some(study) = self.ledger.next_runnable(remaining, blocker)? else {
            return Ok(RunnerStep::Done);
        };
        let run = match executor.execute(&study, remaining) {
            Ok(run) => run,
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
        self.ledger
            .record_run(&study, result, reason, run.attempts_used)?;
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
