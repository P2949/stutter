use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    executor::{Executor, RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::{Graph, TaskKind, TaskSpec, core_validation_tasks},
    model::{BlockReason, Boundary, CampaignMode, FailureClass, TaskStatus},
    repository::Binaries,
    state::{CampaignDir, save},
    status,
};

type Script = dyn Fn(&TaskSpec, u32) -> RunnerOutcome;

struct Fake(Arc<Script>, Arc<Mutex<Vec<String>>>);

impl TaskRunner for Fake {
    fn run(&self, _context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        self.1.lock().unwrap().push(spec.id.to_owned());
        (self.0)(spec, attempt)
    }
}

fn rc() -> String {
    "fedcba9876543210fedcba9876543210fedcba98".to_owned()
}

fn executor(script: Arc<Script>, order: &Arc<Mutex<Vec<String>>>) -> Executor {
    let mut runners: BTreeMap<TaskKind, Box<dyn TaskRunner>> = BTreeMap::new();
    for spec in core_validation_tasks() {
        runners.insert(spec.kind, Box::new(Fake(script.clone(), order.clone())));
    }
    Executor {
        graph: Graph::core().unwrap(),
        manifest: manifest(),
        runners,
        repository: PathBuf::from("/nonexistent"),
        binaries: Binaries {
            stutter: PathBuf::from("s"),
            xtask: PathBuf::from("x"),
        },
        source_check: Box::new(|_, _| Ok(())),
        only: None,
    }
}

fn pass(spec: &TaskSpec) -> RunnerOutcome {
    if spec.kind == TaskKind::FreezeRc {
        return RunnerOutcome::Passed {
            artifacts: None,
            receipt: None,
            limitations: vec![],
            fingerprint: Default::default(),
            updates: vec![StateUpdate::SetReleaseCandidate(rc())],
        };
    }
    RunnerOutcome::passed()
}

fn all_pass() -> Arc<Script> {
    Arc::new(|spec, _| pass(spec))
}

fn setup(
    name: &str,
) -> (
    Scratch,
    CampaignDir,
    crate::validation_campaign::model::CampaignState,
) {
    let scratch = Scratch::new(name);
    let (dir, state, _) = fresh(&scratch);
    (scratch, dir, state)
}

#[test]
fn a_full_run_respects_the_dag_and_completes() {
    let (_scratch, dir, mut state) = setup("full");
    let order = Arc::new(Mutex::new(Vec::new()));
    let executor = executor(all_pass(), &order);
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::Complete);
    assert_eq!(state.mode, CampaignMode::Complete);
    assert_eq!(report.ran.len(), core_validation_tasks().len());
    let order = order.lock().unwrap().clone();
    let position = |id: &str| order.iter().position(|x| x == id).unwrap();
    for spec in core_validation_tasks() {
        for dependency in spec.depends_on {
            assert!(
                position(dependency) < position(spec.id),
                "{dependency} -> {}",
                spec.id
            );
        }
    }
    let report = status::build(&state, &executor.graph, &manifest());
    assert_eq!(report.overall, "complete");
    assert!(
        report
            .repository_actions
            .iter()
            .filter(|a| a.kind == "close_issue")
            .all(|a| a.ready)
    );
    assert_eq!(state.release_candidate.as_deref(), Some(rc().as_str()));
}

#[test]
fn a_retryable_failure_retries_within_budget_and_keeps_every_attempt() {
    let (_scratch, dir, mut state) = setup("retry");
    let order = Arc::new(Mutex::new(Vec::new()));
    let script: Arc<Script> = Arc::new(|spec, attempt| {
        if spec.id == "offline-ready" && attempt == 1 {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "launch".into(),
                summary: "hiccup".into(),
                artifacts: None,
            };
        }
        pass(spec)
    });
    let executor = executor(script, &order);
    executor
        .continue_campaign(&dir, &mut state, Some(1))
        .unwrap();
    let task = &state.tasks["offline-ready"];
    assert_eq!(task.attempts.len(), 2);
    assert_eq!(task.status, TaskStatus::Passed);
    assert_eq!(task.failed_attempts(), 1);
}

#[test]
fn an_exhausted_retry_budget_blocks_honestly_and_is_retried_on_the_next_continue() {
    let (_scratch, dir, mut state) = setup("exhausted");
    let order = Arc::new(Mutex::new(Vec::new()));
    let healthy = Arc::new(AtomicBool::new(false));
    let flag = healthy.clone();
    let script: Arc<Script> = Arc::new(move |spec, _| {
        if spec.id == "offline-ready" && !flag.load(Ordering::SeqCst) {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "launch".into(),
                summary: "still broken".into(),
                artifacts: None,
            };
        }
        pass(spec)
    });
    let executor = executor(script, &order);
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::BlockedResource);
    assert_eq!(
        state.tasks["offline-ready"].status,
        TaskStatus::BlockedResource
    );
    assert_eq!(state.tasks["offline-ready"].attempts.len(), 2);
    assert!(
        state.tasks["offline-ready"]
            .block
            .as_ref()
            .unwrap()
            .detail
            .contains("retry budget exhausted")
    );
    healthy.store(true, Ordering::SeqCst);
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::Complete);
    assert_eq!(
        state.tasks["offline-ready"].attempts.len(),
        3,
        "old failures retained"
    );
}

