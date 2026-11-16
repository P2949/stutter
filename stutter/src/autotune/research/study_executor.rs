//! The production `StudyExecutor` (PLAN section 6.3): one child study = apply the frozen graphics
//! profile, run the study's attempts through the normal driver, always restore the profile, and map
//! what happened to a study result. It owns no game logic: profile application and attempt running
//! are traits (the real ones are `profile_applier` and the `Driver` over the Steam host), so every
//! decision here is testable with fakes, and a restore that cannot be proven is fatal to the funnel.

use super::{
    child_runner::{StudyExecutor, StudyRun},
    child_study::{ChildStudy, Profile, StudyResult},
    native_context::ProfileConformance,
};

/// What a profile hook reports when it cannot apply a profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileFailure {
    /// A required setting has no observed mapping: the study cannot run, nothing was changed.
    Blocked(&'static str),
    /// The profile could not be applied for another reason; nothing was left changed.
    Failed(&'static str),
}

/// Apply and restore a study's graphics profile in the study-owned prefixes.
pub trait ProfileHook {
    /// `Ok(true)` when a profile was applied (and must be restored), `Ok(false)` when the study
    /// needs none.
    fn apply(&mut self, profile: Profile) -> Result<bool, ProfileFailure>;
    /// After a crash or restart: put back the settings of any application that was never proven
    /// restored. Run before any study; an `Err` means a profile may still be applied.
    fn recover(&mut self) -> Result<(), &'static str>;
    /// Put the original settings back and prove it. `Err` means the proof failed.
    fn restore(&mut self) -> Result<(), &'static str>;
}

/// What the attempts of one study produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptsOutcome {
    /// Whole attempts that ran to a sealed result.
    pub attempts_used: u32,
    /// Sealed, valid native results among them.
    pub completed: u32,
    /// A GPU reset or any health stop ended the study early.
    pub health_halt: bool,
    /// The comparison context was proven consistent across the study's arms.
    pub context_consistent: bool,
    /// How each completed result's reported context conforms to the study's frozen profile.
    pub conformance: Vec<ProfileConformance>,
}

/// Runs a study's attempts (the real one drives the journalled `Driver` over the Steam host, one
/// tick per poll).
pub trait StudyAttempts {
    /// The study's own durable journal already holds progress: a restart found it mid-run.
    fn has_progress(&mut self, study: &ChildStudy) -> bool;
    /// Advance the study. `Ok(None)` means it is still running.
    fn poll(&mut self, study: &ChildStudy, budget: u32) -> anyhow::Result<Option<AttemptsOutcome>>;
}

enum State {
    Idle,
    /// Attempts are running; `applied` says a profile must be restored when they end.
    Running {
        applied: bool,
    },
}

pub struct HostStudyExecutor<P: ProfileHook, A: StudyAttempts> {
    pub profiles: P,
    pub attempts: A,
    state: State,
}

impl<P: ProfileHook, A: StudyAttempts> HostStudyExecutor<P, A> {
    pub fn new(profiles: P, attempts: A) -> Self {
        Self {
            profiles,
            attempts,
            state: State::Idle,
        }
    }
}

fn run(result: StudyResult, attempts_used: u32, reason: &'static str, halt: bool) -> StudyRun {
    StudyRun {
        result,
        attempts_used,
        reason,
        halt,
    }
}

impl<P: ProfileHook, A: StudyAttempts> HostStudyExecutor<P, A> {
    /// The study's result once its attempts have ended (the profile is already restored).
    fn judge(study: &ChildStudy, outcome: &AttemptsOutcome) -> StudyRun {
        if outcome.health_halt {
            run(StudyResult::Failed, outcome.attempts_used, "GpuReset", true)
        } else if outcome.completed < study.min_attempts {
            run(
                StudyResult::Failed,
                outcome.attempts_used,
                "IncompleteAttempts",
                false,
            )
        } else if outcome.conformance.contains(&ProfileConformance::Violates) {
            run(
                StudyResult::Unqualified,
                outcome.attempts_used,
                "ProfileViolated",
                false,
            )
        } else if !outcome.context_consistent
            || outcome.conformance.contains(&ProfileConformance::Unproven)
        {
            run(
                StudyResult::Unqualified,
                outcome.attempts_used,
                "UnprovenContext",
                false,
            )
        } else {
            run(StudyResult::Qualified, outcome.attempts_used, "", false)
        }
    }
}

impl<P: ProfileHook, A: StudyAttempts> StudyExecutor for HostStudyExecutor<P, A> {
    fn poll(
        &mut self,
        study: &ChildStudy,
        remaining_attempts: u32,
    ) -> anyhow::Result<Option<StudyRun>> {
        if matches!(self.state, State::Idle) {
            // Settings left edited by a crash are restored before anything else is decided.
            if self.profiles.recover().is_err() {
                return Ok(Some(run(
                    StudyResult::Failed,
                    study.min_attempts,
                    "RecoveryUnproven",
                    true,
                )));
            }
            // A study whose journal already holds progress was interrupted (a restart). Whether
            // its profile is applied is unknown and its attempts must never be replayed, so the
            // campaign stops rather than guess.
            if self.attempts.has_progress(study) {
                return Ok(Some(run(
                    StudyResult::Failed,
                    study.min_attempts,
                    "InterruptedStudy",
                    true,
                )));
            }
            let applied = match self.profiles.apply(study.profile) {
                Ok(applied) => applied,
                Err(ProfileFailure::Blocked(reason)) => {
                    return Ok(Some(run(StudyResult::Blocked, 0, reason, false)));
                }
                Err(ProfileFailure::Failed(reason)) => {
                    return Ok(Some(run(StudyResult::Failed, 0, reason, false)));
                }
            };
            self.state = State::Running { applied };
        }
        let State::Running { applied } = self.state else {
            return Ok(None);
        };
        let outcome = match self.attempts.poll(study, remaining_attempts) {
            Ok(None) => return Ok(None),
            Ok(Some(outcome)) => Ok(outcome),
            Err(error) => Err(error),
        };
        // The profile is restored whatever happened, before anything else is decided.
        self.state = State::Idle;
        if applied && self.profiles.restore().is_err() {
            // A game profile that cannot be proven restored stops the whole campaign.
            let used = outcome
                .as_ref()
                .map_or(study.min_attempts, |o| o.attempts_used);
            return Ok(Some(run(
                StudyResult::Failed,
                used,
                "RestoreUnproven",
                true,
            )));
        }
        Ok(Some(Self::judge(study, &outcome?)))
    }
}

#[cfg(test)]
#[path = "study_executor_tests.rs"]
mod tests;
