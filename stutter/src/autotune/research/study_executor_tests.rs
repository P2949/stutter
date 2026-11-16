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
}

impl ProfileHook for Profiles {
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
}

impl StudyAttempts for Attempts {
    fn run(&mut self, _: &ChildStudy, _: u32) -> anyhow::Result<AttemptsOutcome> {
        self.log.borrow_mut().push("run");
        self.outcome.clone().map_err(|e| anyhow::anyhow!(e))
    }
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
        HostStudyExecutor {
            profiles: Profiles {
                log: log.clone(),
                apply,
                restore,
            },
            attempts: Attempts {
                log: log.clone(),
                outcome,
            },
        },
        log,
    )
}

#[test]
fn a_clean_study_applies_runs_restores_and_is_qualified() {
    let study = cpu_screen();
    let (mut exec, log) = executor(Ok(true), Ok(()), Ok(good(&study)));
    let outcome = exec.execute(&study, 100).unwrap();
    assert_eq!(outcome.result, StudyResult::Qualified);
    assert_eq!(outcome.attempts_used, study.min_attempts);
    assert!(!outcome.halt);
    assert_eq!(*log.borrow(), ["apply", "run", "restore"]);
}

#[test]
fn a_profile_that_cannot_be_applied_runs_nothing() {
    let study = cpu_screen();
    let (mut blocked, log) = executor(
        Err(ProfileFailure::Blocked("BlockedProfileMapping")),
        Ok(()),
        Ok(good(&study)),
    );
    let outcome = blocked.execute(&study, 100).unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.attempts_used),
        (StudyResult::Blocked, "BlockedProfileMapping", 0)
    );
    assert_eq!(*log.borrow(), ["apply"], "no attempt, nothing to restore");
    let (mut failed, _) = executor(
        Err(ProfileFailure::Failed("ReadbackMismatch")),
        Ok(()),
        Ok(good(&study)),
    );
    assert_eq!(
        failed.execute(&study, 100).unwrap().result,
        StudyResult::Failed
    );
}

#[test]
fn the_profile_is_restored_after_every_outcome_and_an_unproven_restore_halts_the_campaign() {
    let study = cpu_screen();
    // The attempts error out: the profile is still restored, and the error surfaces.
    let (mut errored, log) = executor(Ok(true), Ok(()), Err("steam went away"));
    assert!(errored.execute(&study, 100).is_err());
    assert_eq!(*log.borrow(), ["apply", "run", "restore"]);
    // A restore that cannot be proven ends the funnel, whatever the attempts did.
    let (mut unproven, _) = executor(Ok(true), Err("mismatch"), Ok(good(&study)));
    let outcome = unproven.execute(&study, 100).unwrap();
    assert_eq!(
        (outcome.result, outcome.reason, outcome.halt),
        (StudyResult::Failed, "RestoreUnproven", true)
    );
    let (mut both, _) = executor(Ok(true), Err("mismatch"), Err("steam went away"));
    let outcome = both.execute(&study, 100).unwrap();
    assert!(outcome.halt, "restore failure dominates an attempts error");
    // A study that applied no profile has nothing to restore.
    let (mut none, log) = executor(Ok(false), Err("never called"), Ok(good(&study)));
    assert_eq!(
        none.execute(&study, 100).unwrap().result,
        StudyResult::Qualified
    );
    assert_eq!(*log.borrow(), ["apply", "run"]);
}

#[test]
fn health_incomplete_and_unproven_context_outcomes_are_not_qualified() {
    let study = cpu_screen();
    let case = |mutate: &dyn Fn(&mut AttemptsOutcome)| {
        let mut outcome = good(&study);
        mutate(&mut outcome);
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        exec.execute(&study, 100).unwrap()
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
        exec.execute(&study, 100).unwrap()
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
