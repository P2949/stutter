use std::{fs::File, os::unix::fs::PermissionsExt};

use super::*;
use crate::autotune::research::{
    ProtonSource, ResearchAction, ResearchTool,
    child_study::{Profile, Stage, plan},
    tests::intent as base_intent,
};

fn studies() -> Vec<ChildStudy> {
    let mut intent = base_intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    plan(&intent).unwrap()
}

fn open(root: &tempfile::TempDir, plan: Vec<ChildStudy>) -> ChildLedger {
    let path = root.path().join("child-studies");
    if !path.exists() {
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    ChildLedger::open(File::open(path).unwrap(), crate::syscall::geteuid(), plan).unwrap()
}

/// An executor scripted by a closure; it records what it was asked to run.
struct Script<F: FnMut(&ChildStudy) -> anyhow::Result<StudyRun>> {
    run: F,
    asked: Vec<(Profile, Stage)>,
}

impl<F: FnMut(&ChildStudy) -> anyhow::Result<StudyRun>> StudyExecutor for Script<F> {
    fn execute(&mut self, study: &ChildStudy, _: u32) -> anyhow::Result<StudyRun> {
        self.asked.push((study.profile, study.stage));
        (self.run)(study)
    }
}

fn clean(study: &ChildStudy) -> anyhow::Result<StudyRun> {
    Ok(StudyRun {
        result: StudyResult::Qualified,
        attempts_used: study.min_attempts,
        reason: "",
        halt: false,
    })
}

fn no_blocker(_: &ChildStudy) -> Option<&'static str> {
    None
}

fn drive(
    runner: &mut StageRunner,
    executor: &mut dyn StudyExecutor,
    blocker: &dyn Fn(&ChildStudy) -> Option<&'static str>,
) -> Vec<RunnerStep> {
    let mut steps = Vec::new();
    loop {
        let step = runner.step(executor, blocker, &mut || false).unwrap();
        let end = matches!(step, RunnerStep::Done | RunnerStep::Halted(_));
        steps.push(step);
        if end || steps.len() > 40 {
            return steps;
        }
    }
}

#[test]
fn the_whole_funnel_runs_in_order_within_budget_and_ends_done() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut exec = Script {
        run: clean,
        asked: Vec::new(),
    };
    let steps = drive(&mut runner, &mut exec, &no_blocker);
    assert_eq!(steps.last(), Some(&RunnerStep::Done));
    // Funnel order: screens, confirmations, then the action study.
    assert_eq!(exec.asked.len(), plan.len());
    assert_eq!(exec.asked[0], (Profile::Cpu, Stage::Screen));
    assert_eq!(exec.asked[1], (Profile::Gpu, Stage::Screen));
    assert!(
        runner
            .ledger()
            .dispositions()
            .iter()
            .all(|d| d.result == Some(StudyResult::Qualified))
    );
    let spent: u32 = plan.iter().map(|s| s.min_attempts).sum();
    assert_eq!(runner.ledger().spent_attempts(), spent);
}

#[test]
fn the_budget_survives_a_restart_and_unaffordable_studies_are_deferred() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    // Enough for the two screens (2 tools each) and nothing more.
    let budget = plan[0].min_attempts + plan[1].min_attempts;
    let mut runner = StageRunner::new(open(&root, plan.clone()), budget);
    let mut exec = Script {
        run: clean,
        asked: Vec::new(),
    };
    for _ in 0..2 {
        assert!(matches!(
            runner.step(&mut exec, &no_blocker, &mut || false).unwrap(),
            RunnerStep::Ran { .. }
        ));
    }
    drop(runner);
    // A restarted controller reads the spent attempts from the ledger and defers the rest.
    let mut restarted = StageRunner::new(open(&root, plan.clone()), budget);
    assert_eq!(restarted.remaining_attempts(), 0);
    let mut again = Script {
        run: clean,
        asked: Vec::new(),
    };
    let steps = drive(&mut restarted, &mut again, &no_blocker);
    assert_eq!(steps.last(), Some(&RunnerStep::Done));
    assert!(again.asked.is_empty(), "nothing runs without budget");
    let all = restarted.ledger().dispositions();
    assert_eq!(all[0].result, Some(StudyResult::Qualified));
    // The confirmations are deferred for budget; what depends on them is blocked by them.
    assert!(
        all[2..4]
            .iter()
            .all(|d| d.result == Some(StudyResult::DeferredBudget))
    );
    assert!(
        all[4..]
            .iter()
            .all(|d| d.result == Some(StudyResult::Blocked))
    );
}

