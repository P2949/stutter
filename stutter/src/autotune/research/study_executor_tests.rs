use std::{cell::RefCell, rc::Rc};

use super::*;
use crate::autotune::research::{
    ProtonSource, ResearchTool,
    child_study::{Stage, plan},
    native_context::ProfileConformance,
    tests::intent as base_intent,
};

fn studies() -> Vec<ChildStudy> {
    let mut intent = base_intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    plan(&intent).unwrap()
}

fn cpu_screen() -> ChildStudy {
    studies()
        .into_iter()
        .find(|s| s.profile == Profile::Cpu && s.stage == Stage::Screen)
        .unwrap()
}

type Log = Rc<RefCell<Vec<&'static str>>>;

struct Profiles {
    log: Log,
    apply: Result<bool, ProfileFailure>,
    restore: Result<(), &'static str>,
    recover: Result<(), &'static str>,
}

impl ProfileHook for Profiles {
    fn recover(&mut self) -> Result<(), &'static str> {
        self.log.borrow_mut().push("recover");
        self.recover
    }
    fn apply(&mut self, _: Profile) -> Result<bool, ProfileFailure> {
        self.log.borrow_mut().push("apply");
        self.apply.clone()
    }
    fn restore(&mut self) -> Result<(), &'static str> {
        self.log.borrow_mut().push("restore");
        self.restore
    }
}

struct Attempts {
    log: Log,
    outcome: Result<AttemptsOutcome, &'static str>,
    /// Polls that return "still running" before the outcome.
    slow_polls: u32,
    progress: bool,
    abort_ok: bool,
    settle_ok: bool,
}

impl StudyAttempts for Attempts {
    fn abort(&mut self) -> Result<(), &'static str> {
        self.log.borrow_mut().push("abort");
        if self.abort_ok {
            Ok(())
        } else {
            Err("SettlementUnproven")
        }
    }
    fn settle_interrupted(&mut self, _: &ChildStudy) -> Result<(), &'static str> {
        self.log.borrow_mut().push("settle");
        if self.settle_ok {
            Ok(())
        } else {
            Err("SettlementUnproven")
        }
    }
    fn has_progress(&mut self, _: &ChildStudy) -> bool {
        self.progress
    }
    fn poll(&mut self, _: &ChildStudy, _: u32) -> anyhow::Result<Option<AttemptsOutcome>> {
        if self.slow_polls > 0 {
            self.slow_polls -= 1;
            return Ok(None);
        }
        self.log.borrow_mut().push("run");
        self.outcome
            .clone()
            .map(Some)
            .map_err(|e| anyhow::anyhow!(e))
    }
}

/// Poll until the study finishes.
fn run_to_end(
    exec: &mut HostStudyExecutor<Profiles, Attempts>,
    study: &ChildStudy,
) -> anyhow::Result<StudyRun> {
    for _ in 0..50 {
        if let Some(done) = exec.poll(study, 100)? {
            return Ok(done);
        }
    }
    anyhow::bail!("the study never finished")
}

fn good(study: &ChildStudy) -> AttemptsOutcome {
    AttemptsOutcome {
        attempts_used: study.min_attempts,
        completed: study.min_attempts,
        health_halt: false,
        context_consistent: true,
        conformance: vec![ProfileConformance::Conforms; study.min_attempts as usize],
        raw_verified: true,
        runtime_matched: true,
        stage_evidence: None,
        run_fps: (0..study.min_attempts)
            .map(|i| {
                let arm = if i % 2 == 0 {
                    ProtonSource::Ge
                } else {
                    ProtonSource::ValveStable
                };
                (arm, 70.0 + f64::from(i % 3) * 0.1)
            })
            .collect(),
    }
}

fn executor(
    apply: Result<bool, ProfileFailure>,
    restore: Result<(), &'static str>,
    outcome: Result<AttemptsOutcome, &'static str>,
) -> (HostStudyExecutor<Profiles, Attempts>, Log) {
    let log: Log = Rc::default();
    (
        HostStudyExecutor::new(
            Profiles {
                log: log.clone(),
                apply,
                restore,
                recover: Ok(()),
            },
            Attempts {
                log: log.clone(),
                outcome,
                slow_polls: 0,
                progress: false,
                abort_ok: true,
                settle_ok: true,
            },
        ),
        log,
    )
}

