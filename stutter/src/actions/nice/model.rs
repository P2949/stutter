use std::{fs, path::Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, ApplyResult, NiceRestoreRecord,
    RestoreIdentityStatus, RollbackToken, SafetyClass, TaskIdentity, TaskRestoreIdentity,
    TuningAction,
    restore_write::{RestoreSummary, RestoreWriteError, classify_restore_write_error},
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
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
        Err(ActionBoundaryError::missing_explicit_rollback_token(self.id(), "nice-restore").into())
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(ActionBoundaryError::missing_explicit_rollback_token(self.id(), "nice-restore").into())
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_nice_restore().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "nice-restore",
                token.kind(),
            )
            .into());
        }
        Ok(token_dry_run_preview(self.id(), token, "nice-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let Some(records) = token.as_nice_restore() else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "nice-restore",
                token.kind(),
            )
            .into());
        };

        let mut restored = 0;
        let mut skipped_dead = 0;
        let mut skipped_identity_mismatch = 0;
        let mut legacy_unverified = 0;
        let mut errors = 0;
        let mut messages = Vec::new();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid.as_u32();
            let status = verify_task_identity(Path::new("/proc"), &identity);
            match status {
                RestoreIdentityStatus::Missing => {
                    skipped_dead += 1;
                    log::debug!("nice restore skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!("nice restore identity mismatch for tid={}: {}", tid, reason);
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "nice restore running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            match set_task_nice(tid, record.original_nice) {
                Ok(_) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(Path::new("/proc"), tid, e) {
                    RestoreWriteError::MissingTask => {
                        skipped_dead += 1;
                        log::debug!("nice restore skipped: task tid={} is dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(e)
                    | RestoreWriteError::InvalidValue(e)
                    | RestoreWriteError::Io(e) => {
                        errors += 1;
                        let msg = format!(
                            "failed to restore original nice={} for tid={}: {}",
                            record.original_nice, tid, e
                        );
                        log::error!("{}", msg);
                        messages.push(msg);
                    }
                },
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
pub(super) struct NiceTargetSnapshot {
    pub(super) tid: u32,
    pub(super) process_pid: Option<u32>,
    pub(super) current_nice: i32,
    pub(super) comm: Option<String>,
    pub(super) starttime_ticks: Option<u64>,
    pub(super) exe: Option<std::path::PathBuf>,
}

impl NiceAction {
    pub fn preflight_with_policy(&self, policy: &NicePolicy) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    pub(super) fn preflight_at(
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

    pub(super) fn dry_run_at(
        &self,
        proc_root: &Path,
        policy: &NicePolicy,
    ) -> anyhow::Result<ActionState> {
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

    pub(super) fn verify_at(
        &self,
        proc_root: &Path,
        policy: &NicePolicy,
    ) -> anyhow::Result<ActionState> {
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

    pub(super) fn collect_target_snapshots_at(
        &self,
        proc_root: &Path,
        policy: &NicePolicy,
    ) -> anyhow::Result<Vec<(NiceTargetSnapshot, Vec<ActionWarning>)>> {
        validate_policy_and_request(policy, self.nice)?;

        if self.targets.is_empty() {
            return Err(ActionBoundaryError::MissingExplicitTargets {
                action_kind: "nice",
            }
            .into());
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
        (|| {
            let snapshots = self.collect_target_snapshots_at(Path::new("/proc"), &self.policy)?;
            let filtered_snapshots: Vec<_> = snapshots
                .into_iter()
                .filter(|(snapshot, _)| snapshot.current_nice != self.nice)
                .collect();

            let records = filtered_snapshots
                .into_iter()
                .map(|(snapshot, _)| {
                    let identity = TaskRestoreIdentity::observed(
                        snapshot.tid,
                        snapshot.process_pid,
                        snapshot.comm.clone(),
                        snapshot.starttime_ticks,
                        snapshot.exe.clone(),
                    );

                    NiceRestoreRecord::new(identity, snapshot.current_nice)
                })
                .collect::<Vec<_>>();

            let tx = crate::actions::transaction::ApplyTransaction::new();
            tx.apply_planned_loop(
                records,
                |record| {
                    set_task_nice(record.tid(), self.nice).map_err(|e| {
                        e.context(format!("failed to set nice for tid={}", record.tid()))
                    })
                },
                |records| RollbackToken::NiceRestore { records },
            )
        })()
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some(records) = token.as_nice_restore() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("nice-restore"),
            )
            .into());
        };

        let mut summary = RestoreSummary::default();
        let mut failures = Vec::new();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid.as_u32();
            match verify_task_identity(Path::new("/proc"), &identity) {
                RestoreIdentityStatus::SameTask => {}
                RestoreIdentityStatus::UnknownLegacy => {
                    log::warn!(
                        "nice rollback running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::Missing => {
                    summary.record_missing();
                    log::warn!("nice rollback skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    summary.record_identity_mismatch();
                    log::warn!(
                        "nice rollback skipped identity mismatch for tid={}: {}",
                        tid,
                        reason
                    );
                    continue;
                }
            }

            match set_task_nice(tid, record.original_nice) {
                Ok(()) => summary.record_restored(),
                Err(err) => match classify_restore_write_error(Path::new("/proc"), tid, err) {
                    RestoreWriteError::MissingTask => {
                        summary.record_missing();
                        log::warn!("nice rollback skipped: task tid={} is missing/dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(err)
                    | RestoreWriteError::InvalidValue(err)
                    | RestoreWriteError::Io(err) => {
                        summary.record_failure();
                        failures.push(format!(
                            "failed to restore original nice={} for tid={}: {err:#}",
                            record.original_nice, tid
                        ));
                    }
                },
            }
        }

        if summary.has_failures() {
            return Err(ActionBoundaryError::restore_failed(
                "nice",
                format!(
                    "failed to rollback nice after attempting all records: restored={} skipped_missing={} skipped_identity_mismatch={} failed={} errors={}",
                    summary.restored,
                    summary.skipped_missing,
                    summary.skipped_identity_mismatch,
                    summary.failed,
                    failures.join("; ")
                ),
            )
            .into());
        }

        Ok(())
    }
}

fn validate_policy_and_request(policy: &NicePolicy, requested_nice: i32) -> anyhow::Result<()> {
    if !policy.allow_nice_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "nice",
            requirement: "allow_nice_changes",
        }
        .into());
    }

    if policy.min_nice < LINUX_MIN_NICE || policy.max_nice > LINUX_MAX_NICE {
        return Err(ActionBoundaryError::InvalidPolicy {
            action_kind: "nice",
            reason: format!(
                "invalid nice policy range {}..={}; Linux nice range is {}..={}",
                policy.min_nice, policy.max_nice, LINUX_MIN_NICE, LINUX_MAX_NICE
            ),
        }
        .into());
    }

    if policy.min_nice > policy.max_nice {
        return Err(ActionBoundaryError::InvalidPolicy {
            action_kind: "nice",
            reason: format!(
                "invalid nice policy range {}..={}: min is greater than max",
                policy.min_nice, policy.max_nice
            ),
        }
        .into());
    }

    if !(LINUX_MIN_NICE..=LINUX_MAX_NICE).contains(&requested_nice) {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "nice",
            field: "nice".to_owned(),
            reason: format!(
                "requested nice {} is outside Linux nice range {}..={}",
                requested_nice, LINUX_MIN_NICE, LINUX_MAX_NICE
            ),
        }
        .into());
    }

    if !(policy.min_nice..=policy.max_nice).contains(&requested_nice) {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "nice",
            field: "nice".to_owned(),
            reason: format!(
                "requested nice {} is outside policy range {}..={}",
                requested_nice, policy.min_nice, policy.max_nice
            ),
        }
        .into());
    }

    Ok(())
}

pub(super) fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<NiceTargetSnapshot> {
    if target.tid == 0 {
        return Err(ActionBoundaryError::InvalidTargetTid {
            action_kind: "nice",
            tid: target.tid.as_u32(),
        }
        .into());
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
        return Err(ActionBoundaryError::TargetIdentityMismatch {
            action_kind: "nice",
            tid: target.tid.as_u32(),
            expected_starttime,
            actual_starttime: starttime_ticks,
        }
        .into());
    }

    let comm_path = proc_root.join(target.tid.to_string()).join("comm");
    let comm = fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty());
    let exe = fs::read_link(proc_root.join(target.tid.to_string()).join("exe")).ok();

    Ok(NiceTargetSnapshot {
        tid: target.tid.as_u32(),
        process_pid: target.process_pid.map(|pid| pid.as_u32()),
        current_nice,
        comm,
        starttime_ticks: Some(starttime_ticks),
        exe,
    })
}

pub(crate) fn read_task_nice(tid: u32) -> anyhow::Result<i32> {
    if tid == 0 {
        return Err(ActionBoundaryError::InvalidTargetTid {
            action_kind: "nice",
            tid,
        }
        .into());
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

pub(super) fn parse_stat_nice_and_starttime(stat: &str) -> anyhow::Result<(i32, u64)> {
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
    crate::actions::syscalls::setpriority_process(tid, nice)
        .with_context(|| format!("setpriority(PRIO_PROCESS, {tid}, {nice}) failed"))
}
