use std::{fs, path::Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionId, ActionState, ActionWarning, ApplyResult, IoPrioRestoreRecord, RestoreIdentityStatus,
    RollbackToken, SafetyClass, TaskIdentity, TaskRestoreIdentity, TuningAction,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
    },
    verify_task_identity,
};

const IOPRIO_WHO_PROCESS: libc::c_int = 1;
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
        matches!(token, RollbackToken::IoPrioRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("I/O priority rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "ioprio-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::IoPrioRestore { records } = token else {
            anyhow::bail!("I/O priority rollback handler does not support {token:?}");
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
                        "ioprio restore skipped: task tid={} is missing/dead",
                        record.identity.tid
                    );
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "ioprio restore identity mismatch for tid={}: {}",
                        record.identity.tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "ioprio restore running in legacy mode (unverified identity) for tid={}",
                        record.identity.tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            match set_task_ioprio(record.identity.tid, record.original_ioprio) {
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
                                "ioprio restore skipped: task tid={} is dead",
                                record.identity.tid
                            );
                        } else {
                            errors += 1;
                            let msg = format!(
                                "failed to restore original I/O priority={} for tid={}: {}",
                                record.original_ioprio, record.identity.tid, io_err
                            );
                            log::error!("{}", msg);
                            messages.push(msg);
                        }
                    } else {
                        errors += 1;
                        let msg = format!(
                            "failed to restore original I/O priority={} for tid={}: {}",
                            record.original_ioprio, record.identity.tid, e
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
struct IoPrioTargetSnapshot {
    tid: u32,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
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
        let res = (|| {
            let snapshots = self.collect_target_snapshots_at(Path::new("/proc"), &self.policy)?;
            let requested = self.ioprio.encode()?;
            let filtered_snapshots: Vec<_> = snapshots
                .into_iter()
                .filter(|(snapshot, _)| snapshot.current_ioprio != requested)
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

                    set_task_ioprio(snapshot.tid, requested)
                        .map_err(|e| {
                            anyhow::Error::from(e).context(format!(
                                "failed to set I/O priority for tid={}",
                                snapshot.tid
                            ))
                        })
                        .map(|_| IoPrioRestoreRecord {
                            identity,
                            original_ioprio: snapshot.current_ioprio,
                        })
                },
                |records| RollbackToken::IoPrioRestore { records },
            )
        })();
        res
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::IoPrioRestore { records } = token else {
            anyhow::bail!("rollback token is not an I/O priority restore token");
        };

        for record in records {
            set_task_ioprio(record.identity.tid, record.original_ioprio).with_context(|| {
                format!(
                    "failed to restore original I/O priority={} for tid={}",
                    record.original_ioprio, record.identity.tid
                )
            })?;
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

    let current_ioprio = read_task_ioprio(target.tid)
        .with_context(|| format!("current I/O priority is unreadable for tid={}", target.tid))?;
    let current_value = IoPrioValue::decode(current_ioprio).with_context(|| {
        format!(
            "current I/O priority value is invalid for tid={}",
            target.tid
        )
    })?;

    Ok(IoPrioTargetSnapshot {
        tid: target.tid,
        comm,
        starttime_ticks: Some(starttime_ticks),
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
    let rc = unsafe { libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, tid as libc::c_int) };

    if rc < 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("ioprio_get(IOPRIO_WHO_PROCESS, {tid}) failed"));
    }

    Ok(rc as i32)
}

