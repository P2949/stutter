use std::{fs, path::Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionId, ActionState, ActionWarning, ApplyResult, NiceRestoreRecord, PartialApplyError,
    RestoreIdentityStatus, RollbackToken, SafetyClass, TaskIdentity, TaskRestoreIdentity,
    TuningAction,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
    verify_task_identity,
};

const LINUX_MIN_NICE: i32 = -20;
const LINUX_MAX_NICE: i32 = 19;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NicePolicy {
    pub allow_nice_changes: bool,
    pub min_nice: i32,
    pub max_nice: i32,
}

impl Default for NicePolicy {
    fn default() -> Self {
        Self {
            allow_nice_changes: true,
            min_nice: LINUX_MIN_NICE,
            max_nice: LINUX_MAX_NICE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NiceAction {
    pub targets: Vec<TaskIdentity>,
    pub nice: i32,
    pub policy: NicePolicy,
}

pub(crate) struct NiceRollbackHandler;

impl RollbackHandler for NiceRollbackHandler {
    fn id(&self) -> &'static str {
        "nice-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("nice rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("nice rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::NiceRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("nice rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "nice-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::NiceRestore { records } = token else {
            anyhow::bail!("nice rollback handler does not support {token:?}");
        };

        let mut restored = 0;
        let mut skipped_dead = 0;
        let mut skipped_identity_mismatch = 0;
        let mut legacy_unverified = 0;
        let mut errors = 0;
        let mut messages = Vec::new();

        for record in records {
            let status = verify_task_identity(Path::new("/proc"), &record.identity);
            match status {
                RestoreIdentityStatus::Missing => {
                    skipped_dead += 1;
                    log::debug!(
                        "nice restore skipped: task tid={} is missing/dead",
                        record.identity.tid
                    );
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "nice restore identity mismatch for tid={}: {}",
                        record.identity.tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "nice restore running in legacy mode (unverified identity) for tid={}",
                        record.identity.tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            match set_task_nice(record.identity.tid, record.original_nice) {
                Ok(_) => {
                    restored += 1;
                }
                Err(e) => {
                    if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                        if io_err.kind() == std::io::ErrorKind::NotFound
                            || io_err.raw_os_error() == Some(3)
                        {
                            skipped_dead += 1;
                            log::debug!(
                                "nice restore skipped: task tid={} is dead",
                                record.identity.tid
                            );
                        } else {
                            errors += 1;
                            let msg = format!(
                                "failed to restore original nice={} for tid={}: {}",
                                record.original_nice, record.identity.tid, io_err
                            );
                            log::error!("{}", msg);
                            messages.push(msg);
                        }
                    } else {
                        errors += 1;
                        let msg = format!(
                            "failed to restore original nice={} for tid={}: {}",
                            record.original_nice, record.identity.tid, e
                        );
                        log::error!("{}", msg);
                        messages.push(msg);
                    }
                }
            }
        }

        Ok(RollbackResult {
            handler_id: self.id(),
            restore_path: token.restore_path().cloned().unwrap_or_default(),
            restored,
            skipped_dead,
            skipped_identity_mismatch,
            legacy_unverified,
            errors,
            messages,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NiceTargetSnapshot {
    tid: u32,
    current_nice: i32,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
}

impl NiceAction {
    pub fn preflight_with_policy(&self, policy: &NicePolicy) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    fn preflight_at(
        &self,
        proc_root: &Path,
        policy: &NicePolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.collect_target_snapshots_at(proc_root, policy)
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .flat_map(|(_, warnings)| warnings)
                    .collect()
            })
    }

    fn dry_run_at(&self, proc_root: &Path, policy: &NicePolicy) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if snapshot.current_nice != self.nice {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: false,
            affected_tasks: pending_changes,
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    fn verify_at(&self, proc_root: &Path, policy: &NicePolicy) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if snapshot.current_nice != self.nice {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: !self.targets.is_empty() && pending_changes == 0,
            affected_tasks: self.targets.len(),
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    fn collect_target_snapshots_at(
        &self,
        proc_root: &Path,
        policy: &NicePolicy,
    ) -> anyhow::Result<Vec<(NiceTargetSnapshot, Vec<ActionWarning>)>> {
        validate_policy_and_request(policy, self.nice)?;

        if self.targets.is_empty() {
            anyhow::bail!("nice action requires at least one target task");
        }

        let mut snapshots = Vec::with_capacity(self.targets.len());

        for target in &self.targets {
            let snapshot = read_target_snapshot_at(proc_root, target)
                .with_context(|| format!("failed to preflight nice target tid={}", target.tid))?;
            let warnings = identity_warnings(target, &snapshot);
            snapshots.push((snapshot, warnings));
        }

        Ok(snapshots)
    }
}

impl TuningAction for NiceAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "nice:set:{}:targets:{}",
            self.nice,
            self.targets.len()
        ))
    }

    fn describe(&self) -> String {
        let tids = self
            .targets
            .iter()
            .map(|target| target.tid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!("set nice={} for task(s) [{}]", self.nice, tids)
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleMediumRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&self.policy)
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(Path::new("/proc"), &self.policy)
    }

    fn apply(&self) -> ApplyResult {
        let res = (|| {
            let snapshots = self.collect_target_snapshots_at(Path::new("/proc"), &self.policy)?;
            let filtered_snapshots: Vec<_> = snapshots
                .into_iter()
                .filter(|(snapshot, _)| snapshot.current_nice != self.nice)
                .collect();

            let tx = crate::actions::transaction::ApplyTransaction::new();
            tx.apply_loop(
                filtered_snapshots,
                |(snapshot, _)| {
                    let identity = TaskRestoreIdentity {
                        tid: snapshot.tid,
                        comm: snapshot
                            .comm
                            .clone()
                            .unwrap_or_else(|| "unknown".to_owned()),
                        process_starttime_ticks: None,
                        task_starttime_ticks: snapshot.starttime_ticks,
                    };

                    set_task_nice(snapshot.tid, self.nice)
                        .map_err(|e| {
                            anyhow::Error::from(e)
                                .context(format!("failed to set nice for tid={}", snapshot.tid))
                        })
                        .map(|_| NiceRestoreRecord {
                            identity,
                            original_nice: snapshot.current_nice,
                        })
                },
                |records| RollbackToken::NiceRestore { records },
            )
        })();
        res
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::NiceRestore { records } = token else {
            anyhow::bail!("rollback token is not a nice restore token");
        };

        for record in records {
            set_task_nice(record.identity.tid, record.original_nice).with_context(|| {
                format!(
                    "failed to restore original nice={} for tid={}",
                    record.original_nice, record.identity.tid
                )
            })?;
        }

        Ok(())
    }
}

fn validate_policy_and_request(policy: &NicePolicy, requested_nice: i32) -> anyhow::Result<()> {
    if !policy.allow_nice_changes {
        anyhow::bail!("policy does not allow nice changes");
    }

    if policy.min_nice < LINUX_MIN_NICE || policy.max_nice > LINUX_MAX_NICE {
        anyhow::bail!(
            "invalid nice policy range {}..={}; Linux nice range is {}..={}",
            policy.min_nice,
            policy.max_nice,
            LINUX_MIN_NICE,
            LINUX_MAX_NICE
        );
    }

    if policy.min_nice > policy.max_nice {
        anyhow::bail!(
            "invalid nice policy range {}..={}: min is greater than max",
            policy.min_nice,
            policy.max_nice
        );
    }

    if !(LINUX_MIN_NICE..=LINUX_MAX_NICE).contains(&requested_nice) {
        anyhow::bail!(
            "requested nice {} is outside Linux nice range {}..={}",
            requested_nice,
            LINUX_MIN_NICE,
            LINUX_MAX_NICE
        );
    }

    if !(policy.min_nice..=policy.max_nice).contains(&requested_nice) {
        anyhow::bail!(
            "requested nice {} is outside policy range {}..={}",
            requested_nice,
            policy.min_nice,
            policy.max_nice
        );
    }

    Ok(())
}

fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<NiceTargetSnapshot> {
    if target.tid == 0 {
        anyhow::bail!("target tid must be greater than zero");
    }

    let stat_path = proc_root.join(target.tid.to_string()).join("stat");
    let stat = fs::read_to_string(&stat_path).with_context(|| {
        format!(
            "target task does not exist or stat is unreadable: {}",
            stat_path.display()
        )
    })?;
    let (current_nice, starttime_ticks) =
        parse_stat_nice_and_starttime(&stat).with_context(|| {
            format!(
                "failed to parse nice/starttime from {}",
                stat_path.display()
            )
        })?;

    if let Some(expected_starttime) = target.starttime_ticks
        && expected_starttime != starttime_ticks
    {
        anyhow::bail!(
            "target tid={} starttime mismatch: expected={} actual={}",
            target.tid,
            expected_starttime,
            starttime_ticks
        );
    }

    let comm_path = proc_root.join(target.tid.to_string()).join("comm");
    let comm = fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty());

    Ok(NiceTargetSnapshot {
        tid: target.tid,
        current_nice,
        comm,
        starttime_ticks: Some(starttime_ticks),
    })
}

pub(crate) fn read_task_nice(tid: u32) -> anyhow::Result<i32> {
    if tid == 0 {
        anyhow::bail!("target tid must be greater than zero");
    }

    let stat_path = Path::new("/proc").join(tid.to_string()).join("stat");
    let stat = fs::read_to_string(&stat_path).with_context(|| {
        format!(
            "target task does not exist or stat is unreadable: {}",
            stat_path.display()
        )
    })?;
    let (nice, _) = parse_stat_nice_and_starttime(&stat)
        .with_context(|| format!("failed to parse nice from {}", stat_path.display()))?;
    Ok(nice)
}

fn identity_warnings(target: &TaskIdentity, snapshot: &NiceTargetSnapshot) -> Vec<ActionWarning> {
    let mut warnings = Vec::new();

    if let (Some(expected_comm), Some(actual_comm)) = (&target.comm, &snapshot.comm)
        && expected_comm != actual_comm
    {
        warnings.push(ActionWarning {
            message: format!(
                "target tid={} comm changed from {:?} to {:?}; continuing because starttime matched or was not provided",
                target.tid, expected_comm, actual_comm
            ),
        });
    }

    if target.process_pid.is_none() {
        warnings.push(ActionWarning {
            message: format!(
                "target tid={} has no process_pid identity; rollback will use tid only",
                target.tid
            ),
        });
    }

    warnings
}

fn parse_stat_nice_and_starttime(stat: &str) -> anyhow::Result<(i32, u64)> {
    let close_paren = stat
        .rfind(')')
        .context("stat line does not contain closing comm parenthesis")?;
    let fields = stat[close_paren + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();

    let nice = fields
        .get(16)
        .context("stat line missing nice field")?
        .parse::<i32>()
        .context("invalid nice field")?;
    let starttime_ticks = fields
        .get(19)
        .context("stat line missing starttime field")?
        .parse::<u64>()
        .context("invalid starttime field")?;

    Ok((nice, starttime_ticks))
}

pub(crate) fn set_task_nice(tid: u32, nice: i32) -> anyhow::Result<()> {
    let rc =
        unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, nice as libc::c_int) };

    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("setpriority(PRIO_PROCESS, {tid}, {nice}) failed"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn target(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
        TaskIdentity {
            tid,
            process_pid: Some(tid),
            comm: Some(comm.to_owned()),
            starttime_ticks: Some(starttime_ticks),
        }
    }

    fn target_without_process_pid(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
        TaskIdentity {
            tid,
            process_pid: None,
            comm: Some(comm.to_owned()),
            starttime_ticks: Some(starttime_ticks),
        }
    }

    fn action_for(tid: u32, nice: i32) -> NiceAction {
        NiceAction {
            targets: vec![target(tid, "game-thread", 12345)],
            nice,
            policy: NicePolicy::default(),
        }
    }

    fn temp_proc_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-nice-action-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fake_task(proc_root: &Path, tid: u32, comm: &str, nice: i32, starttime_ticks: u64) {
        let task_dir = proc_root.join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(
            task_dir.join("stat"),
            fake_stat_line(tid, comm, nice, starttime_ticks),
        )
        .unwrap();
    }

    fn fake_stat_line(tid: u32, comm: &str, nice: i32, starttime_ticks: u64) -> String {
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = "S".to_owned();
        fields[16] = nice.to_string();
        fields[19] = starttime_ticks.to_string();

        format!("{tid} ({comm}) {}", fields.join(" "))
    }

    #[test]
    fn safety_class_is_reversible_medium_risk() {
        assert_eq!(
            action_for(42, 5).safety_class(),
            SafetyClass::ReversibleMediumRisk
        );
    }

    #[test]
    fn action_id_and_description_include_requested_nice() {
        let action = action_for(42, 7);

        assert_eq!(
            action.id(),
            ActionId::new("nice:set:7:targets:1".to_owned())
        );
        assert_eq!(action.describe(), "set nice=7 for task(s) [42]");
    }

    #[test]
    fn parses_nice_and_starttime_from_proc_stat() {
        let stat = fake_stat_line(42, "game-thread", 5, 98765);

        let parsed = parse_stat_nice_and_starttime(&stat).unwrap();

        assert_eq!(parsed, (5, 98765));
    }

    #[test]
    fn preflight_rejects_empty_targets() {
        let action = NiceAction {
            targets: Vec::new(),
            nice: 5,
            policy: NicePolicy::default(),
        };

        let err = action
            .preflight_at(&temp_proc_root("empty-targets"), &NicePolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("requires at least one target task"));
    }

    #[test]
    fn preflight_rejects_nice_outside_linux_range() {
        let action = action_for(42, 20);

        let err = action
            .preflight_at(&temp_proc_root("bad-range"), &NicePolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("outside Linux nice range"));
    }

    #[test]
    fn preflight_rejects_when_policy_disallows_nice_changes() {
        let proc_root = temp_proc_root("policy-disallow");
        write_fake_task(&proc_root, 42, "game-thread", 0, 12345);

        let policy = NicePolicy {
            allow_nice_changes: false,
            ..NicePolicy::default()
        };

        let err = action_for(42, 5)
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("policy does not allow nice changes"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_requested_nice_outside_policy_range() {
        let proc_root = temp_proc_root("policy-range");
        write_fake_task(&proc_root, 42, "game-thread", 0, 12345);

        let policy = NicePolicy {
            allow_nice_changes: true,
            min_nice: 0,
            max_nice: 10,
        };

        let err = action_for(42, -1)
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("outside policy range"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_missing_task() {
        let proc_root = temp_proc_root("missing-task");

        let err = action_for(42, 5)
            .preflight_at(&proc_root, &NicePolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("failed to preflight nice target tid=42"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_starttime_mismatch() {
        let proc_root = temp_proc_root("starttime-mismatch");
        write_fake_task(&proc_root, 42, "game-thread", 0, 99999);

        let err = action_for(42, 5)
            .preflight_at(&proc_root, &NicePolicy::default())
            .unwrap_err();

        assert!(format!("{:#}", err).contains("starttime mismatch"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_warns_on_comm_mismatch_and_missing_process_pid() {
        let proc_root = temp_proc_root("comm-warning");
        write_fake_task(&proc_root, 42, "new-comm", 0, 12345);

        let action = NiceAction {
            targets: vec![target_without_process_pid(42, "old-comm", 12345)],
            nice: 5,
            policy: NicePolicy::default(),
        };

        let warnings = action
            .preflight_at(&proc_root, &NicePolicy::default())
            .unwrap();

        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("comm changed"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("no process_pid identity"))
        );
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn dry_run_counts_pending_changes_without_mutating() {
        let proc_root = temp_proc_root("dry-run");
        write_fake_task(&proc_root, 42, "game-thread", 0, 12345);

        let state = action_for(42, 5)
            .dry_run_at(&proc_root, &NicePolicy::default())
            .unwrap();

        assert!(!state.applied);
        assert_eq!(state.checked_tasks, 1);
        assert_eq!(state.affected_tasks, 1);
        assert_eq!(state.pending_changes, 1);

        let snapshot =
            read_target_snapshot_at(&proc_root, &target(42, "game-thread", 12345)).unwrap();
        assert_eq!(snapshot.current_nice, 0);
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn dry_run_reports_zero_pending_when_already_at_requested_nice() {
        let proc_root = temp_proc_root("dry-run-noop");
        write_fake_task(&proc_root, 42, "game-thread", 5, 12345);

        let state = action_for(42, 5)
            .dry_run_at(&proc_root, &NicePolicy::default())
            .unwrap();

        assert!(!state.applied);
        assert_eq!(state.checked_tasks, 1);
        assert_eq!(state.affected_tasks, 0);
        assert_eq!(state.pending_changes, 0);
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn rollback_rejects_wrong_token_kind() {
        let action = action_for(42, 5);
        let token = RollbackToken::IoPrioRestore {
            records: Vec::new(),
        };

        let err = action.rollback(&token).unwrap_err().to_string();

        assert!(err.contains("rollback token is not a nice restore token"));
    }
}