#[test]
fn an_unqualified_or_failed_prerequisite_blocks_its_dependents() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut exec = Script {
        run: |study: &ChildStudy| {
            if study.profile == Profile::Cpu && study.stage == Stage::Screen {
                return Ok(StudyRun {
                    result: StudyResult::Unqualified,
                    attempts_used: study.min_attempts,
                    reason: "UnprovenProvenance",
                    halt: false,
                });
            }
            clean(study)
        },
        asked: Vec::new(),
    };
    let steps = drive(&mut runner, &mut exec, &no_blocker);
    assert_eq!(steps.last(), Some(&RunnerStep::Done));
    let all = runner.ledger().dispositions();
    // The GPU screen is independent; the CPU confirmation, action and feature studies are blocked.
    assert_eq!(all[1].result, Some(StudyResult::Qualified));
    assert!(
        all.iter()
            .filter(|d| d.profile == Profile::Cpu && d.stage != Stage::Screen)
            .all(|d| d.result == Some(StudyResult::Blocked))
    );
    assert!(!exec.asked.contains(&(Profile::Cpu, Stage::Confirm)));
}

#[test]
fn a_health_stop_ends_the_funnel_without_deciding_what_it_prevented() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut calls = 0;
    let mut exec = Script {
        run: |study: &ChildStudy| {
            calls += 1;
            if calls == 2 {
                return Ok(StudyRun {
                    result: StudyResult::Failed,
                    attempts_used: 1,
                    reason: "GpuReset",
                    halt: true,
                });
            }
            clean(study)
        },
        asked: Vec::new(),
    };
    let steps = drive(&mut runner, &mut exec, &no_blocker);
    assert_eq!(steps.last(), Some(&RunnerStep::Halted("GpuReset")));
    let all = runner.ledger().dispositions();
    assert_eq!(all[1].result, Some(StudyResult::Failed));
    assert!(
        all[2..].iter().all(|d| d.result.is_none()),
        "undecided, not invented"
    );
    // Even a healthy executor cannot run anything once health is blocked, after a restart too.
    drop(runner);
    let mut restarted = StageRunner::new(open(&root, plan), 100);
    let mut healthy = Script {
        run: clean,
        asked: Vec::new(),
    };
    let step = restarted
        .step(&mut healthy, &no_blocker, &mut || true)
        .unwrap();
    assert_eq!(step, RunnerStep::Halted("BlockedGpuStability"));
    assert!(healthy.asked.is_empty());
}

#[test]
fn an_executor_error_is_charged_recorded_failed_and_never_replayed() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut exec = Script {
        run: |_: &ChildStudy| Err(anyhow::anyhow!("steam went away")),
        asked: Vec::new(),
    };
    let step = runner.step(&mut exec, &no_blocker, &mut || false).unwrap();
    assert_eq!(step, RunnerStep::Halted("ExecutorError"));
    let first = &runner.ledger().dispositions()[0];
    assert_eq!(first.result, Some(StudyResult::Failed));
    assert_eq!(first.reason, "ExecutorError");
    assert_eq!(runner.ledger().spent_attempts(), plan[0].min_attempts);
    // The next step moves on (and blocks the dependents); the failed study is not asked again.
    let mut healthy = Script {
        run: clean,
        asked: Vec::new(),
    };
    runner
        .step(&mut healthy, &no_blocker, &mut || false)
        .unwrap();
    assert!(!healthy.asked.contains(&(Profile::Cpu, Stage::Screen)));
}

#[test]
fn qualified_evidence_below_the_studys_own_attempt_count_is_downgraded() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut exec = Script {
        run: |study: &ChildStudy| {
            Ok(StudyRun {
                result: StudyResult::Qualified,
                attempts_used: study.min_attempts - 1,
                reason: "",
                halt: false,
            })
        },
        asked: Vec::new(),
    };
    let step = runner.step(&mut exec, &no_blocker, &mut || false).unwrap();
    assert!(matches!(
        step,
        RunnerStep::Ran {
            result: StudyResult::Unqualified,
            ..
        }
    ));
    assert_eq!(
        runner.ledger().dispositions()[0].reason,
        "InsufficientAttempts"
    );
}

#[test]
fn a_named_blocker_records_the_study_instead_of_running_it() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut exec = Script {
        run: clean,
        asked: Vec::new(),
    };
    let steps = drive(&mut runner, &mut exec, &|_| Some("BlockedProfileApplier"));
    assert_eq!(steps.last(), Some(&RunnerStep::Done));
    assert!(exec.asked.is_empty());
    assert!(
        runner
            .ledger()
            .dispositions()
            .iter()
            .all(|d| d.result == Some(StudyResult::Blocked))
    );
}