#[test]
fn a_clean_study_applies_runs_restores_and_is_qualified() {
    let study = cpu_screen();
    let (mut exec, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    let outcome = run_to_end(&mut exec, &study).unwrap();
    assert_eq!(outcome.result, StudyResult::Qualified);
    assert_eq!(outcome.attempts_used, study.min_attempts);
    assert!(!outcome.halt);
    assert_eq!(*log.borrow(), ["recover", "apply", "run", "restore"]);
}

#[test]
fn a_profile_that_cannot_be_applied_runs_nothing() {
    let study = cpu_screen();
    let (mut blocked, log) = executor(
        Err(ProfileFailure::Blocked("BlockedProfileMapping")),
        Ok(()),
        Ok(good(&study)),
    );
    let outcome = run_to_end(&mut blocked, &study).unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.attempts_used),
        (StudyResult::Blocked, "BlockedProfileMapping", 0)
    );
    assert_eq!(
        *log.borrow(),
        ["recover", "apply"],
        "no attempt, nothing to restore"
    );
    let (mut failed, _) = executor(
        Err(ProfileFailure::Failed("ReadbackMismatch")),
        Ok(()),
        Ok(good(&study)),
    );
    assert_eq!(
        run_to_end(&mut failed, &study).unwrap().result,
        StudyResult::Failed
    );
}

#[test]
fn the_profile_is_restored_after_every_outcome_and_an_unproven_restore_halts_the_campaign() {
    let study = cpu_screen();
    // The attempts error out: the profile is still restored, and the error surfaces.
    let (mut errored, log) = executor(Ok(true), Ok(()), Err("steam went away"));
    assert!(run_to_end(&mut errored, &study).is_err());
    assert_eq!(
        *log.borrow(),
        ["recover", "apply", "run", "abort", "restore"],
        "the attempt is settled before the settings are restored"
    );
    // A restore that cannot be proven ends the funnel, whatever the attempts did.
    let (mut unproven, _) = executor(Ok(true), Err("mismatch"), Ok(good(&study)));
    let outcome = run_to_end(&mut unproven, &study).unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.halt),
        (StudyResult::Failed, "RestoreUnproven", true)
    );
    let (mut both, _) = executor(Ok(true), Err("mismatch"), Err("steam went away"));
    let outcome = run_to_end(&mut both, &study).unwrap();
    assert!(outcome.halt, "restore failure dominates an attempts error");
    // A study that applied no profile has nothing to restore.
    let (mut none, log) = executor(Ok(false), Err("never called"), Ok(good(&study)));
    assert_eq!(
        run_to_end(&mut none, &study).unwrap().result,
        StudyResult::Qualified
    );
    assert_eq!(*log.borrow(), ["recover", "apply", "run"]);
}

#[test]
fn health_incomplete_and_unproven_context_outcomes_are_not_qualified() {
    let study = cpu_screen();
    let case = |mutate: &dyn Fn(&mut AttemptsOutcome)| {
        let mut outcome = good(&study);
        mutate(&mut outcome);
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        run_to_end(&mut exec, &study).unwrap()
    };
    let reset = case(&|o| o.health_halt = true);
    assert_eq!(
        (reset.result, reset.reason, reset.halt),
        (StudyResult::Failed, "GpuReset", true)
    );
    let short = case(&|o| o.completed -= 1);
    assert_eq!(
        (short.result, short.reason, short.halt),
        (StudyResult::Failed, "IncompleteAttempts", false)
    );
    let loose = case(&|o| o.context_consistent = false);
    assert_eq!(
        (loose.result, loose.reason, loose.halt),
        (StudyResult::Unqualified, "UnprovenContext", false)
    );
}

