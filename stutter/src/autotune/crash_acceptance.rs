//! Real process-death checkpoints around the production low-risk transaction.
//! Only a parent harness's owned sibling (or traced grandchild) tree is accepted.
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::{
    action_transaction::{JournaledActionInput, apply_journaled_action},
    controller_journal::{
        ControllerJournalActionMetadata, ControllerJournalState, journal_process_identity,
        read_controller_journal,
    },
    startup_recovery::{
        StartupRecoveryConfig, StartupRecoveryOutcome, recover_controller_journal_on_startup,
    },
};
use crate::actions::{
    ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction,
    cpu_affinity::CpuAffinityProfileAction, runner::ActionRunPolicy,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryCheckpoint {
    BeforeApply,
    AfterPreparation,
    MidApply,
    AfterApply,
    DuringVerify,
    AfterVerify,
}
impl RecoveryCheckpoint {
    pub const ALL: [Self; 6] = [
        Self::BeforeApply,
        Self::AfterPreparation,
        Self::MidApply,
        Self::AfterApply,
        Self::DuringVerify,
        Self::AfterVerify,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BeforeApply => "before_apply",
            Self::AfterPreparation => "after_preparation",
            Self::MidApply => "mid_apply",
            Self::AfterApply => "after_apply",
            Self::DuringVerify => "during_verify",
            Self::AfterVerify => "after_verify",
        }
    }
}
impl std::str::FromStr for RecoveryCheckpoint {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|phase| phase.as_str() == value)
            .with_context(|| format!("unknown recovery checkpoint {value}"))
    }
}

fn parent(pid: u32) -> anyhow::Result<u32> {
    fs::read_to_string(format!("/proc/{pid}/stat"))?
        .rsplit_once(')')
        .and_then(|(_, tail)| tail.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .context("parent identity unavailable")
}
fn ticks(pid: u32) -> anyhow::Result<u64> {
    crate::process_tree::process_starttime_at(Path::new("/proc"), pid)
        .context("process identity unavailable")
}
fn publish(out: &Path, name: &str, value: Value) -> anyhow::Result<()> {
    let temporary = out.join(format!("{name}.tmp"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, &value)?;
    file.sync_all()?;
    fs::rename(temporary, out.join(name))?;
    fs::File::open(out)?.sync_all()?;
    Ok(())
}
fn pause(
    out: &Path,
    checkpoint: RecoveryCheckpoint,
    verification_read: bool,
) -> anyhow::Result<()> {
    let record = read_controller_journal(&out.join("controller.json"))?;
    let expected = match checkpoint {
        RecoveryCheckpoint::BeforeApply => ControllerJournalState::Clean,
        RecoveryCheckpoint::AfterPreparation => ControllerJournalState::Applying,
        _ => ControllerJournalState::Applied,
    };
    ensure!(
        record.state() == expected,
        "unexpected checkpoint journal state"
    );
    ensure!(
        record.rollback_token().is_some() == (checkpoint != RecoveryCheckpoint::BeforeApply),
        "checkpoint rollback token mismatch"
    );
    publish(
        out,
        "pause-ready.json",
        json!({"controller_pid":std::process::id(),"controller_starttime_ticks":ticks(std::process::id())?,"checkpoint":checkpoint.as_str(),"journal_state":record.state().as_str(),"real_verification_read_completed":verification_read,"verification_returned_to_runner":checkpoint == RecoveryCheckpoint::AfterVerify}),
    )?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    anyhow::bail!("checkpoint was not SIGKILLed before deadline; no crash acceptance established")
}

struct CheckpointAction<'a> {
    inner: CpuAffinityProfileAction,
    out: &'a Path,
    checkpoint: RecoveryCheckpoint,
}
impl TuningAction for CheckpointAction<'_> {
    fn id(&self) -> ActionId {
        self.inner.id()
    }
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn safety_class(&self) -> SafetyClass {
        self.inner.safety_class()
    }
    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        self.inner.descriptor()
    }
    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.inner.preflight()
    }
    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.inner.dry_run()
    }
    fn apply(&self) -> crate::actions::ApplyResult {
        self.inner.apply()
    }
    fn apply_with_prepared_rollback(
        &self,
        callback: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> crate::actions::ApplyResult {
        self.inner.apply_with_prepared_rollback(&mut |token| {
            callback(token)?; // The actual runner durably journals this token.
            if self.checkpoint == RecoveryCheckpoint::AfterPreparation {
                pause(self.out, self.checkpoint, false)?;
            }
            Ok(())
        })
    }
    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.inner.rollback(token)
    }
    fn verify(&self) -> anyhow::Result<ActionState> {
        if self.checkpoint == RecoveryCheckpoint::AfterApply {
            pause(self.out, self.checkpoint, false)?;
        }
        ensure!(
            self.checkpoint != RecoveryCheckpoint::MidApply,
            "mid-apply tracer failed to kill before commit completed"
        );
        let state = self.inner.verify()?;
        ensure!(
            state.applied && state.checked_tasks == 5 && state.pending_changes == 0,
            "real kernel verification did not cover all five tasks"
        );
        if self.checkpoint == RecoveryCheckpoint::DuringVerify {
            pause(self.out, self.checkpoint, true)?;
        }
        Ok(state)
    }
}

pub struct RecoveryControllerInput<'a> {
    pub tree_pid: u32,
    pub tree_ticks: u64,
    pub cpu: u32,
    pub out: &'a Path,
    pub owner_pid: u32,
    pub owner_ticks: u64,
    pub checkpoint: RecoveryCheckpoint,
}

