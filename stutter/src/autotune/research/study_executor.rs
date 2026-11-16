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

/// Runs a study's attempts (the real one drives the journalled `Driver` over the Steam host).
pub trait StudyAttempts {
    fn run(&mut self, study: &ChildStudy, budget: u32) -> anyhow::Result<AttemptsOutcome>;
}

pub struct HostStudyExecutor<P: ProfileHook, A: StudyAttempts> {
    pub profiles: P,
    pub attempts: A,
}

fn run(result: StudyResult, attempts_used: u32, reason: &'static str, halt: bool) -> StudyRun {
    StudyRun {
        result,
        attempts_used,
        reason,
        halt,
    }
}

impl<P: ProfileHook, A: StudyAttempts> StudyExecutor for HostStudyExecutor<P, A> {
    fn execute(&mut self, study: &ChildStudy, remaining_attempts: u32) -> anyhow::Result<StudyRun> {
        let applied = match self.profiles.apply(study.profile) {
            Ok(applied) => applied,
            Err(ProfileFailure::Blocked(reason)) => {
                return Ok(run(StudyResult::Blocked, 0, reason, false));
            }
            Err(ProfileFailure::Failed(reason)) => {
                return Ok(run(StudyResult::Failed, 0, reason, false));
            }
        };
        let outcome = self.attempts.run(study, remaining_attempts);
        // The profile is restored whatever happened, before anything else is decided.
        if applied && self.profiles.restore().is_err() {
            // A game profile that cannot be proven restored stops the whole campaign.
            let used = outcome
                .as_ref()
                .map_or(study.min_attempts, |o| o.attempts_used);
            return Ok(run(StudyResult::Failed, used, "RestoreUnproven", true));
        }
        let outcome = outcome?;
        Ok(if outcome.health_halt {
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
        })
    }
}

#[cfg(test)]
#[path = "study_executor_tests.rs"]
mod tests;
