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
    child_runner::{Ownership, StudyExecutor, StudyRun},
    child_study::{ChildStudy, Profile, Stage, StudyResult},
    functional_probe::Functional,
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
    /// As `apply`, for the study whose profile it is; a hook that cares which arms the study runs reads
    /// it here.
    fn apply_study(&mut self, study: &ChildStudy) -> Result<bool, ProfileFailure> {
        self.apply(study.profile)
    }
    /// As `apply_study`, told which arms the study runs (resolved and frozen before this call), so only those
    /// arms' settings are edited and recorded.
    fn apply_arms(
        &mut self,
        study: &ChildStudy,
        _arms: Option<&[ProtonSource]>,
    ) -> Result<bool, ProfileFailure> {
        self.apply_study(study)
    }
    /// After a crash or restart: put back the settings of any application that was never proven
    /// restored. Run before any study; an `Err` means a profile may still be applied.
    fn recover(&mut self) -> Result<(), &'static str>;
    /// Put the original settings back and prove it. `Err` means the proof failed.
    fn restore(&mut self) -> Result<(), &'static str>;
    /// SHA-256 (hex) of each settings file as the applied profile left it: what the game must then run
    /// with. Empty when nothing is applied.
    fn applied_digests(&self) -> Vec<String> {
        Vec::new()
    }
}

impl ProfileHook for Box<dyn ProfileHook + Send> {
    fn applied_digests(&self) -> Vec<String> {
        (**self).applied_digests()
    }
    fn apply(&mut self, profile: Profile) -> Result<bool, ProfileFailure> {
        (**self).apply(profile)
    }
    fn apply_study(&mut self, study: &ChildStudy) -> Result<bool, ProfileFailure> {
        (**self).apply_study(study)
    }
    fn apply_arms(
        &mut self,
        study: &ChildStudy,
        arms: Option<&[ProtonSource]>,
    ) -> Result<bool, ProfileFailure> {
        (**self).apply_arms(study, arms)
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
    /// Attempts frozen in this child after viable arms were selected.
    pub planned_attempts: u32,
    /// The selected tools in the campaign's frozen order.
    pub source_order: Vec<ProtonSource>,
    /// Whole attempts that ran to a sealed result.
    pub attempts_used: u32,
    /// Sealed, valid native results among them.
    pub completed: u32,
    /// Every sealed failure's original code; integrity or health failures cannot become compatibility exclusions.
    pub failures: Vec<String>,
    /// A GPU reset or any health stop ended the study early.
    pub health_halt: bool,
    /// A typed, settled terminal failure from the resident attempt driver.
    pub terminal_failure: Option<super::driver::FailureClass>,
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
    /// `run_fps` holds each run's 99th-percentile frame time as FPS (every run had a known frame
    /// population) rather than its average FPS; the pilot freezes that choice into its rule.
    pub tail_metric: bool,
    /// The stage-specific evidence a feature or action study needs beyond benchmark FPS: runtime proof
    /// that the factor was really active, or the action's outcome with proven restoration. `None`
    /// when nothing collects it.
    pub stage_evidence: Option<bool>,
    /// An action study only: the controller never applied the action during it, nothing was left outstanding, and
    /// that was recorded durably. Not a qualified action; a justified abstention needs the owned-pressure control.
    pub action_not_indicated: bool,
    /// A comparison study only (confirmation, holdout, matched CachyOS): every arm's paired analysis produced a
    /// numeric verdict. A structurally complete study whose analysis was refused (too few valid pairs, drifted
    /// context) compared nothing, so it is not a qualified comparison. `None` for any other study.
    pub comparison_valid: Option<bool>,
    /// An action study only: a valid paired GameRunnableLatency record (`action_latency`) accompanies the safety
    /// evidence. Applying and restoring an action is not a measurement of its effect.
    pub action_metric_valid: bool,
    /// A feature study only: every completed attempt's own treatment (factor on or off) was observed in
    /// the game process's environment. This is activation of the control, not proof that the feature
    /// works; `None` for any other study.
    pub activation: Option<bool>,
    /// A feature study only: what the game itself reported of the factor (see `functional_probe`).
    pub functional: Option<super::functional_probe::Functional>,
}

/// Runs a study's attempts (the real one drives the journalled `Driver` over the Steam host, one
/// tick per poll).
pub trait StudyAttempts {
    /// The arms the study will run across, from the funnel's recorded results; `None` when the attempts do not
    /// know. Resolved before any profile is applied so the profile and the launches use the same arms.
    fn selected_arms(&mut self, _study: &ChildStudy) -> anyhow::Result<Option<Vec<ProtonSource>>> {
        Ok(None)
    }
    /// Exact attempt cost of the selected frozen inputs, before applying a profile or launching.
    fn required_attempts(&mut self, study: &ChildStudy) -> anyhow::Result<u32> {
        Ok(study.min_attempts)
    }
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
    /// The attempt has settled and only its durable accounting failed; polling again retries just
    /// that, within a bound. Never true while a game or worker could still be owned.
    fn persistence_pending(&self) -> bool {
        false
    }
    /// The driver step of the last poll, taken once.
    fn take_step(&mut self) -> Option<super::driver::Step> {
        None
    }
    /// The profile's settings files as it left them, recorded once before the first attempt so a
    /// verifier can check what each game run actually used.
    fn record_applied_settings(&mut self, _study: &ChildStudy, _digests: Vec<String>) {}
    /// Cancel the running attempt and independently settle it. `Ok` only when no game or worker
    /// owner remains (the settlement witness agrees); anything unproven is an `Err`.
    fn abort(&mut self) -> Result<(), &'static str>;
    /// Settle and retain a durable outcome when an interrupted study created artifacts.
    fn abort_outcome(
        &mut self,
        _study: &ChildStudy,
    ) -> Result<Option<AttemptsOutcome>, &'static str> {
        self.abort()?;
        Ok(None)
    }
    /// After a restart: independently settle whatever an interrupted study left running, before
    /// anything is restored. `Ok` only when no owner remains.
    fn settle_interrupted(&mut self, study: &ChildStudy) -> Result<(), &'static str>;
}

