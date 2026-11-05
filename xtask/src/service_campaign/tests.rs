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
        host_system: "/nix/store/test-system".into(),
        rehearsal: false,
        acceptance: false,
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

/// A fake reboot executor: it records what was durable at the moment the reboot was requested.
struct Rebooting {
    dir: CampaignDir,
    seen_at_reboot: RefCell<Option<(TaskStatus, bool)>>,
}
impl StageRunner for Rebooting {
    fn execute(
        &self,
        _: Stage,
        _: &CampaignDir,
        state: &mut State,
        _: u32,
    ) -> anyhow::Result<AttemptOutcome> {
        state.reboot_pending = true;
        Ok(AttemptOutcome::WaitingForReboot)
    }
    fn after_persist(&self, stage: Stage, _: &State, _: &AttemptOutcome) -> anyhow::Result<()> {
        // The reboot would erase this process, so everything must be on disk by now.
        let on_disk = store::load(&self.dir).unwrap();
        *self.seen_at_reboot.borrow_mut() =
            Some((on_disk.tasks[stage.id()].status, on_disk.reboot_pending));
        Ok(())
    }
}

#[test]
fn the_reboot_is_requested_only_after_the_waiting_attempt_and_pending_flag_are_durable() {
    let scratch = Scratch::new();
    let dir = &scratch.0;
    let mut state = state(dir);
    for stage in [
        Stage::Preflight,
        Stage::Install,
        Stage::Discovery,
        Stage::Recovery,
    ] {
        let mut passed = attempt(&state);
        passed.fingerprint = fingerprint(&state, stage);
        append_task_attempt(dir, state.tasks.get_mut(stage.id()).unwrap(), passed).unwrap();
    }
    store::save(dir, &state).unwrap();
    let runner = Rebooting {
        dir: dir.clone(),
        seen_at_reboot: RefCell::new(None),
    };
    drive_with(dir, &runner, None, &|_| Ok(())).unwrap();
    assert_eq!(
        *runner.seen_at_reboot.borrow(),
        Some((TaskStatus::WaitingForReboot, true))
    );
    let after = store::load(dir).unwrap();
    assert_eq!(after.mode, Mode::Active);
    assert!(after.reboot_pending);
    // A resumed campaign re-enters the same stage; a waiting attempt is not a failure.
    drive_with(dir, &runner, None, &|_| Ok(())).unwrap();
    assert_eq!(
        store::load(dir).unwrap().tasks[Stage::Reboots.id()]
            .attempts
            .len(),
        2
    );
}

#[test]
fn the_campaign_owns_its_binaries_and_a_relinked_cargo_output_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new();
    let dir = &scratch.0;
    let mut state = state(dir);
    state.campaign_id = "service-pinning-test".into();
    let source = dir.0.join("cargo-output");
    fs::create_dir_all(&source).unwrap();
    let (stutter, xtask) = (source.join("stutter"), source.join("xtask"));
    fs::write(&stutter, b"stutter build A").unwrap();
    fs::write(&xtask, b"xtask build A").unwrap();
    state.stutter_sha256 = stutter_core::hash::sha256_hex(b"stutter build A");
    state.xtask_sha256 = stutter_core::hash::sha256_hex(b"xtask build A");
    let binaries = crate::validation_campaign::repository::Binaries {
        stutter: stutter.clone(),
        xtask: xtask.clone(),
    };
    super::driver::snapshot_binaries(&state, &binaries).unwrap();
    let pinned = state.pinned_stutter();
    assert_eq!(fs::read(&pinned).unwrap(), b"stutter build A");
    assert_eq!(
        fs::metadata(&pinned).unwrap().permissions().mode() & 0o777,
        0o555
    );
    // A later build relinks the working output: the campaign's bytes stay as they were.
    fs::write(&stutter, b"stutter build B").unwrap();
    assert_eq!(fs::read(&pinned).unwrap(), b"stutter build A");
    assert_eq!(
        crate::validation_campaign::repository::file_sha256(&pinned).unwrap(),
        state.stutter_sha256
    );
    // Tampering with the pinned copy itself is what the integrity check would see.
    fs::set_permissions(&pinned, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&pinned, b"tampered").unwrap();
    assert_ne!(
        crate::validation_campaign::repository::file_sha256(&pinned).unwrap(),
        state.stutter_sha256
    );
    // Snapshotting a binary that is not the validated one is refused.
    state.stutter_sha256 = "0".repeat(64);
    assert!(super::driver::snapshot_binaries(&state, &binaries).is_err());
}

#[test]
fn every_stage_that_drives_contention_is_granted_the_typed_burst() {
    use stutter_validation_runner::model::RunnerAction;
    let scratch = Scratch::new();
    let state = state(&scratch.0);
    let target = super::runner::target(&state);
    for stage in [
        Stage::Discovery,
        Stage::Recovery,
        Stage::Reboots,
        Stage::Soak,
    ] {
        assert!(
            target.grants.iter().any(|grant| grant.task_id == stage.id()
                && grant.action == RunnerAction::ExecuteSchedulerBurst),
            "{} needs the typed burst",
            stage.id()
        );
    }
    // Preflight, Install and Evidence never start contention.
    for stage in [Stage::Preflight, Stage::Install, Stage::Evidence] {
        assert!(
            !target.grants.iter().any(|grant| grant.task_id == stage.id()
                && grant.action == RunnerAction::ExecuteSchedulerBurst)
        );
    }
}
