use std::fs;

use super::*;
use crate::{
    actions::{
        CgroupRestoreRecord, CpuPowerRestoreRecord, GpuPowerRestoreRecord, IoPrioRestoreRecord,
        IrqAffinityRestoreRecord, NiceRestoreRecord, TaskRestoreIdentity, UclampRestoreRecord,
        VmKnobRestoreRecord,
    },
    autotune::controller_journal::{
        ControllerJournalRecord, ControllerJournalState, write_controller_journal_applied,
        write_controller_journal_applying, write_controller_journal_clean,
        write_controller_journal_record,
    },
};

#[derive(Default)]
struct FakeRollbackExecutor {
    calls: usize,
    fail: bool,
    affected_tasks: usize,
}

impl StartupRecoveryRollbackExecutor for FakeRollbackExecutor {
    fn rollback(
        &mut self,
        _token: &RollbackToken,
    ) -> anyhow::Result<StartupRecoveryRollbackSummary> {
        self.calls += 1;

        if self.fail {
            anyhow::bail!("intentional recovery rollback failure");
        }

        Ok(StartupRecoveryRollbackSummary {
            affected_tasks: self.affected_tasks,
            message: format!("fake restored={}", self.affected_tasks),
        })
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-startup-recovery-test-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn rollback_token() -> RollbackToken {
    RollbackToken::CpuAffinityRestoreFile {
        path: crate::affinity::default_restore_path(),
        affected_tasks: 31,
    }
}

fn config_for_dir(dir: &Path, rollback_on_crash_recovery: bool) -> StartupRecoveryConfig {
    StartupRecoveryConfig {
        rollback_on_crash_recovery,
        journal_path: dir.join("controller_journal.json"),
        audit_path: dir.join("audit.jsonl"),
        history_path: dir.join("history.jsonl"),
        state_snapshot_path: dir.join("daemon_state.json"),
    }
}

fn read_daemon_state_snapshot(path: &Path) -> DaemonState {
    crate::daemon::state::load_daemon_state(path).unwrap()
}

mod journal_read;

mod restore;