pub(crate) fn set_task_ioprio(tid: u32, encoded_ioprio: i32) -> anyhow::Result<()> {
    IoPrioValue::decode(encoded_ioprio)?;

    let rc = unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            tid as libc::c_int,
            encoded_ioprio as libc::c_int,
        )
    };

    if rc != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "ioprio_set(IOPRIO_WHO_PROCESS, {}, {}) failed",
                tid, encoded_ioprio
            )
        });
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

    fn action_for(tid: u32, ioprio: IoPrioValue) -> IoPrioAction {
        IoPrioAction {
            targets: vec![target(tid, "storage-worker", 12345)],
            ioprio,
            policy: permissive_evidence_policy(),
        }
    }

    fn permissive_evidence_policy() -> IoPrioPolicy {
        IoPrioPolicy {
            allow_ioprio_changes: true,
            allow_realtime_class: false,
            allow_none_class: false,
            max_best_effort_level: 7,
            require_strong_block_io_evidence: true,
            strong_block_io_evidence: true,
        }
    }

    fn temp_proc_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-ioprio-action-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fake_task(proc_root: &Path, tid: u32, comm: &str, starttime_ticks: u64) {
        let task_dir = proc_root.join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(
            task_dir.join("stat"),
            fake_stat_line(tid, comm, starttime_ticks),
        )
        .unwrap();
    }

    fn fake_stat_line(tid: u32, comm: &str, starttime_ticks: u64) -> String {
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = "S".to_owned();
        fields[19] = starttime_ticks.to_string();

        format!("{tid} ({comm}) {}", fields.join(" "))
    }

    #[test]
    fn safety_class_is_reversible_medium_risk() {
        assert_eq!(
            action_for(42, IoPrioValue::best_effort(6)).safety_class(),
            SafetyClass::ReversibleMediumRisk
        );
    }

    #[test]
    fn action_id_and_description_include_requested_ioprio() {
        let action = action_for(42, IoPrioValue::best_effort(6));

        assert_eq!(
            action.id(),
            ActionId::new("ioprio:set:best-effort:6:targets:1".to_owned())
        );
        assert_eq!(
            action.describe(),
            "set I/O priority best-effort:6 for task(s) [42]"
        );
    }

    #[test]
    fn encodes_and_decodes_best_effort_ioprio() {
        let value = IoPrioValue::best_effort(4);
        let encoded = value.encode().unwrap();

        assert_eq!(encoded, (2 << IOPRIO_CLASS_SHIFT) | 4);
        assert_eq!(IoPrioValue::decode(encoded).unwrap(), value);
    }

    #[test]
    fn encodes_and_decodes_idle_ioprio() {
        let value = IoPrioValue::idle();
        let encoded = value.encode().unwrap();

        assert_eq!(encoded, 3 << IOPRIO_CLASS_SHIFT);
        assert_eq!(IoPrioValue::decode(encoded).unwrap(), value);
    }

    #[test]
    fn rejects_invalid_level_for_idle_class() {
        let err = IoPrioValue {
            class: IoPrioClass::Idle,
            level: Some(1),
        }
        .encode()
        .unwrap_err()
        .to_string();

        assert!(err.contains("must not specify a level"));
    }

    #[test]
    fn rejects_missing_level_for_best_effort_class() {
        let err = IoPrioValue {
            class: IoPrioClass::BestEffort,
            level: None,
        }
        .encode()
        .unwrap_err()
        .to_string();

        assert!(err.contains("requires level 0..=7"));
    }

    #[test]
    fn rejects_level_above_seven() {
        let err = IoPrioValue::best_effort(8)
            .encode()
            .unwrap_err()
            .to_string();

        assert!(err.contains("outside range 0..=7"));
    }

    #[test]
    fn parses_starttime_from_proc_stat() {
        let stat = fake_stat_line(42, "storage-worker", 98765);

        let parsed = parse_stat_starttime(&stat).unwrap();

        assert_eq!(parsed, 98765);
    }

    #[test]
    fn preflight_rejects_empty_targets() {
        let action = IoPrioAction {
            targets: Vec::new(),
            ioprio: IoPrioValue::best_effort(6),
            policy: permissive_evidence_policy(),
        };

        let err = action
            .preflight_at(
                &temp_proc_root("empty-targets"),
                &permissive_evidence_policy(),
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("requires at least one explicit target task"));
    }

    #[test]
    fn preflight_rejects_when_policy_disallows_ioprio_changes() {
        let proc_root = temp_proc_root("policy-disallow");
        write_fake_task(&proc_root, 42, "storage-worker", 12345);

        let policy = IoPrioPolicy {
            allow_ioprio_changes: false,
            strong_block_io_evidence: true,
            ..IoPrioPolicy::default()
        };

        let err = action_for(42, IoPrioValue::best_effort(6))
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("policy does not allow I/O priority changes"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_without_strong_block_io_evidence() {
        let proc_root = temp_proc_root("no-evidence");
        write_fake_task(&proc_root, 42, "storage-worker", 12345);

        let policy = IoPrioPolicy {
            allow_ioprio_changes: true,
            require_strong_block_io_evidence: true,
            strong_block_io_evidence: false,
            ..IoPrioPolicy::default()
        };

        let err = action_for(42, IoPrioValue::best_effort(6))
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("strong block I/O evidence is required"));
        assert!(err.contains("investigate-first"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_realtime_class_by_default_policy() {
        let proc_root = temp_proc_root("realtime-policy");
        write_fake_task(&proc_root, 42, "storage-worker", 12345);

        let err = action_for(
            42,
            IoPrioValue {
                class: IoPrioClass::Realtime,
                level: Some(0),
            },
        )
        .preflight_at(&proc_root, &permissive_evidence_policy())
        .unwrap_err()
        .to_string();

        assert!(err.contains("does not allow realtime I/O priority class"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_none_class_by_default_policy() {
        let proc_root = temp_proc_root("none-policy");
        write_fake_task(&proc_root, 42, "storage-worker", 12345);

        let err = action_for(42, IoPrioValue::none())
            .preflight_at(&proc_root, &permissive_evidence_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("does not allow resetting I/O priority to class none"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_best_effort_level_above_policy_maximum() {
        let proc_root = temp_proc_root("best-effort-policy");
        write_fake_task(&proc_root, 42, "storage-worker", 12345);

        let policy = IoPrioPolicy {
            max_best_effort_level: 3,
            ..permissive_evidence_policy()
        };

        let err = action_for(42, IoPrioValue::best_effort(6))
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("exceeds policy maximum"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_missing_task() {
        let proc_root = temp_proc_root("missing-task");

        let err = action_for(42, IoPrioValue::best_effort(6))
            .preflight_at(&proc_root, &permissive_evidence_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("failed to preflight ioprio target tid=42"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_starttime_mismatch_before_ioprio_read() {
        let proc_root = temp_proc_root("starttime-mismatch");
        write_fake_task(&proc_root, 42, "storage-worker", 99999);

        let err = action_for(42, IoPrioValue::best_effort(6))
            .preflight_at(&proc_root, &permissive_evidence_policy())
            .unwrap_err();

        assert!(format!("{:#}", err).contains("starttime mismatch"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn identity_warnings_report_comm_mismatch_and_missing_process_pid() {
        let snapshot = IoPrioTargetSnapshot {
            tid: 42,
            comm: Some("new-comm".to_owned()),
            starttime_ticks: Some(12345),
            current_ioprio: IoPrioValue::best_effort(4).encode().unwrap(),
            current_value: IoPrioValue::best_effort(4),
        };
        let warnings = identity_warnings(
            &target_without_process_pid(42, "old-comm", 12345),
            &snapshot,
        );

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
    }

    #[test]
    fn rollback_rejects_wrong_token_kind() {
        let action = action_for(42, IoPrioValue::best_effort(6));
        let token = RollbackToken::NiceRestore {
            records: Vec::new(),
        };

        let err = action.rollback(&token).unwrap_err().to_string();

        assert!(err.contains("rollback token is not an I/O priority restore token"));
    }

    #[test]
    fn rollback_token_reports_affected_tasks() {
        let token = RollbackToken::IoPrioRestore {
            records: vec![
                IoPrioRestoreRecord {
                    identity: TaskRestoreIdentity {
                        tid: 1,
                        comm: "test".to_owned(),
                        process_starttime_ticks: None,
                        task_starttime_ticks: None,
                    },
                    original_ioprio: IoPrioValue::best_effort(4).encode().unwrap(),
                },
                IoPrioRestoreRecord {
                    identity: TaskRestoreIdentity {
                        tid: 2,
                        comm: "test".to_owned(),
                        process_starttime_ticks: None,
                        task_starttime_ticks: None,
                    },
                    original_ioprio: IoPrioValue::idle().encode().unwrap(),
                },
            ],
        };

        assert_eq!(token.affected_tasks(), 2);
        assert!(token.restore_path().is_none());
    }
}