#[test]
fn the_executor_plugs_into_the_stage_runner_and_a_blocked_profile_blocks_its_dependents() {
    use std::{fs::File, os::unix::fs::PermissionsExt};

    use crate::autotune::research::{
        child_ledger::ChildLedger,
        child_runner::{RunnerStep, StageRunner},
    };
    let plan = studies();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("child-studies");
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ledger =
        ChildLedger::open(File::open(path).unwrap(), crate::syscall::geteuid(), plan).unwrap();
    let mut runner = StageRunner::new(ledger, 100);
    // Every profile is blocked (no observed index): the funnel records that, runs no attempts and
    // blocks what depends on it.
    let (mut exec, log) = executor(
        Err(ProfileFailure::Blocked("BlockedProfileMapping")),
        Ok(()),
        Ok(good(&cpu_screen())),
    );
    let mut steps = Vec::new();
    loop {
        let step = runner.step(&mut exec, &|_| None, &mut || false).unwrap();
        let done = matches!(step, RunnerStep::Done);
        steps.push(step);
        if done || steps.len() > 20 {
            break;
        }
    }
    assert!(!log.borrow().contains(&"run"));
    let all = runner.ledger().dispositions();
    assert!(all.iter().all(|d| d.result == Some(StudyResult::Blocked)));
    assert_eq!(runner.ledger().spent_attempts(), 0);
}

#[test]
fn a_result_that_did_not_run_the_frozen_profile_is_unqualified() {
    let study = cpu_screen();
    let case = |conformance: Vec<ProfileConformance>| {
        let mut outcome = good(&study);
        outcome.conformance = conformance;
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        run_to_end(&mut exec, &study).unwrap()
    };
    let violated = case(vec![
        ProfileConformance::Conforms,
        ProfileConformance::Violates,
    ]);
    assert_eq!(
        (violated.result, violated.reason),
        (StudyResult::Unqualified, "ProfileViolated")
    );
    let unproven = case(vec![
        ProfileConformance::Conforms,
        ProfileConformance::Unproven,
    ]);
    assert_eq!(
        (unproven.result, unproven.reason),
        (StudyResult::Unqualified, "UnprovenContext")
    );
    assert_eq!(
        case(vec![
            ProfileConformance::NotSpecified;
            study.min_attempts as usize
        ])
        .result,
        StudyResult::Qualified
    );
}

#[test]
fn a_running_study_keeps_its_profile_applied_until_it_ends_and_then_restores_once() {
    let study = cpu_screen();
    let (mut exec, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    exec.attempts.slow_polls = 3;
    for _ in 0..3 {
        assert_eq!(exec.poll(&study, 100).unwrap(), None);
    }
    assert_eq!(
        *log.borrow(),
        ["recover", "apply"],
        "applied once, still running, not restored"
    );
    let done = exec.poll(&study, 100).unwrap().unwrap();
    assert_eq!(done.result, StudyResult::Qualified);
    assert_eq!(*log.borrow(), ["recover", "apply", "run", "restore"]);
}

#[test]
fn a_study_found_mid_run_after_a_restart_is_failed_and_halts_never_replayed() {
    let study = cpu_screen();
    let (mut exec, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    exec.attempts.progress = true;
    let outcome = exec.poll(&study, 100).unwrap().unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.halt),
        (StudyResult::Failed, "InterruptedStudy", true)
    );
    assert_eq!(
        *log.borrow(),
        ["settle", "recover"],
        "the interrupted study is settled, then settings recovered; nothing applied or run"
    );
}

#[test]
fn settings_that_a_crash_left_edited_must_be_recovered_before_any_study_or_the_campaign_halts() {
    let study = cpu_screen();
    let (mut exec, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    exec.profiles.recover = Err("RecoveryUnproven");
    let outcome = exec.poll(&study, 100).unwrap().unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.halt),
        (StudyResult::Failed, "RecoveryUnproven", true)
    );
    assert_eq!(
        *log.borrow(),
        ["recover"],
        "nothing applied or run over unproven settings"
    );
}

fn attempts_failing(
    study: &ChildStudy,
    abort_ok: bool,
) -> (HostStudyExecutor<Profiles, Attempts>, Log) {
    let (mut exec, log) = executor(Ok(true), Ok(()), Err("steam went away"));
    exec.attempts.abort_ok = abort_ok;
    let _ = study;
    (exec, log)
}

#[test]
fn an_attempt_error_settles_before_restoring_and_an_unsettled_attempt_never_gets_its_settings_back()
{
    let study = cpu_screen();
    let (mut settled, log) = attempts_failing(&study, true);
    assert!(run_to_end(&mut settled, &study).is_err());
    assert_eq!(
        *log.borrow(),
        ["recover", "apply", "run", "abort", "restore"]
    );
    // Settlement cannot be proven: no restore under an unresolved game, and the campaign halts.
    let (mut unsettled, log) = attempts_failing(&study, false);
    let outcome = run_to_end(&mut unsettled, &study).unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.halt),
        (StudyResult::Failed, "SettlementUnproven", true)
    );
    assert_eq!(*log.borrow(), ["recover", "apply", "run", "abort"]);
}

