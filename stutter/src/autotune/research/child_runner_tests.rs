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
    aborts: u32,
}

impl<F: FnMut(&ChildStudy) -> anyhow::Result<StudyRun>> StudyExecutor for Script<F> {
    fn abort(&mut self) -> Result<(), &'static str> {
        self.aborts += 1;
        Ok(())
    }
    fn poll(&mut self, study: &ChildStudy, _: u32) -> anyhow::Result<Option<StudyRun>> {
        self.asked.push((study.profile, study.stage));
        (self.run)(study).map(Some)
    }
}

fn clean(study: &ChildStudy) -> anyhow::Result<StudyRun> {
    Ok(StudyRun {
        result: StudyResult::Qualified,
        attempts_used: study.min_attempts,
        reason: "",
        halt: false,
        ownership: Ownership::Settled,
        rule: (study.stage == Stage::Pilot)
            .then(|| super::super::pilot_rule::freeze(&[70.0, 73.0, 67.0]).unwrap()),
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
        let end = matches!(
            step,
            RunnerStep::Done | RunnerStep::Halted(_) | RunnerStep::OwnershipUnresolved(_)
        );
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
        aborts: 0,
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
        aborts: 0,
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
        aborts: 0,
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
                    ownership: Ownership::Settled,
                    rule: None,
                });
            }
            clean(study)
        },
        asked: Vec::new(),
        aborts: 0,
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
                    ownership: Ownership::Settled,
                    rule: None,
                });
            }
            clean(study)
        },
        asked: Vec::new(),
        aborts: 0,
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
        aborts: 0,
    };
    let step = restarted
        .step(&mut healthy, &no_blocker, &mut || true)
        .unwrap();
    assert_eq!(step, RunnerStep::Halted("BlockedGpuStability"));
    assert!(healthy.asked.is_empty());
}

#[test]
fn an_executor_error_retains_unknown_consumption_and_never_replays() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut exec = Script {
        run: |_: &ChildStudy| Err(anyhow::anyhow!("steam went away")),
        asked: Vec::new(),
        aborts: 0,
    };
    let step = runner.step(&mut exec, &no_blocker, &mut || false).unwrap();
    assert_eq!(step, RunnerStep::OwnershipUnresolved("ExecutorError"));
    let first = &runner.ledger().dispositions()[0];
    assert_eq!(first.result, None);
    assert_eq!(runner.ledger().spent_attempts(), 0);
    // Unknown consumption retains ownership and cannot authorize another attempt.
    let mut healthy = Script {
        run: clean,
        asked: Vec::new(),
        aborts: 0,
    };
    assert_eq!(
        runner
            .step(&mut healthy, &no_blocker, &mut || false)
            .unwrap(),
        RunnerStep::OwnershipUnresolved("ExecutorError")
    );
    assert!(healthy.asked.is_empty());
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
                ownership: Ownership::Settled,
                rule: None,
            })
        },
        asked: Vec::new(),
        aborts: 0,
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
        aborts: 0,
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

/// An executor that needs several polls before a study finishes.
struct Slow {
    polls_needed: u32,
    seen: u32,
    asked: Vec<(Profile, Stage)>,
    aborted: u32,
}

impl StudyExecutor for Slow {
    fn abort(&mut self) -> Result<(), &'static str> {
        self.seen = 0;
        self.aborted += 1;
        Ok(())
    }
    fn poll(&mut self, study: &ChildStudy, _: u32) -> anyhow::Result<Option<StudyRun>> {
        self.seen += 1;
        if self.seen < self.polls_needed {
            return Ok(None);
        }
        self.seen = 0;
        self.asked.push((study.profile, study.stage));
        clean(study).map(Some)
    }
}

#[test]
fn a_running_study_stays_pending_and_is_neither_decided_nor_skipped_until_it_finishes() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut slow = Slow {
        polls_needed: 3,
        seen: 0,
        asked: Vec::new(),
        aborted: 0,
    };
    for _ in 0..2 {
        assert_eq!(
            runner.step(&mut slow, &no_blocker, &mut || false).unwrap(),
            RunnerStep::Pending
        );
        assert!(
            runner
                .ledger()
                .dispositions()
                .iter()
                .all(|d| d.result.is_none()),
            "nothing is decided while the study runs"
        );
    }
    let done = runner.step(&mut slow, &no_blocker, &mut || false).unwrap();
    assert!(matches!(done, RunnerStep::Ran { .. }));
    assert_eq!(slow.asked, [(Profile::Cpu, Stage::Screen)]);
    // The same study (not the next) was polled throughout.
    assert_eq!(
        runner.ledger().dispositions()[0].result,
        Some(StudyResult::Qualified)
    );
    assert!(runner.ledger().dispositions()[1].result.is_none());
}

