use std::{fs, path::Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionId, ActionState, ActionWarning, ApplyResult, IoPrioRestoreRecord, RestoreIdentityStatus,
    RollbackToken, SafetyClass, TaskIdentity, TaskRestoreIdentity, TuningAction,
    restore_write::{RestoreSummary, RestoreWriteError, classify_restore_write_error},
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
    },
    verify_task_identity,
};

const IOPRIO_CLASS_SHIFT: i32 = 13;
const IOPRIO_PRIO_MASK: i32 = (1 << IOPRIO_CLASS_SHIFT) - 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoPrioClass {
    None,
    Realtime,
    BestEffort,
    Idle,
}

impl IoPrioClass {
    fn linux_class(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Realtime => 1,
            Self::BestEffort => 2,
            Self::Idle => 3,
        }
    }

    fn from_linux_class(class: i32) -> anyhow::Result<Self> {
        match class {
            0 => Ok(Self::None),
            1 => Ok(Self::Realtime),
            2 => Ok(Self::BestEffort),
            3 => Ok(Self::Idle),
            other => anyhow::bail!("unsupported Linux I/O priority class {other}"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Realtime => "realtime",
            Self::BestEffort => "best-effort",
            Self::Idle => "idle",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct IoPrioValue {
    pub class: IoPrioClass,
    pub level: Option<u8>,
}

impl IoPrioValue {
    pub fn best_effort(level: u8) -> Self {
        Self {
            class: IoPrioClass::BestEffort,
            level: Some(level),
        }
    }

    pub fn realtime(level: u8) -> Self {
        Self {
            class: IoPrioClass::Realtime,
            level: Some(level),
        }
    }

    pub fn idle() -> Self {
        Self {
            class: IoPrioClass::Idle,
            level: None,
        }
    }

    pub fn none() -> Self {
        Self {
            class: IoPrioClass::None,
            level: None,
        }
    }

    pub fn encode(self) -> anyhow::Result<i32> {
        validate_ioprio_value(self)?;
        Ok((self.class.linux_class() << IOPRIO_CLASS_SHIFT) | i32::from(self.level.unwrap_or(0)))
    }

    pub fn decode(encoded: i32) -> anyhow::Result<Self> {
        if encoded < 0 {
            anyhow::bail!("negative encoded I/O priority {encoded}");
        }

        let class = IoPrioClass::from_linux_class(encoded >> IOPRIO_CLASS_SHIFT)?;
        let data = (encoded & IOPRIO_PRIO_MASK) as u8;

        let level = match class {
            IoPrioClass::BestEffort | IoPrioClass::Realtime => Some(data),
            IoPrioClass::None | IoPrioClass::Idle => None,
        };

        let value = Self { class, level };
        validate_ioprio_value(value)?;
        Ok(value)
    }

    pub fn label(self) -> String {
        match self.level {
            Some(level) => format!("{}:{level}", self.class.label()),
            None => self.class.label().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoPrioPolicy {
    pub allow_ioprio_changes: bool,
    pub allow_realtime_class: bool,
    pub allow_none_class: bool,
    pub max_best_effort_level: u8,
    pub require_strong_block_io_evidence: bool,
    pub strong_block_io_evidence: bool,
}

impl Default for IoPrioPolicy {
    fn default() -> Self {
        Self {
            allow_ioprio_changes: false,
            allow_realtime_class: false,
            allow_none_class: false,
            max_best_effort_level: 7,
            require_strong_block_io_evidence: true,
            strong_block_io_evidence: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoPrioAction {
    pub targets: Vec<TaskIdentity>,
    pub ioprio: IoPrioValue,
    pub policy: IoPrioPolicy,
}

pub(crate) struct IoPrioRollbackHandler;

impl RollbackHandler for IoPrioRollbackHandler {
    fn id(&self) -> &'static str {
        "ioprio-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("I/O priority rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("I/O priority rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_ioprio_restore().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("I/O priority rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "ioprio-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let Some(records) = token.as_ioprio_restore() else {
            anyhow::bail!("I/O priority rollback handler does not support {token:?}");
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
                    log::debug!("ioprio restore skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "ioprio restore identity mismatch for tid={}: {}",
                        tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "ioprio restore running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            match set_task_ioprio(tid, record.original_ioprio) {
                Ok(_) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(Path::new("/proc"), tid, e) {
                    RestoreWriteError::MissingTask => {
                        skipped_dead += 1;
                        log::debug!("ioprio restore skipped: task tid={} is dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(e)
                    | RestoreWriteError::InvalidValue(e)
                    | RestoreWriteError::Io(e) => {
                        errors += 1;
                        let msg = format!(
                            "failed to restore original I/O priority={} for tid={}: {}",
                            record.original_ioprio, tid, e
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
struct IoPrioTargetSnapshot {
    tid: u32,
    process_pid: Option<u32>,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
    exe: Option<std::path::PathBuf>,
    current_ioprio: i32,
    current_value: IoPrioValue,
}

impl IoPrioAction {
    pub fn preflight_with_policy(
        &self,
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    fn preflight_at(
        &self,
        proc_root: &Path,
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.collect_target_snapshots_at(proc_root, policy)
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .flat_map(|(_, warnings)| warnings)
                    .collect()
            })
    }

    fn dry_run_at(&self, proc_root: &Path, policy: &IoPrioPolicy) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let requested = self.ioprio.encode()?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if snapshot.current_ioprio != requested {
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

    fn verify_at(&self, proc_root: &Path, policy: &IoPrioPolicy) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let requested = self.ioprio.encode()?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if snapshot.current_ioprio != requested {
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
        policy: &IoPrioPolicy,
    ) -> anyhow::Result<Vec<(IoPrioTargetSnapshot, Vec<ActionWarning>)>> {
        validate_policy_and_request(policy, self.ioprio)?;

        if self.targets.is_empty() {
            anyhow::bail!("ioprio action requires at least one explicit target task");
        }

        let mut snapshots = Vec::with_capacity(self.targets.len());

        for target in &self.targets {
            let snapshot = read_target_snapshot_at(proc_root, target)
                .with_context(|| format!("failed to preflight ioprio target tid={}", target.tid))?;
            let warnings = identity_warnings(target, &snapshot);
            snapshots.push((snapshot, warnings));
        }

        Ok(snapshots)
    }
}

impl TuningAction for IoPrioAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "ioprio:set:{}:targets:{}",
            self.ioprio.label(),
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
            "set I/O priority {} for task(s) [{}]",
            self.ioprio.label(),
            tids
        )
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
            let requested = self.ioprio.encode()?;
            let filtered_snapshots: Vec<_> = snapshots
                .into_iter()
                .filter(|(snapshot, _)| snapshot.current_ioprio != requested)
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

                    IoPrioRestoreRecord::new(identity, snapshot.current_ioprio)
                })
                .collect::<Vec<_>>();

            let tx = crate::actions::transaction::ApplyTransaction::new();
            tx.apply_planned_loop(
                records,
                |record| {
                    set_task_ioprio(record.tid(), requested).map_err(|e| {
                        e.context(format!(
                            "failed to set I/O priority for tid={}",
                            record.tid()
                        ))
                    })
                },
                |records| RollbackToken::IoPrioRestore { records },
            )
        })()
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some(records) = token.as_ioprio_restore() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("ioprio-restore"),
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
                        "ioprio rollback running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::Missing => {
                    summary.record_missing();
                    log::warn!("ioprio rollback skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    summary.record_identity_mismatch();
                    log::warn!(
                        "ioprio rollback skipped identity mismatch for tid={}: {}",
                        tid,
                        reason
                    );
                    continue;
                }
            }

            match set_task_ioprio(tid, record.original_ioprio) {
                Ok(()) => summary.record_restored(),
                Err(err) => match classify_restore_write_error(Path::new("/proc"), tid, err) {
                    RestoreWriteError::MissingTask => {
                        summary.record_missing();
                        log::warn!("ioprio rollback skipped: task tid={} is missing/dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(err)
                    | RestoreWriteError::InvalidValue(err)
                    | RestoreWriteError::Io(err) => {
                        summary.record_failure();
                        failures.push(format!(
                            "failed to restore original I/O priority={} for tid={}: {err:#}",
                            record.original_ioprio, tid
                        ));
                    }
                },
            }
        }

        if summary.has_failures() {
            anyhow::bail!(
                "failed to rollback I/O priority after attempting all records: restored={} skipped_missing={} skipped_identity_mismatch={} failed={} errors={}",
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

fn validate_policy_and_request(
    policy: &IoPrioPolicy,
    requested: IoPrioValue,
) -> anyhow::Result<()> {
    if !policy.allow_ioprio_changes {
        anyhow::bail!("policy does not allow I/O priority changes");
    }

    if policy.require_strong_block_io_evidence && !policy.strong_block_io_evidence {
        anyhow::bail!(
            "strong block I/O evidence is required before changing I/O priority; current advisor policy is investigate-first"
        );
    }

    validate_ioprio_value(requested)?;

    match requested.class {
        IoPrioClass::Realtime if !policy.allow_realtime_class => {
            anyhow::bail!("policy does not allow realtime I/O priority class")
        }
        IoPrioClass::None if !policy.allow_none_class => {
            anyhow::bail!("policy does not allow resetting I/O priority to class none")
        }
        IoPrioClass::BestEffort => {
            let level = requested.level.unwrap_or(4);
            if level > policy.max_best_effort_level {
                anyhow::bail!(
                    "requested best-effort I/O priority level {} exceeds policy maximum {}",
                    level,
                    policy.max_best_effort_level
                );
            }
        }
        IoPrioClass::Idle | IoPrioClass::Realtime | IoPrioClass::None => {}
    }

    Ok(())
}

fn validate_ioprio_value(value: IoPrioValue) -> anyhow::Result<()> {
    match value.class {
        IoPrioClass::None | IoPrioClass::Idle => {
            if value.level.is_some() {
                anyhow::bail!(
                    "I/O priority class {} must not specify a level",
                    value.class.label()
                );
            }
        }
        IoPrioClass::BestEffort | IoPrioClass::Realtime => {
            let Some(level) = value.level else {
                anyhow::bail!(
                    "I/O priority class {} requires level 0..=7",
                    value.class.label()
                );
            };

            if level > 7 {
                anyhow::bail!(
                    "I/O priority class {} level {} is outside range 0..=7",
                    value.class.label(),
                    level
                );
            }
        }
    }

    Ok(())
}

fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<IoPrioTargetSnapshot> {
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

    let current_ioprio = read_task_ioprio(target.tid.as_u32())
        .with_context(|| format!("current I/O priority is unreadable for tid={}", target.tid))?;
    let current_value = IoPrioValue::decode(current_ioprio).with_context(|| {
        format!(
            "current I/O priority value is invalid for tid={}",
            target.tid
        )
    })?;

    Ok(IoPrioTargetSnapshot {
        tid: target.tid.as_u32(),
        process_pid: target.process_pid.map(|pid| pid.as_u32()),
        comm,
        starttime_ticks: Some(starttime_ticks),
        exe,
        current_ioprio,
        current_value,
    })
}

fn identity_warnings(target: &TaskIdentity, snapshot: &IoPrioTargetSnapshot) -> Vec<ActionWarning> {
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

pub(crate) fn read_task_ioprio(tid: u32) -> anyhow::Result<i32> {
    crate::actions::syscalls::ioprio_get_process(tid)
        .with_context(|| format!("ioprio_get(IOPRIO_WHO_PROCESS, {tid}) failed"))
}

pub(crate) fn set_task_ioprio(tid: u32, encoded_ioprio: i32) -> anyhow::Result<()> {
    IoPrioValue::decode(encoded_ioprio)?;

    crate::actions::syscalls::ioprio_set_process(tid, encoded_ioprio).with_context(|| {
        format!(
            "ioprio_set(IOPRIO_WHO_PROCESS, {}, {}) failed",
            tid, encoded_ioprio
        )
    })
}
#[cfg(test)]
#[path = "ioprio/tests.rs"]
mod tests;