#[test]
fn stopping_a_running_study_settles_then_restores_and_reports_what_it_could_not_prove() {
    let study = cpu_screen();
    let (mut exec, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    exec.attempts.slow_polls = 5;
    assert_eq!(exec.poll(&study, 100).unwrap(), None);
    assert_eq!(exec.abort(), Ok(()));
    assert_eq!(*log.borrow(), ["recover", "apply", "abort", "restore"]);
    // Aborting an idle executor changes nothing.
    assert_eq!(exec.abort(), Ok(()));
    assert_eq!(log.borrow().len(), 4);
    // An abort whose settlement is unproven leaves the settings alone and says so.
    let (mut stuck, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    stuck.attempts.slow_polls = 5;
    stuck.attempts.abort_ok = false;
    assert_eq!(stuck.poll(&study, 100).unwrap(), None);
    assert_eq!(stuck.abort(), Err("SettlementUnproven"));
    assert!(!log.borrow().contains(&"restore"));
    // A restoration that cannot be proven is reported too.
    let (mut bad, _) = executor(Ok(true), Err("mismatch"), Ok(good(&study)));
    bad.attempts.slow_polls = 5;
    assert_eq!(bad.poll(&study, 100).unwrap(), None);
    assert_eq!(bad.abort(), Err("RestoreUnproven"));
}

#[test]
fn an_interrupted_study_is_settled_before_recovery_and_an_unproven_settlement_touches_nothing() {
    let study = cpu_screen();
    let (mut exec, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    exec.attempts.progress = true;
    exec.attempts.settle_ok = false;
    let outcome = exec.poll(&study, 100).unwrap().unwrap();
    assert_eq!((outcome.reason, outcome.halt), ("SettlementUnproven", true));
    assert_eq!(
        *log.borrow(),
        ["settle"],
        "settings are not recovered under an unresolved owner"
    );
    let (mut settled, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    settled.attempts.progress = true;
    let outcome = settled.poll(&study, 100).unwrap().unwrap();
    assert_eq!(outcome.reason, "InterruptedStudy");
    assert_eq!(*log.borrow(), ["settle", "recover"]);
}

#[test]
fn an_unproven_rollback_while_applying_is_an_integrity_stop_not_an_ordinary_failure() {
    let study = cpu_screen();
    let (mut exec, log) = executor(
        Err(ProfileFailure::Unrestored("RollbackUnproven")),
        Ok(()),
        Ok(good(&study)),
    );
    let outcome = run_to_end(&mut exec, &study).unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.halt),
        (StudyResult::Failed, "RollbackUnproven", true)
    );
    assert_eq!(
        *log.borrow(),
        ["recover", "apply"],
        "no attempt runs over possibly edited settings"
    );
}

#[test]
fn a_study_is_qualified_only_by_verified_raw_runtime_and_complete_conformance_evidence() {
    let study = cpu_screen();
    let case = |mutate: &dyn Fn(&mut AttemptsOutcome)| {
        let mut outcome = good(&study);
        mutate(&mut outcome);
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        run_to_end(&mut exec, &study).unwrap()
    };
    assert_eq!(case(&|_| {}).result, StudyResult::Qualified);
    let raw = case(&|o| o.raw_verified = false);
    assert_eq!(
        (raw.result, raw.reason),
        (StudyResult::Unqualified, "RawEvidenceUnverified")
    );
    let runtime = case(&|o| o.runtime_matched = false);
    assert_eq!(
        (runtime.result, runtime.reason),
        (StudyResult::Unqualified, "RuntimeUnproven")
    );
    // An empty or short conformance list proves nothing about the profile.
    let empty = case(&|o| o.conformance.clear());
    assert_eq!(
        (empty.result, empty.reason),
        (StudyResult::Unqualified, "UnprovenContext")
    );
    let short = case(&|o| {
        o.conformance.pop();
    });
    assert_eq!(
        (short.result, short.reason),
        (StudyResult::Unqualified, "UnprovenContext")
    );
}

#[test]
fn a_feature_or_action_study_is_not_qualified_by_benchmark_attempts_alone() {
    use crate::autotune::research::{ResearchAction, ResearchFactor};
    let mut intent = base_intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    intent.factors = vec![ResearchFactor::NtSync];
    let plan = plan(&intent).unwrap();
    let feature = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap();
    let action = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Action(_)))
        .unwrap();
    let confirm = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Confirm))
        .unwrap();
    let run_with = |study: &ChildStudy, evidence: Option<bool>| {
        let mut outcome = good(study);
        outcome.stage_evidence = evidence;
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        run_to_end(&mut exec, study).unwrap()
    };
    // Clean benchmark counts, context and evidence, but no proof the factor was active.
    let unproven = run_with(feature, None);
    assert_eq!(
        (unproven.result, unproven.reason),
        (StudyResult::Unqualified, "FeatureActivationUnproven")
    );
    let no_action = run_with(action, None);
    assert_eq!(
        (no_action.result, no_action.reason),
        (StudyResult::Unqualified, "ActionEvidenceMissing")
    );
    let denied = run_with(feature, Some(false));
    assert_eq!(denied.result, StudyResult::Unqualified);
    // With the stage's own evidence they qualify; a plain comparison never needed it.
    assert_eq!(run_with(feature, Some(true)).result, StudyResult::Qualified);
    assert_eq!(run_with(action, Some(true)).result, StudyResult::Qualified);
    assert_eq!(run_with(confirm, None).result, StudyResult::Qualified);
}