#[test]
fn a_privilege_block_stops_its_dependents_but_not_independent_work() {
    let (_scratch, dir, mut state) = setup("privilege");
    let order = Arc::new(Mutex::new(Vec::new()));
    let script: Arc<Script> = Arc::new(|spec, _| {
        if spec.id == "live-observation" {
            return RunnerOutcome::Blocked(BlockReason {
                class: FailureClass::BlockedPrivilege,
                detail: "no CAP_BPF".into(),
            });
        }
        pass(spec)
    });
    let executor = executor(script, &order);
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::BlockedPrivilege);
    let ran: BTreeSet<_> = report.ran.iter().map(String::as_str).collect();
    assert!(
        ran.contains("game-runnable") && ran.contains("corpus"),
        "independent tasks still run"
    );
    assert!(
        !ran.contains("evidence-integration"),
        "dependents wait for the blocked task"
    );
    let status = status::build(&state, &executor.graph, &manifest());
    assert_eq!(status.overall, "blocked_privilege");
    assert_eq!(status.blocked[0].task, "live-observation");
}

#[test]
fn a_software_defect_and_a_reboot_wait_are_distinct_boundaries() {
    let (_scratch, dir, mut state) = setup("defect");
    let order = Arc::new(Mutex::new(Vec::new()));
    let defect: Arc<Script> = Arc::new(|spec, _| {
        if spec.id == "offline-ready" {
            return RunnerOutcome::Failed {
                class: FailureClass::SoftwareDefect,
                stage: "build-gate".into(),
                summary: "clippy".into(),
                artifacts: None,
            };
        }
        pass(spec)
    });
    let report = executor(defect, &order)
        .continue_campaign(&dir, &mut state, None)
        .unwrap();
    assert_eq!(report.boundary, Boundary::FailureRequiresSourceChange);
    assert_eq!(
        state.tasks["offline-ready"].attempts.len(),
        1,
        "a defect is not retried"
    );

    let (_scratch2, dir2, mut state2) = setup("reboot");
    let waiting: Arc<Script> = Arc::new(|spec, _| {
        if spec.id == "kernel-matrix" {
            return RunnerOutcome::waiting_for_reboot(Box::new(|| Ok(())));
        }
        pass(spec)
    });
    let report = executor(waiting, &order)
        .continue_campaign(&dir2, &mut state2, None)
        .unwrap();
    assert_eq!(report.boundary, Boundary::WaitingForReboot);
}

#[test]
fn unexpected_source_change_invalidates_source_bound_work_but_keeps_history() {
    let (_scratch, dir, mut state) = setup("invalidate");
    let order = Arc::new(Mutex::new(Vec::new()));
    let moved = Arc::new(AtomicBool::new(false));
    let flag = moved.clone();
    let mut executor = executor(all_pass(), &order);
    executor.source_check = Box::new(move |_, _| {
        if flag.load(Ordering::SeqCst) {
            Err("HEAD moved".to_owned())
        } else {
            Ok(())
        }
    });
    let first = executor
        .continue_campaign(&dir, &mut state, Some(2))
        .unwrap();
    let collected = first.ran[1].clone();
    assert_eq!(state.tasks[collected.as_str()].status, TaskStatus::Passed);
    moved.store(true, Ordering::SeqCst);
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::FailureRequiresSourceChange);
    assert_eq!(state.mode, CampaignMode::Invalidated);
    assert_eq!(
        state.tasks[collected.as_str()].status,
        TaskStatus::Invalidated
    );
    assert_eq!(
        state.tasks[collected.as_str()].attempts.len(),
        1,
        "evidence stays readable"
    );
    assert!(
        state.ready(&executor.graph).is_empty(),
        "nothing is runnable once invalidated"
    );
    let status = status::build(&state, &executor.graph, &manifest());
    assert_eq!(status.overall, "invalidated");
}

#[test]
fn max_tasks_bounds_a_run_and_state_survives_a_restart() {
    let (_scratch, dir, mut state) = setup("bounded");
    let order = Arc::new(Mutex::new(Vec::new()));
    let executor = executor(all_pass(), &order);
    let first = executor
        .continue_campaign(&dir, &mut state, Some(3))
        .unwrap();
    assert_eq!(first.ran.len(), 3);
    assert_eq!(first.boundary, Boundary::Idle);
    // Simulate a process restart: reload from disk and finish.
    save(&dir, &state).unwrap();
    let mut reloaded = crate::validation_campaign::state::load(&dir).unwrap();
    let second = executor
        .continue_campaign(&dir, &mut reloaded, None)
        .unwrap();
    assert_eq!(second.boundary, Boundary::Complete);
    assert_eq!(
        first.ran.len() + second.ran.len(),
        core_validation_tasks().len()
    );
}

