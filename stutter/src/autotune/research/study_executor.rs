//! The production `StudyExecutor` (PLAN section 6.3): one child study = apply the frozen graphics
//! profile, run the study's attempts through the normal driver, settle them, restore the profile,
//! and map what happened to a study result. It owns no game logic: profile application and attempt
//! running are traits (the real ones are `profile_applier` and the `Driver` over the Steam host), so
//! every decision here is testable with fakes.
//!
//! Ordering is the safety property: **settle the attempt, verify no game owner remains, only then
//! restore the settings, and prove the original bytes are back.** Settings are never restored under
//! a game whose ownership is unresolved, and a restoration or settlement that cannot be proven halts
//! the whole campaign.

use super::{
    ProtonSource,
    child_runner::{StudyExecutor, StudyRun},
    child_study::{ChildStudy, Profile, Stage, StudyResult},
    native_context::ProfileConformance,
    pilot_rule::FrozenConfirmationRule,
};

/// What a profile hook reports when it cannot apply a profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileFailure {
    /// A required setting has no observed mapping: the study cannot run, nothing was changed.
    Blocked(&'static str),
    /// The profile could not be applied for another reason; nothing was left changed.
    Failed(&'static str),
    /// The settings may still be edited (a rollback could not be proven): an integrity stop.
    Unrestored(&'static str),
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

impl ProfileHook for Box<dyn ProfileHook + Send> {
    fn apply(&mut self, profile: Profile) -> Result<bool, ProfileFailure> {
        (**self).apply(profile)
    }
    fn recover(&mut self) -> Result<(), &'static str> {
        (**self).recover()
    }
    fn restore(&mut self) -> Result<(), &'static str> {
        (**self).restore()
    }
}

/// What the attempts of one study produced.
#[derive(Clone, Debug, PartialEq)]
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
    /// Every completed attempt's native files replayed against its sealed record.
    pub raw_verified: bool,
    /// Every completed attempt's mapped runtime was matched to its own arm's installation.
    pub runtime_matched: bool,
    /// Each completed attempt's native average FPS with its arm (a pilot freezes its rule from these).
    pub run_fps: Vec<(ProtonSource, f64)>,
    /// The stage-specific evidence a feature or action study needs beyond benchmark FPS: runtime proof
    /// that the factor was really active, or the action's outcome with proven restoration. `None`
    /// when nothing collects it.
    pub stage_evidence: Option<bool>,
}

/// Runs a study's attempts (the real one drives the journalled `Driver` over the Steam host, one
/// tick per poll).
pub trait StudyAttempts {
    /// The study's own durable journal already holds progress: a restart found it mid-run.
    fn has_progress(&mut self, study: &ChildStudy) -> bool;
    /// Advance the study. `Ok(None)` means it is still running.
    fn poll(&mut self, study: &ChildStudy, budget: u32) -> anyhow::Result<Option<AttemptsOutcome>>;
    /// As `poll`, passing the controller's launch fence to the driver.
    fn poll_fenced(
        &mut self,
        study: &ChildStudy,
        budget: u32,
        _fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<AttemptsOutcome>> {
        self.poll(study, budget)
    }
    /// The driver step of the last poll, taken once.
    fn take_step(&mut self) -> Option<super::driver::Step> {
        None
    }
    /// Cancel the running attempt and independently settle it. `Ok` only when no game or worker
    /// owner remains (the settlement witness agrees); anything unproven is an `Err`.
    fn abort(&mut self) -> Result<(), &'static str>;
    /// After a restart: independently settle whatever an interrupted study left running, before
    /// anything is restored. `Ok` only when no owner remains.
    fn settle_interrupted(&mut self, study: &ChildStudy) -> Result<(), &'static str>;
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
        rule: None,
    }
}

impl<P: ProfileHook, A: StudyAttempts> HostStudyExecutor<P, A> {
    /// The study's result once its attempts have ended and the profile is restored. A benchmark
    /// study is qualified only by independently verified evidence, never by counts alone.
    fn judge(study: &ChildStudy, outcome: &AttemptsOutcome) -> StudyRun {
        let used = outcome.attempts_used;
        if outcome.health_halt {
            run(StudyResult::Failed, used, "GpuReset", true)
        } else if outcome.completed < study.min_attempts {
            run(StudyResult::Failed, used, "IncompleteAttempts", false)
        } else if let Some(reason) = Self::stage_gap(study, outcome) {
            run(StudyResult::Unqualified, used, reason, false)
        } else if outcome.conformance.contains(&ProfileConformance::Violates) {
            run(StudyResult::Unqualified, used, "ProfileViolated", false)
        } else if outcome.conformance.len() != outcome.completed as usize {
            // Exactly one conformance verdict per completed result; an empty or short list proves
            // nothing about the profile.
            run(StudyResult::Unqualified, used, "UnprovenContext", false)
        } else if !outcome.raw_verified {
            run(
                StudyResult::Unqualified,
                used,
                "RawEvidenceUnverified",
                false,
            )
        } else if !outcome.runtime_matched {
            run(StudyResult::Unqualified, used, "RuntimeUnproven", false)
        } else if !outcome.context_consistent
            || outcome.conformance.contains(&ProfileConformance::Unproven)
        {
            run(StudyResult::Unqualified, used, "UnprovenContext", false)
        } else if matches!(study.stage, Stage::Pilot) {
            match Self::pilot_rule(outcome) {
                Ok(rule) => StudyRun {
                    rule: Some(rule),
                    ..run(StudyResult::Qualified, used, "", false)
                },
                Err(reason) => run(StudyResult::Unqualified, used, reason, false),
            }
        } else {
            run(StudyResult::Qualified, used, "", false)
        }
    }