fn pilot() -> ChildStudy {
    studies()
        .into_iter()
        .find(|s| s.profile == Profile::Cpu && s.stage == Stage::Pilot)
        .unwrap()
}

#[test]
fn a_qualified_pilot_freezes_the_rule_a_confirmation_will_be_bound_to() {
    let study = pilot();
    assert_eq!(study.min_attempts, 6, "three runs for each of two arms");
    let (mut exec, _) = executor(Ok(true), Ok(()), Ok(good(&study)));
    let outcome = run_to_end(&mut exec, &study).unwrap();
    assert_eq!(outcome.result, StudyResult::Qualified);
    let rule = outcome.rule.expect("a qualified pilot freezes a rule");
    assert!(rule.is_consistent());
    assert_eq!(
        rule.pilot_runs, 3,
        "the rule comes from one arm's three runs"
    );
    // Studies that are not pilots never carry a rule.
    let screen = cpu_screen();
    let (mut other, _) = executor(Ok(true), Ok(()), Ok(good(&screen)));
    assert_eq!(run_to_end(&mut other, &screen).unwrap().rule, None);
}

#[test]
fn a_pilot_that_cannot_support_a_confirmation_is_unqualified_with_its_reason() {
    let study = pilot();
    let case = |mutate: &dyn Fn(&mut AttemptsOutcome)| {
        let mut outcome = good(&study);
        mutate(&mut outcome);
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        let result = run_to_end(&mut exec, &study).unwrap();
        (result.result, result.reason, result.rule)
    };
    // One arm has only two runs.
    let thin = case(&|o| {
        let index = o
            .run_fps
            .iter()
            .position(|(arm, _)| *arm == ProtonSource::Ge)
            .unwrap();
        o.run_fps.remove(index);
    });
    assert_eq!((thin.0, thin.1), (StudyResult::Unqualified, "PilotTooThin"));
    assert!(thin.2.is_none());
    // The runs are wildly noisy.
    let noisy = case(&|o| {
        for (index, (_, fps)) in o.run_fps.iter_mut().enumerate() {
            *fps = [40.0, 100.0, 70.0][index % 3];
        }
    });
    assert_eq!(
        (noisy.0, noisy.1),
        (StudyResult::Unqualified, "PilotTooNoisy")
    );
    // Only one arm produced runs.
    let single = case(&|o| {
        for (arm, _) in o.run_fps.iter_mut() {
            *arm = ProtonSource::Ge;
        }
    });
    assert_eq!(
        (single.0, single.1),
        (StudyResult::Unqualified, "PilotArms")
    );
    // A nonsense measurement.
    let invalid = case(&|o| o.run_fps[0].1 = f64::NAN);
    assert_eq!(
        (invalid.0, invalid.1),
        (StudyResult::Unqualified, "PilotInvalid")
    );
}