enum State {
    Idle,
    /// Ownership cannot be released by another abort; independent recovery is required.
    Unresolved,
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
    /// Idle is the only state that proves no profile or attempt is retained.
    pub(crate) fn is_quiescent(&self) -> bool {
        matches!(self.state, State::Idle)
    }

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
        ownership: Ownership::Settled,
        rule: None,
    }
}

fn unresolved(attempts_used: u32, reason: &'static str) -> StudyRun {
    StudyRun {
        ownership: Ownership::Unresolved,
        ..run(StudyResult::Failed, attempts_used, reason, true)
    }
}

/// A screen is a compatibility census: every authorized slot is sealed, and at least two distinct
/// builds have independently usable whole-run results. A failed build is retained as a failure.
pub(super) fn screen_sufficient(
    required: u32,
    used: u32,
    completed: u32,
    runs: &[(ProtonSource, f64)],
    failures: &[String],
) -> bool {
    if used != required
        || completed < 2
        || runs.len() != completed as usize
        || u64::from(completed) + failures.len() as u64 != u64::from(used)
        || failures
            .iter()
            .any(|code| code != "ExitedWithoutNativeResult")
        || runs.iter().any(|(_, fps)| !fps.is_finite() || *fps <= 0.0)
    {
        return false;
    }
    runs.iter()
        .map(|(source, _)| *source)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        >= 2
}

pub(super) fn pilot_sufficient(
    expected: u32,
    used: u32,
    completed: u32,
    sources: &[ProtonSource],
    runs: &[(ProtonSource, f64)],
    failures: &[String],
) -> bool {
    if !pilot_accounted(expected, used, completed, sources, runs, failures) {
        return false;
    }
    let mut counts = std::collections::BTreeMap::<ProtonSource, usize>::new();
    for (source, fps) in runs {
        if !sources.contains(source) || !fps.is_finite() || *fps <= 0.0 {
            return false;
        }
        *counts.entry(*source).or_default() += 1;
    }
    counts.values().all(|count| *count <= 3)
        && counts.values().filter(|count| **count == 3).count() >= 2
}

fn pilot_accounted(
    expected: u32,
    used: u32,
    completed: u32,
    sources: &[ProtonSource],
    runs: &[(ProtonSource, f64)],
    failures: &[String],
) -> bool {
    used == expected
        && expected as usize == sources.len() * 3
        && u64::from(completed) + failures.len() as u64 == u64::from(used)
        && runs.len() == completed as usize
        && failures
            .iter()
            .all(|code| code == "ExitedWithoutNativeResult")
}