#[test]
fn a_restart_while_a_study_is_running_polls_the_same_undecided_study_again() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut slow = Slow {
        polls_needed: 5,
        seen: 0,
        asked: Vec::new(),
        aborted: 0,
    };
    runner.step(&mut slow, &no_blocker, &mut || false).unwrap();
    drop(runner);
    // New process: the first undecided study is chosen again (the executor adopts its journal).
    let mut restarted = StageRunner::new(open(&root, plan), 100);
    let mut fresh = Script {
        run: clean,
        asked: Vec::new(),
        aborts: 0,
    };
    restarted
        .step(&mut fresh, &no_blocker, &mut || false)
        .unwrap();
    assert_eq!(fresh.asked, [(Profile::Cpu, Stage::Screen)]);
}

#[test]
fn a_health_stop_interrupts_a_running_study_without_deciding_it() {
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan), 100);
    let mut slow = Slow {
        polls_needed: 5,
        seen: 0,
        asked: Vec::new(),
        aborted: 0,
    };
    assert_eq!(
        runner.step(&mut slow, &no_blocker, &mut || false).unwrap(),
        RunnerStep::Pending
    );
    assert_eq!(
        runner.step(&mut slow, &no_blocker, &mut || true).unwrap(),
        RunnerStep::Halted("BlockedGpuStability")
    );
    assert_eq!(
        slow.aborted, 1,
        "the running study is settled and restored before the halt"
    );
    assert!(
        runner
            .ledger()
            .dispositions()
            .iter()
            .all(|d| d.result.is_none())
    );
}

struct Stuck;

impl StudyExecutor for Stuck {
    fn poll(&mut self, _: &ChildStudy, _: u32) -> anyhow::Result<Option<StudyRun>> {
        Ok(None)
    }
    fn abort(&mut self) -> Result<(), &'static str> {
        Err("RestoreUnproven")
    }
}

#[test]
fn a_health_stop_whose_restoration_cannot_be_proven_reports_that_not_a_clean_halt() {
    let root = tempfile::tempdir().unwrap();
    let mut runner = StageRunner::new(open(&root, studies()), 100);
    assert_eq!(
        runner.step(&mut Stuck, &no_blocker, &mut || false).unwrap(),
        RunnerStep::Pending
    );
    assert_eq!(
        runner.step(&mut Stuck, &no_blocker, &mut || true).unwrap(),
        RunnerStep::OwnershipUnresolved("RestoreUnproven")
    );
}

#[test]
fn a_qualified_pilots_rule_is_recorded_survives_a_restart_and_reaches_only_its_confirmation() {
    use crate::autotune::research::pilot_rule::freeze;
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let frozen = freeze(&[70.0, 73.0, 67.0]).unwrap();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let rule = frozen.clone();
    let mut exec = Script {
        run: move |study: &ChildStudy| {
            let mut done = clean(study)?;
            if study.stage == Stage::Pilot && study.profile == Profile::Cpu {
                done.rule = Some(rule.clone());
            }
            Ok(done)
        },
        asked: Vec::new(),
        aborts: 0,
    };
    // Screens, then the CPU pilot.
    for _ in 0..3 {
        runner.step(&mut exec, &no_blocker, &mut || false).unwrap();
    }
    let confirm = |profile| {
        plan.iter()
            .find(|s| s.stage == Stage::Confirm && s.profile == profile)
            .unwrap()
            .clone()
    };
    assert_eq!(
        runner.rule_for(&confirm(Profile::Cpu)),
        Some(frozen.clone())
    );
    // The GPU confirmation depends on a different pilot, which froze nothing.
    assert_eq!(runner.rule_for(&confirm(Profile::Gpu)), None);
    // Only a confirmation is handed a rule.
    assert_eq!(runner.rule_for(&plan[0]), None);
    // A restarted controller reads the same rule back from the durable ledger.
    drop(runner);
    let restarted = StageRunner::new(open(&root, plan.clone()), 100);
    assert_eq!(restarted.rule_for(&confirm(Profile::Cpu)), Some(frozen));
}

#[test]
fn a_rule_offered_by_a_study_that_did_not_qualify_is_never_recorded() {
    use crate::autotune::research::pilot_rule::freeze;
    let root = tempfile::tempdir().unwrap();
    let plan = studies();
    let mut runner = StageRunner::new(open(&root, plan.clone()), 100);
    let mut exec = Script {
        run: |study: &ChildStudy| {
            let mut done = clean(study)?;
            done.result = StudyResult::Unqualified;
            done.rule = Some(freeze(&[70.0, 73.0, 67.0]).unwrap());
            Ok(done)
        },
        asked: Vec::new(),
        aborts: 0,
    };
    runner.step(&mut exec, &no_blocker, &mut || false).unwrap();
    assert!(
        plan.iter()
            .all(|study| runner.ledger().rule_of(&study.study_sha256).is_none())
    );
}
