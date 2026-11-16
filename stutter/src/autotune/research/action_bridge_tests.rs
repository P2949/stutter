use super::*;

const TARGET: ChildTarget = ChildTarget {
    pid: stutter_core::ids::Pid::new(4242),
    start_ticks: 99,
    uid: 1000,
};
const NOW: u64 = 1_000;
const CPU: ResearchAction = ResearchAction::CpuAffinityProfile;
const CGROUP: ResearchAction = ResearchAction::CgroupPlacement;

#[derive(Default)]
struct Worker {
    apply: Option<WorkerApply>,
    restores: bool,
    tokens: Option<u32>,
    applied: u32,
    restored: u32,
}

impl Worker {
    fn ok() -> Self {
        Self {
            apply: Some(WorkerApply::Applied { readback_ok: true }),
            restores: true,
            tokens: Some(0),
            ..Self::default()
        }
    }
}

impl ActionWorker for Worker {
    fn apply(&mut self, _: ResearchAction, _: ChildTarget) -> WorkerApply {
        self.applied += 1;
        self.apply.unwrap()
    }
    fn restore(&mut self, _: ResearchAction, _: ChildTarget) -> bool {
        self.restored += 1;
        self.restores
    }
    fn outstanding_tokens(&mut self) -> Option<u32> {
        self.tokens
    }
}

fn fresh_scope() -> ActionChildScope {
    ActionChildScope::new([CPU, CGROUP], TARGET, NOW + 60)
}

fn ask(action: ResearchAction) -> ActionRequest {
    ActionRequest {
        action,
        target: TARGET,
        now_unix: NOW,
    }
}

fn run(
    scope: &mut ActionChildScope,
    worker: &mut Worker,
    action: ResearchAction,
    d: Decision,
) -> ActionOutcome {
    run_action(scope, worker, ask(action), &mut || d)
}

#[test]
fn a_reverted_action_is_restored_and_proven_and_the_next_may_then_run() {
    let (mut scope, mut worker) = (fresh_scope(), Worker::ok());
    assert_eq!(
        run(&mut scope, &mut worker, CPU, Decision::Revert),
        ActionOutcome::Reverted { forced: false }
    );
    assert!(scope.is_clean());
    assert_eq!((worker.applied, worker.restored), (1, 1));
    assert_eq!(
        run(&mut scope, &mut worker, CGROUP, Decision::Revert),
        ActionOutcome::Reverted { forced: false }
    );
}

#[test]
fn a_kept_action_stays_outstanding_until_the_child_settles_it() {
    let (mut scope, mut worker) = (fresh_scope(), Worker::ok());
    assert_eq!(
        run(&mut scope, &mut worker, CPU, Decision::Keep),
        ActionOutcome::Kept
    );
    assert!(!scope.is_clean());
    assert_eq!(
        run(&mut scope, &mut worker, CGROUP, Decision::Keep),
        ActionOutcome::Refused(Refusal::ActionOutstanding)
    );
    assert_eq!(
        worker.applied, 1,
        "a refused request never reaches the worker"
    );
    assert!(settle_kept(&mut scope, &mut worker, CPU, TARGET));
    assert!(scope.is_clean());
}

#[test]
fn the_scope_refuses_before_the_worker_is_ever_asked() {
    let (mut scope, mut worker) = (ActionChildScope::new([CPU], TARGET, NOW + 60), Worker::ok());
    assert_eq!(
        run(&mut scope, &mut worker, CGROUP, Decision::Revert),
        ActionOutcome::Refused(Refusal::ActionNotAuthorized)
    );
    let wrong = ActionRequest {
        target: ChildTarget { uid: 0, ..TARGET },
        ..ask(CPU)
    };
    assert_eq!(
        run_action(&mut scope, &mut worker, wrong, &mut || Decision::Keep),
        ActionOutcome::Refused(Refusal::WrongTarget)
    );
    assert_eq!(worker.applied, 0);
}

#[test]
fn a_worker_refusal_changes_nothing_and_needs_no_restore() {
    let (mut scope, mut worker) = (fresh_scope(), Worker::ok());
    worker.apply = Some(WorkerApply::Refused);
    assert_eq!(
        run(&mut scope, &mut worker, CPU, Decision::Keep),
        ActionOutcome::WorkerRefused
    );
    assert_eq!(worker.restored, 0);
    assert!(scope.is_clean());
}

#[test]
fn a_failed_readback_or_a_lost_reply_is_forced_back_and_the_child_is_never_asked_to_keep() {
    for apply in [
        WorkerApply::Applied { readback_ok: false },
        WorkerApply::Ambiguous,
    ] {
        let (mut scope, mut worker) = (fresh_scope(), Worker::ok());
        worker.apply = Some(apply);
        let mut asked = false;
        let outcome = run_action(&mut scope, &mut worker, ask(CPU), &mut || {
            asked = true;
            Decision::Keep
        });
        assert_eq!(outcome, ActionOutcome::Reverted { forced: true });
        assert!(!asked, "an unverified action cannot be kept");
        assert_eq!(worker.restored, 1);
        assert!(scope.is_clean());
    }
}

#[test]
fn a_restoration_that_is_not_proven_leaves_the_action_outstanding_and_blocks_the_next() {
    // The kernel readback does not show the original state.
    let (mut scope, mut worker) = (fresh_scope(), Worker::ok());
    worker.restores = false;
    assert_eq!(
        run(&mut scope, &mut worker, CPU, Decision::Revert),
        ActionOutcome::RestoreUnproven
    );
    assert!(!scope.is_clean());
    worker.restores = true;
    assert_eq!(
        run(&mut scope, &mut worker, CGROUP, Decision::Revert),
        ActionOutcome::Refused(Refusal::ActionOutstanding)
    );
    // The worker confirms the original state but still holds a token, or cannot say: unproven.
    for tokens in [Some(1), None] {
        let (mut scope, mut worker) = (fresh_scope(), Worker::ok());
        worker.tokens = tokens;
        assert_eq!(
            run(&mut scope, &mut worker, CPU, Decision::Revert),
            ActionOutcome::RestoreUnproven,
            "tokens {tokens:?}"
        );
    }
}

#[test]
fn a_kept_action_whose_settlement_fails_leaves_the_child_unclean() {
    let (mut scope, mut worker) = (fresh_scope(), Worker::ok());
    run(&mut scope, &mut worker, CPU, Decision::Keep);
    worker.restores = false;
    assert!(!settle_kept(&mut scope, &mut worker, CPU, TARGET));
    assert!(!scope.is_clean());
    worker.restores = true;
    assert!(settle_kept(&mut scope, &mut worker, CPU, TARGET));
    // A clean scope with a lingering worker token is still not clean.
    worker.tokens = Some(2);
    assert!(!settle_kept(&mut scope, &mut worker, CPU, TARGET));
}
