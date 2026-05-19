use std::{fs, mem, path::Path};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TaskIdentity, TuningAction,
    UclampRestoreRecord,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
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

        for record in records {
            set_task_uclamp(
                record.tid,
                UclampCurrentValues {
                    sched_util_min: record.original_util_min,
                    sched_util_max: record.original_util_max,
                },
            )
            .with_context(|| {
                format!(
                    "failed to restore uclamp min={} max={} for tid={}",
                    record.original_util_min, record.original_util_max, record.tid
                )
            })?;
        }

        Ok(token_restore_result(
            self.id(),
            token,
            records.len(),
            0,
            Vec::new(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UclampTargetSnapshot {
    tid: u32,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
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

    fn apply(&self) -> anyhow::Result<RollbackToken> {
        let policy = UclampPolicy::default();
        let snapshots = self.collect_target_snapshots_at(Path::new("/proc"), &policy)?;
        let mut records = Vec::new();

        for (snapshot, _) in snapshots {
            if !requested_values_differ(self.values, snapshot.current) {
                continue;
            }

            let requested = UclampCurrentValues {
                sched_util_min: self.values.requested_min_or(snapshot.current),
                sched_util_max: self.values.requested_max_or(snapshot.current),
            };

            set_task_uclamp(snapshot.tid, requested)
                .with_context(|| format!("failed to set uclamp for tid={}", snapshot.tid))?;

            records.push(UclampRestoreRecord {
                tid: snapshot.tid,
                original_util_min: snapshot.current.sched_util_min,
                original_util_max: snapshot.current.sched_util_max,
            });
        }

        Ok(RollbackToken::UclampRestore { records })
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &UclampPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::UclampRestore { records } = token else {
            anyhow::bail!("rollback token is not a uclamp restore token");
        };

        for record in records {
            set_task_uclamp(
                record.tid,
                UclampCurrentValues {
                    sched_util_min: record.original_util_min,
                    sched_util_max: record.original_util_max,
                },
            )
            .with_context(|| {
                format!(
                    "failed to restore uclamp min={} max={} for tid={}",
                    record.original_util_min, record.original_util_max, record.tid
                )
            })?;
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

    let current = if proc_root == Path::new("/proc") {
        read_task_uclamp(target.tid)
            .or_else(|_| read_task_uclamp_from_sched_at(proc_root, target.tid))
    } else {
        read_task_uclamp_from_sched_at(proc_root, target.tid)
    }
    .with_context(|| format!("current uclamp is unreadable for tid={}", target.tid))?;

    Ok(UclampTargetSnapshot {
        tid: target.tid,
        comm,
        starttime_ticks: Some(starttime_ticks),
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

    fn action_for(tid: u32, min: Option<u32>, max: Option<u32>) -> UclampAction {
        UclampAction {
            targets: vec![target(tid, "game-thread", 12345)],
            values: UclampValues {
                sched_util_min: min,
                sched_util_max: max,
            },
        }
    }

    fn temp_proc_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-uclamp-action-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fake_task(
        proc_root: &Path,
        tid: u32,
        comm: &str,
        starttime_ticks: u64,
        util_min: u32,
        util_max: u32,
    ) {
        let task_dir = proc_root.join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(
            task_dir.join("stat"),
            fake_stat_line(tid, comm, starttime_ticks),
        )
        .unwrap();
        fs::write(
            task_dir.join("sched"),
            format!(
                "game-thread ({tid}, #threads: 1)\n-------------------------------------------------------------------\nuclamp.min                                   :                  {util_min}\nuclamp.max                                   :                  {util_max}\neffective uclamp.min                         :                  {util_min}\neffective uclamp.max                         :                  {util_max}\n"
            ),
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
            action_for(42, Some(128), Some(1024)).safety_class(),
            SafetyClass::ReversibleMediumRisk
        );
    }

    #[test]
    fn action_id_and_description_include_requested_values() {
        let action = action_for(42, Some(128), None);

        assert_eq!(
            action.id(),
            ActionId::new("uclamp:set:min=128:max=keep:targets:1".to_owned())
        );
        assert_eq!(
            action.describe(),
            "set uclamp min=128 max=keep for task(s) [42]"
        );
    }

    #[test]
    fn parses_starttime_from_proc_stat() {
        let stat = fake_stat_line(42, "game-thread", 98765);

        let parsed = parse_stat_starttime(&stat).unwrap();

        assert_eq!(parsed, 98765);
    }

    #[test]
    fn parses_uclamp_values_from_sched() {
        let sched = "uclamp.min                                   :                  128\nuclamp.max                                   :                  1024\neffective uclamp.min                         :                  128\neffective uclamp.max                         :                  1024\n";

        let values = parse_sched_uclamp(sched).unwrap();

        assert_eq!(
            values,
            UclampCurrentValues {
                sched_util_min: 128,
                sched_util_max: 1024
            }
        );
    }

    #[test]
    fn preflight_rejects_empty_targets() {
        let action = UclampAction {
            targets: Vec::new(),
            values: UclampValues {
                sched_util_min: Some(128),
                sched_util_max: None,
            },
        };

        let err = action
            .preflight_at(&temp_proc_root("empty-targets"), &UclampPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("requires at least one explicit target task"));
    }

    #[test]
    fn preflight_rejects_empty_requested_values() {
        let action = action_for(42, None, None);

        let err = action
            .preflight_at(&temp_proc_root("empty-values"), &UclampPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("requires sched_util_min, sched_util_max, or both"));
    }

    #[test]
    fn preflight_rejects_values_outside_uclamp_range() {
        let action = action_for(42, Some(1025), None);

        let err = action
            .preflight_at(&temp_proc_root("bad-range"), &UclampPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("outside uclamp range"));
    }

    #[test]
    fn preflight_rejects_min_greater_than_max() {
        let action = action_for(42, Some(900), Some(100));

        let err = action
            .preflight_at(&temp_proc_root("bad-min-max"), &UclampPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("greater than sched_util_max"));
    }

    #[test]
    fn preflight_rejects_when_policy_disallows_uclamp_changes() {
        let proc_root = temp_proc_root("policy-disallow");
        write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

        let policy = UclampPolicy {
            allow_uclamp_changes: false,
            ..UclampPolicy::default()
        };

        let err = action_for(42, Some(128), None)
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("policy does not allow uclamp changes"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_when_policy_disallows_per_task_control() {
        let proc_root = temp_proc_root("policy-no-per-task");
        write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

        let policy = UclampPolicy {
            allow_per_task: false,
            allow_cgroup: true,
            ..UclampPolicy::default()
        };

        let err = action_for(42, Some(128), None)
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("policy does not allow per-task uclamp changes"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_requested_values_outside_policy_range() {
        let proc_root = temp_proc_root("policy-range");
        write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

        let policy = UclampPolicy {
            allow_uclamp_changes: true,
            min_allowed_util_min: 0,
            max_allowed_util_min: 256,
            min_allowed_util_max: 512,
            max_allowed_util_max: 1024,
            allow_per_task: true,
            allow_cgroup: false,
        };

        let err = action_for(42, Some(512), None)
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("requested sched_util_min 512 is outside policy range"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_missing_task() {
        let proc_root = temp_proc_root("missing-task");

        let err = action_for(42, Some(128), None)
            .preflight_at(&proc_root, &UclampPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("failed to preflight uclamp target tid=42"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_rejects_starttime_mismatch() {
        let proc_root = temp_proc_root("starttime-mismatch");
        write_fake_task(&proc_root, 42, "game-thread", 99999, 0, 1024);

        let err = action_for(42, Some(128), None)
            .preflight_at(&proc_root, &UclampPolicy::default())
            .unwrap_err();

        assert!(format!("{:#}", err).contains("starttime mismatch"));
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn preflight_warns_on_comm_mismatch_and_missing_process_pid() {
        let proc_root = temp_proc_root("comm-warning");
        write_fake_task(&proc_root, 42, "new-comm", 12345, 0, 1024);

        let action = UclampAction {
            targets: vec![target_without_process_pid(42, "old-comm", 12345)],
            values: UclampValues {
                sched_util_min: Some(128),
                sched_util_max: None,
            },
        };

        let warnings = action
            .preflight_at(&proc_root, &UclampPolicy::default())
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
        write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

        let state = action_for(42, Some(128), Some(900))
            .dry_run_at(&proc_root, &UclampPolicy::default())
            .unwrap();

        assert!(!state.applied);
        assert_eq!(state.checked_tasks, 1);
        assert_eq!(state.affected_tasks, 1);
        assert_eq!(state.pending_changes, 1);

        let snapshot = read_task_uclamp_from_sched_at(&proc_root, 42).unwrap();
        assert_eq!(
            snapshot,
            UclampCurrentValues {
                sched_util_min: 0,
                sched_util_max: 1024
            }
        );
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn dry_run_reports_zero_pending_when_already_at_requested_values() {
        let proc_root = temp_proc_root("dry-run-noop");
        write_fake_task(&proc_root, 42, "game-thread", 12345, 128, 900);

        let state = action_for(42, Some(128), Some(900))
            .dry_run_at(&proc_root, &UclampPolicy::default())
            .unwrap();

        assert!(!state.applied);
        assert_eq!(state.checked_tasks, 1);
        assert_eq!(state.affected_tasks, 0);
        assert_eq!(state.pending_changes, 0);
        fs::remove_dir_all(proc_root).ok();
    }

    #[test]
    fn rollback_rejects_wrong_token_kind() {
        let action = action_for(42, Some(128), None);
        let token = RollbackToken::IoPrioRestore {
            records: Vec::new(),
        };

        let err = action.rollback(&token).unwrap_err().to_string();

        assert!(err.contains("rollback token is not a uclamp restore token"));
    }

    #[test]
    fn rollback_token_reports_affected_tasks() {
        let token = RollbackToken::UclampRestore {
            records: vec![
                UclampRestoreRecord {
                    tid: 1,
                    original_util_min: 0,
                    original_util_max: 1024,
                },
                UclampRestoreRecord {
                    tid: 2,
                    original_util_min: 128,
                    original_util_max: 900,
                },
            ],
        };

        assert_eq!(token.affected_tasks(), 2);
        assert!(token.restore_path().is_none());
    }
}