/// The study's result once its attempts have ended and the profile is restored. The caller must
/// derive its inputs from verified evidence; the live and offline paths use this same decision.
pub(super) fn judge(study: &ChildStudy, outcome: &AttemptsOutcome) -> StudyRun {
    let used = outcome.attempts_used;
    if let Some(class) = outcome.terminal_failure {
        run(StudyResult::Failed, used, class.terminal_code(), true)
    } else if outcome.health_halt {
        run(StudyResult::Failed, used, "GpuReset", true)
    } else if study.stage == Stage::Init {
        // Nothing is measured: it qualifies only when every listed arm ran and its prefix was then observed
        // to hold what initialization creates.
        if outcome.attempts_used < study.min_attempts || outcome.completed < study.min_attempts {
            run(StudyResult::Failed, used, "IncompleteAttempts", false)
        } else if outcome.stage_evidence == Some(true) {
            run(StudyResult::Qualified, used, "", false)
        } else {
            run(
                StudyResult::Unqualified,
                used,
                "InitializationUnproven",
                false,
            )
        }
    } else if if matches!(study.stage, Stage::Screen) {
        !screen_sufficient(
            study.min_attempts,
            outcome.attempts_used,
            outcome.completed,
            &outcome.run_fps,
            &outcome.failures,
        )
    } else if matches!(study.stage, Stage::Pilot) {
        !pilot_accounted(
            outcome.planned_attempts,
            outcome.attempts_used,
            outcome.completed,
            &outcome.source_order,
            &outcome.run_fps,
            &outcome.failures,
        )
    } else {
        outcome.attempts_used < study.min_attempts || outcome.completed < study.min_attempts
    } {
        run(StudyResult::Failed, used, "IncompleteAttempts", false)
    } else if let Some(reason) = stage_gap(study, outcome) {
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
    } else if outcome.comparison_valid == Some(false) {
        run(StudyResult::Unqualified, used, "ComparisonRefused", false)
    } else if matches!(study.stage, Stage::Pilot) {
        match pilot_rule(outcome) {
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
    super::pilot_rule::rule_from_all_pilots(&outcome.source_order, &outcome.run_fps)
        .map(|rule| rule.with_tail_metric(outcome.tail_metric))
}

/// A feature or action study is not qualified by benchmark attempts alone.
fn stage_gap(study: &ChildStudy, outcome: &AttemptsOutcome) -> Option<&'static str> {
    if outcome.stage_evidence == Some(true) {
        return (matches!(study.stage, Stage::Action(_)) && !outcome.action_metric_valid)
            .then_some("ActionMetricUnavailable");
    }
    match study.stage {
        // Activation (the treatment reached the game) and function (the feature works) are
        // separate: the first can be observed, the second needs a functional receipt.
        Stage::Feature(_) if outcome.activation == Some(true) => {
            Some(match outcome.functional {
                // The game reported the feature off where it should be on (or on where off).
                Some(Functional::NotObserved) => "FeatureActivatedButNotFunctional",
                // The game's own result carries no report for this factor at all.
                Some(Functional::NotObservable) => "FeatureNotObservable",
                _ => "FeatureFunctionalUnproven",
            })
        }
        Stage::Feature(_) => Some("FeatureActivationUnproven"),
        Stage::Action(_) if outcome.action_not_indicated => Some("ActionNotIndicated"),
        Stage::Action(_) => Some("ActionEvidenceMissing"),
        Stage::Init => Some("InitializationUnproven"),
        Stage::Screen | Stage::Pilot | Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs => {
            None
        }
    }
}

impl<P: ProfileHook, A: StudyAttempts> HostStudyExecutor<P, A> {
    /// Restore the profile (if one was applied) and return to idle. `Err` means it is unproven.
    fn restore_if_applied(&mut self, applied: bool) -> Result<(), &'static str> {
        let result = if applied {
            self.profiles.restore()
        } else {
            Ok(())
        };
        self.state = if result.is_ok() {
            State::Idle
        } else {
            State::Unresolved
        };
        result
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
        if matches!(self.state, State::Unresolved) {
            return Ok(Some(unresolved(0, "OwnershipUnresolved")));
        }
        if matches!(self.state, State::Idle) {
            // A study whose journal already holds progress was interrupted (a restart). Its
            // attempts must never be replayed, and settings are not touched until whatever it left
            // running is independently settled.
            let interrupted = self.attempts.has_progress(study);
            if interrupted && self.attempts.settle_interrupted(study).is_err() {
                self.state = State::Unresolved;
                return Ok(Some(unresolved(0, "SettlementUnproven")));
            }
            // Settings left edited by a crash are restored before anything else is decided.
            if self.profiles.recover().is_err() {
                self.state = State::Unresolved;
                return Ok(Some(unresolved(0, "RecoveryUnproven")));
            }
            if interrupted {
                // A settled prior owner is not evidence of its attempt count. Independent
                // journal and accounting recovery must resolve it before ownership is released.
                self.state = State::Unresolved;
                return Ok(Some(unresolved(0, "InterruptedAccountingUnproven")));
            }
            if self.attempts.required_attempts(study)? > remaining_attempts {
                return Ok(Some(run(
                    StudyResult::DeferredBudget,
                    0,
                    "DeferredBudget",
                    false,
                )));
            }
            // Initialization applies no settings: the settings file does not exist until it has run.
            let applied = match if study.stage == Stage::Init {
                Ok(false)
            } else {
                let arms = self.attempts.selected_arms(study)?;
                self.profiles.apply_arms(study, arms.as_deref())
            } {
                Ok(applied) => applied,
                Err(ProfileFailure::Blocked(reason)) => {
                    return Ok(Some(run(StudyResult::Blocked, 0, reason, false)));
                }
                Err(ProfileFailure::Failed(reason)) => {
                    return Ok(Some(run(StudyResult::Failed, 0, reason, false)));
                }
                // The settings may still be edited: nothing else may run in this campaign.
                Err(ProfileFailure::Unrestored(reason)) => {
                    self.state = State::Unresolved;
                    return Ok(Some(unresolved(0, reason)));
                }
            };
            if applied {
                let digests = self.profiles.applied_digests();
                self.attempts.record_applied_settings(study, digests);
            }
            self.state = State::Running { applied };
        }
        let State::Running { applied } = self.state else {
            return Ok(None);
        };
        let outcome = match self.attempts.poll_fenced(study, remaining_attempts, fence) {
            Ok(None) => return Ok(None),
            Ok(Some(outcome)) => outcome,
            // A settled child whose accounting could not be written yet owns nothing: keep the
            // study running and retry only the persistence on the next poll.
            Err(_) if self.attempts.persistence_pending() => return Ok(None),
            Err(error) => {
                // The attempt may still own a game: settle it first. If that cannot be proven the
                // settings are left as they are (the durable snapshot recovers them later) and the
                // campaign stops; they are never restored under an unresolved owner.
                let recovered = match self.attempts.abort_outcome(study) {
                    Ok(recovered) => recovered,
                    Err(reason) => {
                        self.state = State::Unresolved;
                        return Ok(Some(unresolved(0, reason)));
                    }
                };
                if self.restore_if_applied(applied).is_err() {
                    return Ok(Some(unresolved(
                        recovered.as_ref().map_or(0, |o| o.attempts_used),
                        "RestoreUnproven",
                    )));
                }
                return match recovered {
                    Some(outcome) => Ok(Some(run(
                        StudyResult::Failed,
                        outcome.attempts_used,
                        "ExecutorError",
                        true,
                    ))),
                    None => Err(error),
                };
            }
        };
        let used = outcome.attempts_used;
        // A study that ended normally has settled its own attempts; the profile is restored now.
        if self.restore_if_applied(applied).is_err() {
            return Ok(Some(unresolved(used, "RestoreUnproven")));
        }
        Ok(Some(judge(study, &outcome)))
    }

    /// Stop a running study (a health stop, cancellation, deadline or shutdown): settle its
    /// attempt, verify no owner remains, then restore and prove the settings.
    fn abort(&mut self) -> Result<(), &'static str> {
        if matches!(self.state, State::Unresolved) {
            return Err("OwnershipUnresolved");
        }
        let State::Running { applied } = self.state else {
            return Ok(());
        };
        if self.attempts.abort().is_err() {
            self.state = State::Unresolved;
            return Err("SettlementUnproven");
        }
        self.restore_if_applied(applied)
            .map_err(|_| "RestoreUnproven")
    }

    fn abort_study(&mut self, study: &ChildStudy) -> Result<Option<StudyRun>, &'static str> {
        if matches!(self.state, State::Unresolved) {
            return Err("OwnershipUnresolved");
        }
        let State::Running { applied } = self.state else {
            return Ok(None);
        };
        let outcome = match self.attempts.abort_outcome(study) {
            Ok(outcome) => outcome,
            Err(reason) => {
                self.state = State::Unresolved;
                return Err(reason);
            }
        };
        self.restore_if_applied(applied)
            .map_err(|_| "RestoreUnproven")?;
        Ok(outcome.map(|outcome| {
            run(
                StudyResult::Failed,
                outcome.attempts_used,
                "InterruptedStudy",
                true,
            )
        }))
    }
}

#[cfg(test)]
#[path = "study_executor_tests.rs"]
mod tests;

/// Whether a comparison study's every arm produced a numeric analysis; `None` for a study that compares nothing.
pub(super) fn comparison_valid(stage: Stage, arms: &[super::report::ArmResult]) -> Option<bool> {
    matches!(
        stage,
        Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs
    )
    .then(|| !arms.is_empty() && arms.iter().all(|arm| arm.numeric_verdict.is_some()))
}
