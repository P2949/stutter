use std::{collections::BTreeSet, path::Path, time::Duration};

use crate::{
    actions::{ActionState, TuningAction},
    process_tree::{TargetSnapshot, TargetSnapshotInput, TaskClass, TaskInfo, target_snapshot},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WashoutWindowConfig {
    pub washout_seconds: u64,
    pub verify_interval_ms: u64,
}

impl Default for WashoutWindowConfig {
    fn default() -> Self {
        Self {
            washout_seconds: 10,
            verify_interval_ms: 1_000,
        }
    }
}

impl WashoutWindowConfig {
    pub fn washout_duration(&self) -> Duration {
        Duration::from_secs(self.washout_seconds)
    }

    pub fn verify_interval(&self) -> Duration {
        Duration::from_millis(self.verify_interval_ms)
    }

    pub fn washout_ms(&self) -> u64 {
        self.washout_seconds.saturating_mul(1_000)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WashoutTargetSnapshot {
    pub target_present: bool,
    pub root_pid: u32,
    pub active_target_count: usize,
    pub identities: BTreeSet<WashoutTaskIdentity>,
}

impl WashoutTargetSnapshot {
    pub fn absent(root_pid: u32) -> Self {
        Self {
            target_present: false,
            root_pid,
            active_target_count: 0,
            identities: BTreeSet::new(),
        }
    }

    pub fn from_target_snapshot(root_pid: u32, snapshot: &TargetSnapshot) -> Self {
        let identities = snapshot
            .tasks
            .values()
            .map(WashoutTaskIdentity::from_task_info)
            .collect::<BTreeSet<_>>();

        Self {
            target_present: snapshot.process_roots.contains(&root_pid) && !identities.is_empty(),
            root_pid,
            active_target_count: identities.len(),
            identities,
        }
    }

    pub fn capture(root_pid: u32) -> Self {
        Self::capture_at(Path::new("/proc"), root_pid)
    }

    pub fn capture_at(proc_root: &Path, root_pid: u32) -> Self {
        if root_pid == 0 {
            return Self::absent(root_pid);
        }

        let tree_pids = [root_pid];
        let snapshot = target_snapshot(
            TargetSnapshotInput::default()
                .proc_root(proc_root)
                .tree_pids(&tree_pids),
        );
        Self::from_target_snapshot(root_pid, &snapshot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WashoutTaskIdentity {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub process_comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub class: TaskClass,
}

impl WashoutTaskIdentity {
    pub fn from_task_info(task: &TaskInfo) -> Self {
        Self {
            tid: task.tid,
            process_pid: task.process_pid,
            comm: task.comm.clone(),
            process_comm: task.process_comm.to_string(),
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            exe_dev: task.exe_dev,
            exe_ino: task.exe_ino,
            class: task.class,
        }
    }
}

#[derive(Clone, Debug)]
pub enum WashoutWindowStatus {
    WashingOut {
        elapsed_ms: u64,
        remaining_ms: u64,
        verify_state: ActionState,
    },
    Complete {
        elapsed_ms: u64,
        verify_state: ActionState,
    },
    Failed {
        elapsed_ms: u64,
        reasons: Vec<String>,
    },
}

impl WashoutWindowStatus {
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Failed { reasons, .. } => reasons,
            Self::WashingOut { .. } | Self::Complete { .. } => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct WashoutWindowState {
    config: WashoutWindowConfig,
    started_unix_nanos: u128,
    initial_target: WashoutTargetSnapshot,
}

impl WashoutWindowState {
    pub fn new(
        config: WashoutWindowConfig,
        started_unix_nanos: u128,
        initial_target: WashoutTargetSnapshot,
    ) -> Self {
        Self {
            config,
            started_unix_nanos,
            initial_target,
        }
    }

    pub fn config(&self) -> &WashoutWindowConfig {
        &self.config
    }

    pub fn started_unix_nanos(&self) -> u128 {
        self.started_unix_nanos
    }

    pub fn observe_verify_result(
        &self,
        now_unix_nanos: u128,
        current_target: WashoutTargetSnapshot,
        verify_result: anyhow::Result<ActionState>,
    ) -> WashoutWindowStatus {
        let elapsed_ms = self.elapsed_ms(now_unix_nanos);
        let mut reasons = Vec::new();

        if !current_target.target_present {
            reasons.push("target disappeared during washout".to_owned());
        }

        if self.initial_target.target_present
            && current_target.target_present
            && current_target.identities != self.initial_target.identities
        {
            reasons.push("target identity shifted during washout".to_owned());
        }

        let verify_state = match verify_result {
            Ok(state) => state,
            Err(err) => {
                reasons.push(format!("action verify failed during washout: {err:#}"));
                ActionState {
                    applied: false,
                    affected_tasks: 0,
                    checked_tasks: 0,
                    pending_changes: 0,
                    warnings: Vec::new(),
                }
            }
        };

        if !verify_state.applied {
            reasons
                .push("action verify reported candidate is not applied during washout".to_owned());
        }

        if !reasons.is_empty() {
            return WashoutWindowStatus::Failed {
                elapsed_ms,
                reasons,
            };
        }

        if elapsed_ms >= self.config.washout_ms() {
            WashoutWindowStatus::Complete {
                elapsed_ms,
                verify_state,
            }
        } else {
            WashoutWindowStatus::WashingOut {
                elapsed_ms,
                remaining_ms: self.config.washout_ms().saturating_sub(elapsed_ms),
                verify_state,
            }
        }
    }

    fn elapsed_ms(&self, now_unix_nanos: u128) -> u64 {
        now_unix_nanos
            .saturating_sub(self.started_unix_nanos)
            .checked_div(1_000_000)
            .unwrap_or(0)
            .min(u64::MAX as u128) as u64
    }
}

pub fn unix_nanos_now() -> u128 {
    crate::audit::unix_nanos_now()
}

pub async fn run_washout_for_action<A: TuningAction>(
    action: &A,
    tree_pid: u32,
    config: WashoutWindowConfig,
) -> anyhow::Result<()> {
    let started_unix_nanos = unix_nanos_now();
    let initial_target = WashoutTargetSnapshot::capture(tree_pid);

    if !initial_target.target_present {
        anyhow::bail!("target disappeared during washout");
    }

    let state = WashoutWindowState::new(config, started_unix_nanos, initial_target);

    loop {
        let current_target = WashoutTargetSnapshot::capture(tree_pid);
        let status = state.observe_verify_result(unix_nanos_now(), current_target, action.verify());

        match status {
            WashoutWindowStatus::Complete { .. } => return Ok(()),
            WashoutWindowStatus::WashingOut { .. } => {
                tokio::time::sleep(state.config().verify_interval()).await;
            }
            WashoutWindowStatus::Failed { reasons, .. } => {
                anyhow::bail!("washout failed: {}", reasons.join("; "));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{actions::ActionWarning, process_tree::TaskInfo};

    fn task(tid: u32, comm: &str) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid: 42,
            process_ppid: 1,
            comm: comm.to_owned(),
            process_comm: Arc::from("Game.exe"),
            process_starttime_ticks: Some(100),
            task_starttime_ticks: Some(200 + tid as u64),
            exe_dev: Some(1),
            exe_ino: Some(2),
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        }
    }

    fn target_snapshot_with_tasks(tasks: Vec<TaskInfo>) -> WashoutTargetSnapshot {
        let identities = tasks
            .iter()
            .map(WashoutTaskIdentity::from_task_info)
            .collect::<BTreeSet<_>>();

        WashoutTargetSnapshot {
            target_present: !identities.is_empty(),
            root_pid: 42,
            active_target_count: identities.len(),
            identities,
        }
    }

    fn verify_state(applied: bool, affected_tasks: usize) -> ActionState {
        ActionState {
            applied,
            affected_tasks,
            checked_tasks: affected_tasks,
            pending_changes: 0,
            warnings: Vec::new(),
        }
    }

    fn state() -> WashoutWindowState {
        WashoutWindowState::new(
            WashoutWindowConfig::default(),
            1_000_000_000,
            target_snapshot_with_tasks(vec![task(7, "render")]),
        )
    }

    #[test]
    fn defaults_use_ten_second_washout() {
        let config = WashoutWindowConfig::default();

        assert_eq!(config.washout_seconds, 10);
        assert_eq!(config.washout_ms(), 10_000);
        assert_eq!(config.verify_interval_ms, 1_000);
    }

    #[test]
    fn washout_ignores_score_and_remains_washing_out_before_deadline() {
        let status = state().observe_verify_result(
            5_000_000_000,
            target_snapshot_with_tasks(vec![task(7, "render")]),
            Ok(verify_state(true, 31)),
        );

        match status {
            WashoutWindowStatus::WashingOut {
                elapsed_ms,
                remaining_ms,
                verify_state,
            } => {
                assert_eq!(elapsed_ms, 4_000);
                assert_eq!(remaining_ms, 6_000);
                assert!(verify_state.applied);
                assert_eq!(verify_state.affected_tasks, 31);
            }
            other => panic!("expected washing out, got {other:?}"),
        }
    }

    #[test]
    fn washout_completes_after_deadline_when_target_and_verify_are_healthy() {
        let status = state().observe_verify_result(
            11_000_000_000,
            target_snapshot_with_tasks(vec![task(7, "render")]),
            Ok(verify_state(true, 31)),
        );

        match status {
            WashoutWindowStatus::Complete {
                elapsed_ms,
                verify_state,
            } => {
                assert_eq!(elapsed_ms, 10_000);
                assert!(verify_state.applied);
            }
            other => panic!("expected complete washout, got {other:?}"),
        }
    }

    #[test]
    fn washout_fails_when_target_disappears() {
        let status = state().observe_verify_result(
            2_000_000_000,
            WashoutTargetSnapshot::absent(42),
            Ok(verify_state(true, 31)),
        );

        assert!(status.is_failed());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason == "target disappeared during washout")
        );
    }

    #[test]
    fn washout_fails_when_target_identity_shifts() {
        let status = state().observe_verify_result(
            2_000_000_000,
            target_snapshot_with_tasks(vec![task(8, "worker")]),
            Ok(verify_state(true, 31)),
        );

        assert!(status.is_failed());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason == "target identity shifted during washout")
        );
    }

    #[test]
    fn washout_fails_when_action_verify_fails() {
        let status = state().observe_verify_result(
            2_000_000_000,
            target_snapshot_with_tasks(vec![task(7, "render")]),
            Err(anyhow::anyhow!("intentional verify failure")),
        );

        assert!(status.is_failed());
        assert!(
            status
                .reasons()
                .iter()
                .any(|reason| reason.contains("action verify failed during washout"))
        );
    }

    #[test]
    fn washout_fails_when_action_verify_reports_not_applied() {
        let status = state().observe_verify_result(
            2_000_000_000,
            target_snapshot_with_tasks(vec![task(7, "render")]),
            Ok(verify_state(false, 31)),
        );

        assert!(status.is_failed());
        assert!(status.reasons().iter().any(
            |reason| reason == "action verify reported candidate is not applied during washout"
        ));
    }

    #[test]
    fn washout_preserves_verify_warnings_in_status() {
        let status = state().observe_verify_result(
            11_000_000_000,
            target_snapshot_with_tasks(vec![task(7, "render")]),
            Ok(ActionState {
                applied: true,
                affected_tasks: 31,
                checked_tasks: 31,
                pending_changes: 0,
                warnings: vec![ActionWarning {
                    message: "verify warning".to_owned(),
                }],
            }),
        );

        match status {
            WashoutWindowStatus::Complete { verify_state, .. } => {
                assert_eq!(verify_state.warnings.len(), 1);
                assert_eq!(verify_state.warnings[0].message, "verify warning");
            }
            other => panic!("expected complete washout, got {other:?}"),
        }
    }
}