    /// The confirmation rule a pilot freezes (the one derivation the verifier also uses).
    fn pilot_rule(outcome: &AttemptsOutcome) -> Result<FrozenConfirmationRule, &'static str> {
        super::pilot_rule::rule_from_runs(&outcome.run_fps)
    }

    /// A feature or action study is not qualified by benchmark attempts alone.
    fn stage_gap(study: &ChildStudy, outcome: &AttemptsOutcome) -> Option<&'static str> {
        if outcome.stage_evidence == Some(true) {
            return None;
        }
        match study.stage {
            Stage::Feature(_) => Some("FeatureActivationUnproven"),
            Stage::Action(_) => Some("ActionEvidenceMissing"),
            Stage::Screen | Stage::Pilot | Stage::Confirm => None,
        }
    }

    /// Restore the profile (if one was applied) and return to idle. `Err` means it is unproven.
    fn restore_if_applied(&mut self, applied: bool) -> Result<(), &'static str> {
        self.state = State::Idle;
        if applied {
            self.profiles.restore()
        } else {
            Ok(())
        }
    }
}

impl<P: ProfileHook, A: StudyAttempts> StudyExecutor for HostStudyExecutor<P, A> {
    fn poll(
        &mut self,
        study: &ChildStudy,
        remaining_attempts: u32,
    ) -> anyhow::Result<Option<StudyRun>> {
        self.poll_fenced(study, remaining_attempts, &mut || Ok(()))
    }

    fn take_step(&mut self) -> Option<super::driver::Step> {
        self.attempts.take_step()
    }

    fn poll_fenced(
        &mut self,
        study: &ChildStudy,
        remaining_attempts: u32,
        fence: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<Option<StudyRun>> {
        if matches!(self.state, State::Idle) {
            // A study whose journal already holds progress was interrupted (a restart). Its
            // attempts must never be replayed, and settings are not touched until whatever it left
            // running is independently settled.
            let interrupted = self.attempts.has_progress(study);
            if interrupted && self.attempts.settle_interrupted(study).is_err() {
                return Ok(Some(run(
                    StudyResult::Failed,
                    study.min_attempts,
                    "SettlementUnproven",
                    true,
                )));
            }
            // Settings left edited by a crash are restored before anything else is decided.
            if self.profiles.recover().is_err() {
                return Ok(Some(run(
                    StudyResult::Failed,
                    study.min_attempts,
                    "RecoveryUnproven",
                    true,
                )));
            }
            if interrupted {
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
                // The settings may still be edited: nothing else may run in this campaign.
                Err(ProfileFailure::Unrestored(reason)) => {
                    return Ok(Some(run(StudyResult::Failed, 0, reason, true)));
                }
            };
            self.state = State::Running { applied };
        }
        let State::Running { applied } = self.state else {
            return Ok(None);
        };
        let outcome = match self.attempts.poll_fenced(study, remaining_attempts, fence) {
            Ok(None) => return Ok(None),
            Ok(Some(outcome)) => Ok(outcome),
            Err(error) => {
                // The attempt may still own a game: settle it first. If that cannot be proven the
                // settings are left as they are (the durable snapshot recovers them later) and the
                // campaign stops; they are never restored under an unresolved owner.
                if self.attempts.abort().is_err() {
                    self.state = State::Idle;
                    return Ok(Some(run(
                        StudyResult::Failed,
                        study.min_attempts,
                        "SettlementUnproven",
                        true,
                    )));
                }
                Err(error)
            }
        };
        let used = outcome
            .as_ref()
            .map_or(study.min_attempts, |o| o.attempts_used);
        // A study that ended normally has settled its own attempts; the profile is restored now.
        if self.restore_if_applied(applied).is_err() {
            return Ok(Some(run(
                StudyResult::Failed,
                used,
                "RestoreUnproven",
                true,
            )));
        }
        Ok(Some(Self::judge(study, &outcome?)))
    }

    /// Stop a running study (a health stop, cancellation, deadline or shutdown): settle its
    /// attempt, verify no owner remains, then restore and prove the settings.
    fn abort(&mut self) -> Result<(), &'static str> {
        let State::Running { applied } = self.state else {
            return Ok(());
        };
        if self.attempts.abort().is_err() {
            return Err("SettlementUnproven");
        }
        self.restore_if_applied(applied)
            .map_err(|_| "RestoreUnproven")
    }
}

#[cfg(test)]
#[path = "study_executor_tests.rs"]
mod tests;