pub fn hold_affinity_recovery_controller(input: RecoveryControllerInput<'_>) -> anyhow::Result<()> {
    let RecoveryControllerInput {
        tree_pid,
        tree_ticks,
        cpu,
        out,
        owner_pid,
        owner_ticks,
        checkpoint,
    } = input;
    ensure!(
        ticks(tree_pid)? == tree_ticks && ticks(owner_pid)? == owner_ticks,
        "disposable target/owner identity differs"
    );
    ensure!(
        parent(tree_pid)? == owner_pid,
        "target must be the harness's direct child"
    );
    let controller_parent = parent(std::process::id())?;
    let traced = checkpoint == RecoveryCheckpoint::MidApply
        && parent(controller_parent)? == owner_pid
        && fs::read_to_string(format!("/proc/{controller_parent}/comm"))?.trim() == "strace";
    ensure!(
        controller_parent == owner_pid || traced,
        "controller must be the owned sibling or traced grandchild"
    );
    for name in [
        "restore.json",
        "controller.json",
        "actions.jsonl",
        "pause-ready.json",
        "controller-started.json",
    ] {
        ensure!(
            !out.join(name).exists(),
            "controller state must be new: {name}"
        );
    }
    let snapshot = crate::process_tree::target_snapshot(
        crate::process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    ensure!(
        snapshot.tasks.len() == 5
            && snapshot
                .tasks
                .values()
                .map(|task| task.process_id())
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 3,
        "expected three disposable processes and five tasks"
    );
    let desired = crate::affinity::CpuMask::parse(&cpu.to_string())?;
    ensure!(
        desired.is_subset_of(&crate::affinity::CpuMask::online_cpus()?),
        "selected CPU offline"
    );
    for task in snapshot.tasks.values() {
        let original = crate::affinity::read_allowed_mask(task.task_id())?;
        ensure!(
            desired.is_subset_of(&original) && desired != original,
            "no observable permitted mask change"
        );
    }
    publish(
        out,
        "controller-started.json",
        json!({"controller_pid":std::process::id(),"controller_starttime_ticks":ticks(std::process::id())?}),
    )?;
    if checkpoint == RecoveryCheckpoint::BeforeApply {
        pause(out, checkpoint, false)?;
    }
    let action = CheckpointAction {
        inner: CpuAffinityProfileAction {
            tree_pid,
            restore_path: Some(out.join("restore.json")),
            force_restore_overwrite: false,
            profile: crate::profiles::Profile {
                name: "crash-acceptance-affinity".into(),
                rules: vec![crate::profiles::ProfileRule {
                    affinity: Some(desired),
                    nice: None,
                    ionice: None,
                    match_class: vec![],
                    match_comm: vec![crate::process_tree::CompiledPattern::new("/.*/".into())?],
                }],
            },
        },
        out,
        checkpoint,
    };
    let metadata = ControllerJournalActionMetadata::default()
        .with_candidate("crash-acceptance-affinity")
        .with_target_identity(journal_process_identity(
            tree_pid,
            Some(tree_ticks),
            Some(5),
        ))
        .with_mode(crate::daemon_policy::DaemonMode::ApplyLowRisk)
        .with_safety_class(SafetyClass::ReversibleLowRisk);
    let mut policy =
        ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::AutotuneRuntime, false);
    policy.max_affected_tasks = Some(8);
    apply_journaled_action(
        &action,
        policy,
        JournaledActionInput {
            journal_path: &out.join("controller.json"),
            audit_path: &out.join("actions.jsonl"),
            experiment_id: "live-crash-acceptance",
            metadata,
        },
    )?;
    ensure!(
        checkpoint == RecoveryCheckpoint::AfterVerify,
        "unexpected controller return without SIGKILL"
    );
    pause(out, checkpoint, true)
}

pub fn recover_affinity_acceptance_controller(
    out: &Path,
    expected_tasks: usize,
) -> anyhow::Result<Value> {
    ensure!(
        expected_tasks == 0 || expected_tasks == 5,
        "unexpected recovery scope"
    );
    let outcome = recover_controller_journal_on_startup(StartupRecoveryConfig {
        rollback_on_crash_recovery: true,
        journal_path: out.join("controller.json"),
        audit_path: out.join("actions.jsonl"),
        history_path: out.join("history.jsonl"),
        state_snapshot_path: out.join("daemon-state.json"),
    })?;
    let restored = match outcome {
        StartupRecoveryOutcome::Clean if expected_tasks == 0 => 0,
        StartupRecoveryOutcome::Recovered { affected_tasks, .. }
            if affected_tasks == expected_tasks =>
        {
            affected_tasks
        }
        other => anyhow::bail!("unexpected startup recovery outcome: {other:?}"),
    };
    ensure!(
        read_controller_journal(&out.join("controller.json"))?.is_clean(),
        "recovery left outstanding journal"
    );
    Ok(
        json!({"controller_pid":std::process::id(),"restored_tasks":restored,"startup_recovered":restored > 0,"journal_clean":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoint_names_round_trip_and_unknown_is_rejected() {
        for checkpoint in RecoveryCheckpoint::ALL {
            assert_eq!(
                checkpoint.as_str().parse::<RecoveryCheckpoint>().unwrap(),
                checkpoint
            );
        }
        assert!("unknown".parse::<RecoveryCheckpoint>().is_err());
    }
}
