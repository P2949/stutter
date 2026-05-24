use std::{fs, mem, path::Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionId, ActionState, ActionWarning, ApplyResult, RestoreIdentityStatus, RollbackToken,
    SafetyClass, TaskIdentity, TaskRestoreIdentity, TuningAction, UclampRestoreRecord,
    restore_write::{RestoreSummary, RestoreWriteError, classify_restore_write_error},
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
    },
    verify_task_identity,
};

const UCLAMP_MIN_VALUE: u32 = 0;
const UCLAMP_MAX_VALUE: u32 = 1024;
const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

impl Default for SchedAttr {
    fn default() -> Self {
        Self {
            size: mem::size_of::<SchedAttr>() as u32,
            sched_policy: 0,
            sched_flags: 0,
            sched_nice: 0,
            sched_priority: 0,
            sched_runtime: 0,
            sched_deadline: 0,
            sched_period: 0,
            sched_util_min: UCLAMP_MIN_VALUE,
            sched_util_max: UCLAMP_MAX_VALUE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UclampValues {
    pub sched_util_min: Option<u32>,
    pub sched_util_max: Option<u32>,
}

impl UclampValues {
    pub fn is_empty(self) -> bool {
        self.sched_util_min.is_none() && self.sched_util_max.is_none()
    }

    fn requested_min_or(self, current: UclampCurrentValues) -> u32 {
        self.sched_util_min.unwrap_or(current.sched_util_min)
    }

    fn requested_max_or(self, current: UclampCurrentValues) -> u32 {
        self.sched_util_max.unwrap_or(current.sched_util_max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UclampCurrentValues {
    pub sched_util_min: u32,
    pub sched_util_max: u32,
}

#[derive(Debug, Clone)]
pub struct UclampPolicy {
    pub allow_uclamp_changes: bool,
    pub min_allowed_util_min: u32,
    pub max_allowed_util_min: u32,
    pub min_allowed_util_max: u32,
    pub max_allowed_util_max: u32,
    pub allow_per_task: bool,
    pub allow_cgroup: bool,
}

impl Default for UclampPolicy {
    fn default() -> Self {
        Self {
            allow_uclamp_changes: true,
            min_allowed_util_min: UCLAMP_MIN_VALUE,
            max_allowed_util_min: UCLAMP_MAX_VALUE,
            min_allowed_util_max: UCLAMP_MIN_VALUE,
            max_allowed_util_max: UCLAMP_MAX_VALUE,
            allow_per_task: true,
            allow_cgroup: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UclampAction {
    pub targets: Vec<TaskIdentity>,
    pub values: UclampValues,
}

pub(crate) struct UclampRollbackHandler;

impl RollbackHandler for UclampRollbackHandler {
    fn id(&self) -> &'static str {
        "uclamp-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("uclamp rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("uclamp rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::UclampRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("uclamp rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "uclamp-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::UclampRestore { records } = token else {
            anyhow::bail!("uclamp rollback handler does not support {token:?}");
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
                    log::debug!("uclamp restore skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "uclamp restore identity mismatch for tid={}: {}",
                        tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "uclamp restore running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            let requested = UclampCurrentValues {
                sched_util_min: record.original_util_min,
                sched_util_max: record.original_util_max,
            };

            match set_task_uclamp(tid, requested) {
                Ok(_) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(Path::new("/proc"), tid, e) {
                    RestoreWriteError::MissingTask => {
                        skipped_dead += 1;
                        log::debug!("uclamp restore skipped: task tid={} is dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(e)
                    | RestoreWriteError::InvalidValue(e)
                    | RestoreWriteError::Io(e) => {
                        errors += 1;
                        let msg = format!(
                            "failed to restore uclamp min={} max={} for tid={}: {}",
                            record.original_util_min, record.original_util_max, tid, e
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
struct UclampTargetSnapshot {
    tid: u32,
    process_pid: Option<u32>,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
    exe: Option<std::path::PathBuf>,
    current: UclampCurrentValues,
}

impl UclampAction {
    pub fn preflight_with_policy(
        &self,
        policy: &UclampPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    fn preflight_at(
        &self,
        proc_root: &Path,
        policy: &UclampPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.collect_target_snapshots_at(proc_root, policy)
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .flat_map(|(_, warnings)| warnings)
                    .collect()
            })
    }

    fn dry_run_at(&self, proc_root: &Path, policy: &UclampPolicy) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if requested_values_differ(self.values, snapshot.current) {
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

    fn verify_at(&self, proc_root: &Path, policy: &UclampPolicy) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if requested_values_differ(self.values, snapshot.current) {
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
        policy: &UclampPolicy,
    ) -> anyhow::Result<Vec<(UclampTargetSnapshot, Vec<ActionWarning>)>> {
        validate_policy_and_request(policy, self.values)?;

        if self.targets.is_empty() {
            anyhow::bail!("uclamp action requires at least one explicit target task");
        }

        let mut snapshots = Vec::with_capacity(self.targets.len());

        for target in &self.targets {
            let snapshot = read_target_snapshot_at(proc_root, target)
                .with_context(|| format!("failed to preflight uclamp target tid={}", target.tid))?;
            let warnings = identity_warnings(target, &snapshot);
            snapshots.push((snapshot, warnings));
        }

        Ok(snapshots)
    }
}

impl TuningAction for UclampAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "uclamp:set:min={}:max={}:targets:{}",
            optional_uclamp_value(self.values.sched_util_min),
            optional_uclamp_value(self.values.sched_util_max),
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
        format!(
            "set uclamp min={} max={} for task(s) [{}]",
            optional_uclamp_value(self.values.sched_util_min),
            optional_uclamp_value(self.values.sched_util_max),
            tids
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleMediumRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&UclampPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(Path::new("/proc"), &UclampPolicy::default())
    }

    fn apply(&self) -> ApplyResult {
        (|| {
            let policy = UclampPolicy::default();
            let snapshots = self.collect_target_snapshots_at(Path::new("/proc"), &policy)?;
            let filtered_snapshots: Vec<_> = snapshots
                .into_iter()
                .filter(|(snapshot, _)| requested_values_differ(self.values, snapshot.current))
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

                    let requested = UclampCurrentValues {
                        sched_util_min: self.values.requested_min_or(snapshot.current),
                        sched_util_max: self.values.requested_max_or(snapshot.current),
                    };

                    (
                        UclampRestoreRecord::new(
                            identity,
                            snapshot.current.sched_util_min,
                            snapshot.current.sched_util_max,
                        ),
                        requested,
                    )
                })
                .collect::<Vec<_>>();

            let tx = crate::actions::transaction::ApplyTransaction::new();
            tx.apply_planned_loop(
                records,
                |(record, requested)| {
                    set_task_uclamp(record.tid(), *requested).map_err(|e| {
                        e.context(format!("failed to set uclamp for tid={}", record.tid()))
                    })
                },
                |records| RollbackToken::UclampRestore {
                    records: records.into_iter().map(|(record, _)| record).collect(),
                },
            )
        })()
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &UclampPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::UclampRestore { records } = token else {
            anyhow::bail!("rollback token is not a uclamp restore token");
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
                        "uclamp rollback running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::Missing => {
                    summary.record_missing();
                    log::warn!("uclamp rollback skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    summary.record_identity_mismatch();
                    log::warn!(
                        "uclamp rollback skipped identity mismatch for tid={}: {}",
                        tid,
                        reason
                    );
                    continue;
                }
            }

            let requested = UclampCurrentValues {
                sched_util_min: record.original_util_min,
                sched_util_max: record.original_util_max,
            };

            match set_task_uclamp(tid, requested) {
                Ok(()) => summary.record_restored(),
                Err(err) => match classify_restore_write_error(Path::new("/proc"), tid, err) {
                    RestoreWriteError::MissingTask => {
                        summary.record_missing();
                        log::warn!("uclamp rollback skipped: task tid={} is missing/dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(err)
                    | RestoreWriteError::InvalidValue(err)
                    | RestoreWriteError::Io(err) => {
                        summary.record_failure();
                        failures.push(format!(
                            "failed to restore uclamp min={} max={} for tid={}: {err:#}",
                            record.original_util_min, record.original_util_max, tid
                        ));
                    }
                },
            }
        }

        if summary.has_failures() {
            anyhow::bail!(
                "failed to rollback uclamp after attempting all records: restored={} skipped_missing={} skipped_identity_mismatch={} failed={} errors={}",
                summary.restored,
                summary.skipped_missing,
                summary.skipped_identity_mismatch,
                summary.failed,
                failures.join("; ")
            );
        }

        Ok(())
    }
}

fn validate_policy_and_request(policy: &UclampPolicy, values: UclampValues) -> anyhow::Result<()> {
    if !policy.allow_uclamp_changes {
        anyhow::bail!("policy does not allow uclamp changes");
    }

    if !policy.allow_per_task {
        anyhow::bail!("policy does not allow per-task uclamp changes");
    }

    if values.is_empty() {
        anyhow::bail!("uclamp action requires sched_util_min, sched_util_max, or both");
    }

    if policy.min_allowed_util_min > policy.max_allowed_util_min {
        anyhow::bail!(
            "invalid uclamp policy min range {}..={}: min is greater than max",
            policy.min_allowed_util_min,
            policy.max_allowed_util_min
        );
    }

    if policy.min_allowed_util_max > policy.max_allowed_util_max {
        anyhow::bail!(
            "invalid uclamp policy max range {}..={}: min is greater than max",
            policy.min_allowed_util_max,
            policy.max_allowed_util_max
        );
    }

    if policy.max_allowed_util_min > UCLAMP_MAX_VALUE
        || policy.max_allowed_util_max > UCLAMP_MAX_VALUE
    {
        anyhow::bail!(
            "invalid uclamp policy range; uclamp values must be within {}..={}",
            UCLAMP_MIN_VALUE,
            UCLAMP_MAX_VALUE
        );
    }

    if let Some(util_min) = values.sched_util_min {
        validate_uclamp_value("sched_util_min", util_min)?;
        if !(policy.min_allowed_util_min..=policy.max_allowed_util_min).contains(&util_min) {
            anyhow::bail!(
                "requested sched_util_min {} is outside policy range {}..={}",
                util_min,
                policy.min_allowed_util_min,
                policy.max_allowed_util_min
            );
        }
    }

    if let Some(util_max) = values.sched_util_max {
        validate_uclamp_value("sched_util_max", util_max)?;
        if !(policy.min_allowed_util_max..=policy.max_allowed_util_max).contains(&util_max) {
            anyhow::bail!(
                "requested sched_util_max {} is outside policy range {}..={}",
                util_max,
                policy.min_allowed_util_max,
                policy.max_allowed_util_max
            );
        }
    }

    if let (Some(util_min), Some(util_max)) = (values.sched_util_min, values.sched_util_max)
        && util_min > util_max
    {
        anyhow::bail!(
            "requested sched_util_min {} is greater than sched_util_max {}",
            util_min,
            util_max
        );
    }

    Ok(())
}

fn validate_uclamp_value(name: &str, value: u32) -> anyhow::Result<()> {
    if !(UCLAMP_MIN_VALUE..=UCLAMP_MAX_VALUE).contains(&value) {
        anyhow::bail!(
            "requested {name} {} is outside uclamp range {}..={}",
            value,
            UCLAMP_MIN_VALUE,
            UCLAMP_MAX_VALUE
        );
    }

    Ok(())
}

fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<UclampTargetSnapshot> {
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
    let starttime_ticks = parse_stat_starttime(&stat)
        .with_context(|| format!("failed to parse starttime from {}", stat_path.display()))?;

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
    let exe = fs::read_link(proc_root.join(target.tid.to_string()).join("exe")).ok();

    let current = if proc_root == Path::new("/proc") {
        read_task_uclamp(target.tid.as_u32())
            .or_else(|_| read_task_uclamp_from_sched_at(proc_root, target.tid.as_u32()))
    } else {
        read_task_uclamp_from_sched_at(proc_root, target.tid.as_u32())
    }
    .with_context(|| format!("current uclamp is unreadable for tid={}", target.tid))?;

    Ok(UclampTargetSnapshot {
        tid: target.tid.as_u32(),
        process_pid: target.process_pid.map(|pid| pid.as_u32()),
        comm,
        starttime_ticks: Some(starttime_ticks),
        exe,
        current,
    })
}

fn identity_warnings(target: &TaskIdentity, snapshot: &UclampTargetSnapshot) -> Vec<ActionWarning> {
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

fn parse_stat_starttime(stat: &str) -> anyhow::Result<u64> {
    let close_paren = stat
        .rfind(')')
        .context("stat line does not contain closing comm parenthesis")?;
    let fields = stat[close_paren + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();

    let starttime_ticks = fields
        .get(19)
        .context("stat line missing starttime field")?
        .parse::<u64>()
        .context("invalid starttime field")?;

    Ok(starttime_ticks)
}

fn read_task_uclamp(tid: u32) -> anyhow::Result<UclampCurrentValues> {
    let mut attr = SchedAttr::default();
    let rc = unsafe {
        libc::syscall(
            libc::SYS_sched_getattr,
            tid as libc::pid_t,
            &mut attr as *mut SchedAttr,
            mem::size_of::<SchedAttr>() as u32,
            0u32,
        )
    };

    if rc != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("sched_getattr({tid}) failed"));
    }

    Ok(UclampCurrentValues {
        sched_util_min: attr.sched_util_min,
        sched_util_max: attr.sched_util_max,
    })
}

fn set_task_uclamp(tid: u32, values: UclampCurrentValues) -> anyhow::Result<()> {
    validate_uclamp_value("sched_util_min", values.sched_util_min)?;
    validate_uclamp_value("sched_util_max", values.sched_util_max)?;

    if values.sched_util_min > values.sched_util_max {
        anyhow::bail!(
            "sched_util_min {} is greater than sched_util_max {}",
            values.sched_util_min,
            values.sched_util_max
        );
    }

    let mut attr = SchedAttr {
        sched_flags: SCHED_FLAG_KEEP_POLICY
            | SCHED_FLAG_KEEP_PARAMS
            | SCHED_FLAG_UTIL_CLAMP_MIN
            | SCHED_FLAG_UTIL_CLAMP_MAX,
        sched_util_min: values.sched_util_min,
        sched_util_max: values.sched_util_max,
        ..SchedAttr::default()
    };

    let rc = unsafe {
        libc::syscall(
            libc::SYS_sched_setattr,
            tid as libc::pid_t,
            &mut attr as *mut SchedAttr,
            0u32,
        )
    };

    if rc != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "sched_setattr({}, util_min={}, util_max={}) failed",
                tid, values.sched_util_min, values.sched_util_max
            )
        });
    }

    Ok(())
}

fn read_task_uclamp_from_sched_at(
    proc_root: &Path,
    tid: u32,
) -> anyhow::Result<UclampCurrentValues> {
    let sched_path = proc_root.join(tid.to_string()).join("sched");
    let sched = fs::read_to_string(&sched_path)
        .with_context(|| format!("failed to read {}", sched_path.display()))?;
    parse_sched_uclamp(&sched).with_context(|| {
        format!(
            "failed to parse uclamp values from {}",
            sched_path.display()
        )
    })
}

fn parse_sched_uclamp(sched: &str) -> anyhow::Result<UclampCurrentValues> {
    let mut util_min = None;
    let mut util_max = None;

    for line in sched.lines() {
        if let Some(value) = sched_line_value(line, "uclamp.min") {
            util_min = Some(value);
        } else if let Some(value) = sched_line_value(line, "uclamp.max") {
            util_max = Some(value);
        }
    }

    Ok(UclampCurrentValues {
        sched_util_min: util_min.context("missing uclamp.min")?,
        sched_util_max: util_max.context("missing uclamp.max")?,
    })
}

fn sched_line_value(line: &str, key: &str) -> Option<u32> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with(key) {
        return None;
    }

    let (_, value) = trimmed.split_once(':')?;
    value.trim().parse::<u32>().ok()
}

fn requested_values_differ(values: UclampValues, current: UclampCurrentValues) -> bool {
    values
        .sched_util_min
        .is_some_and(|requested| requested != current.sched_util_min)
        || values
            .sched_util_max
            .is_some_and(|requested| requested != current.sched_util_max)
}

fn optional_uclamp_value(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "keep".to_owned())
}
#[cfg(test)]
#[path = "uclamp/tests.rs"]
mod tests;
