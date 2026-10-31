//! Campaign crash/retry tests use scripted stages; they produce no live qualification evidence.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{
    driver::{StageRunner, drive_with, fingerprint},
    model::{Mode, Stage, State},
    store,
};
use crate::validation_campaign::{
    model::{AttemptOutcome, FailureClass, TaskAttempt, TaskState, TaskStatus},
    state::{CampaignDir, append_task_attempt, unix_ms},
};

struct Scratch(CampaignDir);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = CampaignDir(std::env::temp_dir().join(format!(
            "stutter-service-state-{}-{}-{}",
            std::process::id(),
            unix_ms(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        fs::create_dir_all(&dir.0).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0.0);
    }
}
fn state(dir: &CampaignDir) -> State {
    State {
        schema_version: 1,
        campaign_id: "service-scripted-test".into(),
        mode: Mode::Active,
        repository: dir.0.clone(),
        source_commit: "a".repeat(40),
        manifest_sha256: "b".repeat(64),
        cargo_lock_sha256: "c".repeat(64),
        stutter_sha256: "d".repeat(64),
        xtask_sha256: "e".repeat(64),
        uid: 1000,
        gid: 100,
        created_unix_ms: 1,
        kernel_release: "scripted-test-kernel".into(),
        required_reboots: 2,
        boot_ids: vec!["test-boot".into()],
        reboot_pending: false,
        tasks: Stage::ALL
            .into_iter()
            .map(|stage| (stage.id().into(), TaskState::new(stage.id())))
            .collect::<BTreeMap<_, _>>(),
    }
}
fn attempt(state: &State) -> TaskAttempt {
    TaskAttempt {
        number: 1,
        started_unix_ms: 1,
        finished_unix_ms: 2,
        outcome: AttemptOutcome::Passed,
        fingerprint: fingerprint(state, Stage::Preflight),
        artifacts: None,
        receipt: None,
        limitations: vec!["SCRIPTED TEST ONLY".into()],
    }
}
struct Failing {
    calls: RefCell<Vec<Stage>>,
}
impl StageRunner for Failing {
    fn execute(
        &self,
        stage: Stage,
        _: &CampaignDir,
        _: &mut State,
        _: u32,
    ) -> anyhow::Result<AttemptOutcome> {
        self.calls.borrow_mut().push(stage);
        Ok(AttemptOutcome::Failed {
            class: FailureClass::SoftwareDefect,
            stage: stage.id().into(),
            summary: "injected defect".into(),
        })
    }
}
#[test]
fn retained_attempt_recovers_after_state_write_loss_and_a_defect_cannot_retry_into_pass() {
    let scratch = Scratch::new();
    let dir = &scratch.0;
    let mut state = state(dir);
    state.tasks.get_mut(Stage::Preflight.id()).unwrap().status = TaskStatus::Running;
    store::save(dir, &state).unwrap();
    let completed = attempt(&state);
    let mut entry = TaskState::new(Stage::Preflight.id());
    append_task_attempt(dir, &mut entry, completed.clone()).unwrap();
    // Simulate death after the immutable attempt but before updating service-campaign.json.
    let recovered = store::load(dir).unwrap();
    assert_eq!(
        recovered.tasks[Stage::Preflight.id()].attempts,
        vec![completed]
    );
    assert_eq!(
        recovered.tasks[Stage::Preflight.id()].status,
        TaskStatus::Passed
    );
    let runner = Failing {
        calls: RefCell::new(vec![]),
    };
    drive_with(dir, &runner, None, &|_| Ok(())).unwrap();
    assert_eq!(*runner.calls.borrow(), vec![Stage::Install]);
    assert!(drive_with(dir, &runner, None, &|_| Ok(())).is_err());
    assert_eq!(*runner.calls.borrow(), vec![Stage::Install]);
    let failed = store::load(dir).unwrap();
    assert_eq!(failed.mode, Mode::Active);
    assert_eq!(failed.tasks[Stage::Install.id()].status, TaskStatus::Failed);
}
#[test]
fn an_immutable_attempt_cannot_be_replaced_and_source_change_invalidates_before_execution() {
    let scratch = Scratch::new();
    let dir = &scratch.0;
    let state = state(dir);
    store::save(dir, &state).unwrap();
    let original = attempt(&state);
    append_task_attempt(
        dir,
        &mut TaskState::new(Stage::Preflight.id()),
        original.clone(),
    )
    .unwrap();
    let mut changed = original;
    changed.finished_unix_ms += 1;
    assert!(append_task_attempt(dir, &mut TaskState::new(Stage::Preflight.id()), changed).is_err());
    let runner = Failing {
        calls: RefCell::new(vec![]),
    };
    assert!(drive_with(dir, &runner, None, &|_| anyhow::bail!("source moved")).is_err());
    assert!(runner.calls.borrow().is_empty());
    assert_eq!(store::load(dir).unwrap().mode, Mode::Invalidated);
}