#[test]
fn a_moved_main_after_collection_is_expected_until_the_release_candidate_must_match() {
    let (_scratch, dir, mut state) = setup("moved-main");
    let order = Arc::new(Mutex::new(Vec::new()));
    let moved = Arc::new(AtomicBool::new(false));
    let flag = moved.clone();
    let mut executor = executor(all_pass(), &order);
    executor.source_check = Box::new(move |_, _| {
        if flag.load(Ordering::SeqCst) {
            Err("main moved".to_owned())
        } else {
            Ok(())
        }
    });
    // offline gate + the eight collection tasks, then evidence integration merges to main.
    executor
        .continue_campaign(&dir, &mut state, Some(9))
        .unwrap();
    moved.store(true, Ordering::SeqCst);
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    for id in [
        "evidence-integration",
        "diagnosis-calibration",
        "score-review",
        "capability-policy",
        "release-docs",
        "freeze-rc",
    ] {
        assert_eq!(
            state.tasks[id].status,
            TaskStatus::Passed,
            "{id} runs on a moved main"
        );
    }
    // The exact-RC tasks require HEAD == RC, which this simulated main no longer is.
    assert_eq!(report.boundary, Boundary::FailureRequiresSourceChange);
    assert_ne!(state.tasks["rc-offline"].status, TaskStatus::Passed);
}

#[test]
fn a_defect_in_the_candidate_freezes_a_new_one_and_keeps_collection() {
    let (_scratch, dir, mut state) = setup("refreeze");
    let order = Arc::new(Mutex::new(Vec::new()));
    let flaky: Arc<Script> = Arc::new(|spec, attempt| {
        if spec.id == "rc-offline" && attempt == 1 {
            return RunnerOutcome::Failed {
                class: FailureClass::SoftwareDefect,
                stage: "acceptance".into(),
                summary: "gate failed".into(),
                artifacts: None,
            };
        }
        pass(spec)
    });
    let executor = executor(flaky, &order);
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::FailureRequiresSourceChange);
    assert_eq!(state.tasks["rc-offline"].status, TaskStatus::Failed);

    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::Complete);
    assert_eq!(
        state.tasks["rc-offline"].attempts.len(),
        2,
        "history is kept"
    );
    assert_eq!(state.tasks["freeze-rc"].attempts.len(), 2);
    assert_eq!(
        state.tasks["corpus"].attempts.len(),
        1,
        "collection is not rerun"
    );
    assert!(
        state
            .decisions
            .iter()
            .any(|d| d.decision.starts_with("new release candidate required"))
    );
}

#[test]
fn main_moving_under_a_frozen_candidate_freezes_a_new_one() {
    let (_scratch, dir, mut state) = setup("moved-candidate");
    let order = Arc::new(Mutex::new(Vec::new()));
    let moved = Arc::new(AtomicBool::new(true));
    let flag = moved.clone();
    let mut executor = executor(all_pass(), &order);
    executor.source_check = Box::new(move |_, spec| {
        if spec.id == "rc-offline" && flag.swap(false, Ordering::SeqCst) {
            Err("HEAD moved".to_owned())
        } else {
            Ok(())
        }
    });
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.boundary, Boundary::Complete);
    assert_eq!(state.tasks["freeze-rc"].attempts.len(), 2);
    assert_eq!(state.tasks["corpus"].attempts.len(), 1);
}

#[test]
fn only_runs_the_named_ready_task_and_a_reboot_wait_resumes_first() {
    let (_scratch, dir, mut state) = setup("only");
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut executor = executor(all_pass(), &order);
    // `offline-ready` gates the collection: naming a task whose dependency is unmet runs nothing.
    executor.only = Some("kernel-matrix".to_owned());
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert!(report.ran.is_empty(), "{:?}", report.ran);
    executor.only = Some("offline-ready".to_owned());
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.ran, vec!["offline-ready".to_owned()]);
    executor.only = Some("kernel-matrix".to_owned());
    let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
    assert_eq!(report.ran, vec!["kernel-matrix".to_owned()]);
    // A task waiting for a reboot is resumed before any other pending task.
    let (_scratch, _dir, mut state) = setup("resume-first");
    state.tasks.get_mut("kernel-matrix").unwrap().status = TaskStatus::WaitingForReboot;
    state.tasks.get_mut("offline-ready").unwrap().status = TaskStatus::Passed;
    let graph = Graph::core().unwrap();
    for spec in graph.specs() {
        if spec.id == "offline-ready" {
            state.tasks.get_mut("offline-ready").unwrap().fingerprint =
                Some(state.expected_fingerprint(spec));
        }
    }
    let ready = state.runnable(&graph, true);
    assert_eq!(ready.first().copied(), Some("kernel-matrix"), "{ready:?}");
}
