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
}

impl StudyAttempts for Attempts {
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
    assert_eq!(*log.borrow(), ["recover", "apply", "run", "restore"]);
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
        case(vec![ProfileConformance::NotSpecified]).result,
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
        ["recover"],
        "settings recovered first; no profile applied and no attempt run"
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
