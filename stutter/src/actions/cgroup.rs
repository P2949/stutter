use std::{
    fs,
    fs::OpenOptions,
    io,
    path::{Component, Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    actions::{
        ActionId, ActionState, ActionWarning, ApplyResult, CgroupCpusetRestoreRecord,
        CgroupRestoreRecord, PartialApplyError, RestoreIdentityStatus, RollbackToken, SafetyClass,
        TaskIdentity, TaskRestoreIdentity, TuningAction,
        restore_write::{RestoreSummary, RestoreWriteError, classify_restore_write_error},
        rollback::{
            RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult,
            token_dry_run_preview,
        },
        verify_task_identity,
    },
    process_tree::TaskClass,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CgroupPlacementPolicy {
    pub allow_cgroup_moves: bool,
    pub allow_cpuset_changes: bool,
    pub allow_nested_cgroups: bool,
}

impl Default for CgroupPlacementPolicy {
    fn default() -> Self {
        Self {
            allow_cgroup_moves: true,
            allow_cpuset_changes: true,
            allow_nested_cgroups: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CgroupPlacementTarget {
    pub identity: TaskIdentity,
    pub class: TaskClass,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CgroupPlacementAction {
    pub cgroup_root: PathBuf,
    pub target_cgroup: PathBuf,
    pub targets: Vec<CgroupPlacementTarget>,
    pub cpuset_cpus: Option<String>,
    pub cpuset_mems: Option<String>,
}

pub(crate) struct CgroupRollbackHandler;

impl RollbackHandler for CgroupRollbackHandler {
    fn id(&self) -> &'static str {
        "cgroup-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("cgroup rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("cgroup rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::CgroupRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("cgroup rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "cgroup-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::CgroupRestore { records, cpuset } = token else {
            anyhow::bail!("cgroup rollback handler does not support {token:?}");
        };

        let mut restored = 0;
        let mut skipped_dead = 0;
        let mut skipped_identity_mismatch = 0;
        let mut legacy_unverified = 0;
        let mut errors = 0;
        let mut messages = Vec::new();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid;
            let status = verify_task_identity(Path::new("/proc"), &identity);
            match status {
                RestoreIdentityStatus::Missing => {
                    skipped_dead += 1;
                    log::debug!("cgroup restore skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "cgroup restore identity mismatch for tid={}: {}",
                        tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "cgroup restore running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            let original_cgroup = if record.original_cgroup.is_absolute() {
                record.original_cgroup.clone()
            } else {
                Path::new("/sys/fs/cgroup")
                    .join(strip_cgroup_leading_slash(&record.original_cgroup))
            };
            let cgroup_procs = original_cgroup.join("cgroup.procs");
            match write_trimmed(&cgroup_procs, &tid.to_string()) {
                Ok(()) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(Path::new("/proc"), tid, e) {
                    RestoreWriteError::MissingTask => {
                        skipped_dead += 1;
                        log::debug!("cgroup restore skipped: task tid={} is missing/dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(e)
                    | RestoreWriteError::InvalidValue(e)
                    | RestoreWriteError::Io(e) => {
                        errors += 1;
                        let msg = format!(
                            "failed to restore pid={} to cgroup {}: {}",
                            tid,
                            original_cgroup.display(),
                            e
                        );
                        log::error!("{}", msg);
                        messages.push(msg);
                    }
                },
            }
        }

        if let Some(cpuset) = cpuset {
            match restore_cpuset_record(Path::new("/sys/fs/cgroup"), cpuset) {
                Ok(restored_files) => {
                    restored += restored_files;
                }
                Err(err) => {
                    errors += 1;
                    let msg = format!("failed to restore cgroup cpuset state: {err:#}");
                    log::error!("{}", msg);
                    messages.push(msg);
                }
            }
        }

        if errors > 0 {
            anyhow::bail!(
                "failed to rollback cgroup placement: {}",
                messages.join("; ")
            );
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
struct CgroupTargetSnapshot {
    tid: u32,
    process_pid: Option<u32>,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
    exe: Option<std::path::PathBuf>,
    original_cgroup: PathBuf,
}

impl CgroupPlacementAction {
    pub fn preflight_with_policy(
        &self,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    #[cfg(test)]
    pub(crate) fn preflight_with_policy_at_for_tests(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(proc_root, policy)
    }

    fn preflight_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.collect_target_snapshots_at(proc_root, policy)
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .flat_map(|(_, warnings)| warnings)
                    .collect()
            })
    }

    fn dry_run_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let target_abs = self.target_cgroup_abs()?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&snapshot.original_cgroup))
                != target_abs
            {
                pending_changes += 1;
            }
        }

        if self.cpuset_cpus.is_some() || self.cpuset_mems.is_some() {
            pending_changes = pending_changes.saturating_add(1);
        }

        Ok(ActionState {
            applied: false,
            affected_tasks: pending_changes,
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    fn verify_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<ActionState> {
        validate_action_request(self, policy)?;
        let target_rel = normalize_cgroup_path(&self.target_cgroup)?;
        let target_abs = self.target_cgroup_abs()?;
        let mut warnings = Vec::new();
        let mut checked_tasks = 0usize;
        let mut pending_changes = 0usize;

        for target in &self.targets {
            if !task_exists(proc_root, target.identity.tid) {
                warnings.push(ActionWarning {
                    message: format!(
                        "target tid={} exited before cgroup placement verify completed",
                        target.identity.tid
                    ),
                });
                continue;
            }

            checked_tasks += 1;
            let current =
                read_proc_cgroup_path_at(proc_root, target.identity.tid).with_context(|| {
                    format!(
                        "failed to read current cgroup for tid={}",
                        target.identity.tid
                    )
                })?;

            if normalize_cgroup_path(&current)? != target_rel {
                pending_changes += 1;
            }
        }

        if self.cpuset_cpus.is_some() {
            let current = read_trimmed(&target_abs.join("cpuset.cpus")).with_context(|| {
                format!(
                    "failed to read {}",
                    target_abs.join("cpuset.cpus").display()
                )
            })?;
            if Some(current.as_str()) != self.cpuset_cpus.as_deref() {
                pending_changes += 1;
            }
        }

        if self.cpuset_mems.is_some() {
            let current = read_trimmed(&target_abs.join("cpuset.mems")).with_context(|| {
                format!(
                    "failed to read {}",
                    target_abs.join("cpuset.mems").display()
                )
            })?;
            if Some(current.as_str()) != self.cpuset_mems.as_deref() {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: checked_tasks > 0 && pending_changes == 0,
            affected_tasks: checked_tasks,
            checked_tasks,
            pending_changes,
            warnings,
        })
    }

    fn collect_target_snapshots_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<(CgroupTargetSnapshot, Vec<ActionWarning>)>> {
        validate_action_request(self, policy)?;
        preflight_cgroup_files(self, policy)?;

        let mut snapshots = Vec::with_capacity(self.targets.len());

        for target in &self.targets {
            validate_target_class(target.class)?;
            let snapshot =
                read_target_snapshot_at(proc_root, &target.identity).with_context(|| {
                    format!(
                        "failed to preflight cgroup target tid={}",
                        target.identity.tid
                    )
                })?;
            let warnings = identity_warnings(&target.identity, &snapshot);
            snapshots.push((snapshot, warnings));
        }

        Ok(snapshots)
    }

    fn cpuset_restore_record(
        &self,
        target_abs: &Path,
    ) -> anyhow::Result<Option<CgroupCpusetRestoreRecord>> {
        if self.cpuset_cpus.is_none() && self.cpuset_mems.is_none() {
            return Ok(None);
        }

        Ok(Some(CgroupCpusetRestoreRecord {
            cgroup_path: normalize_cgroup_path(&self.target_cgroup)?,
            original_cpuset_cpus: if self.cpuset_cpus.is_some() {
                read_optional_trimmed(&target_abs.join("cpuset.cpus"))?
            } else {
                None
            },
            original_cpuset_mems: if self.cpuset_mems.is_some() {
                read_optional_trimmed(&target_abs.join("cpuset.mems"))?
            } else {
                None
            },
        }))
    }

    fn target_cgroup_abs(&self) -> anyhow::Result<PathBuf> {
        let target_rel = normalize_cgroup_path(&self.target_cgroup)?;
        Ok(self
            .cgroup_root
            .join(strip_cgroup_leading_slash(&target_rel)))
    }

    pub(crate) fn rollback_at(
        &self,
        proc_root: &Path,
        token: &RollbackToken,
    ) -> anyhow::Result<()> {
        let RollbackToken::CgroupRestore { records, cpuset } = token else {
            anyhow::bail!("rollback token is not a cgroup restore token");
        };

        let mut failures = Vec::new();
        let mut summary = RestoreSummary::default();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid;
            match verify_task_identity(proc_root, &identity) {
                RestoreIdentityStatus::SameTask => {}
                RestoreIdentityStatus::UnknownLegacy => {
                    log::warn!(
                        "cgroup rollback running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::Missing => {
                    summary.record_missing();
                    log::info!(
                        "cgroup_rollback_skip_exited_task tid={} original_cgroup={}",
                        tid,
                        record.original_cgroup.display()
                    );
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    summary.record_identity_mismatch();
                    log::warn!(
                        "cgroup_rollback_skip_identity_mismatch tid={} original_cgroup={} reason={}",
                        tid,
                        record.original_cgroup.display(),
                        reason
                    );
                    continue;
                }
            }

            let original_abs = self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&record.original_cgroup));
            let procs = original_abs.join("cgroup.procs");

            if let Err(err) = write_trimmed(&procs, &tid.to_string()) {
                match classify_restore_write_error(proc_root, tid, err) {
                    RestoreWriteError::MissingTask => {
                        summary.record_missing();
                        log::info!(
                            "cgroup_rollback_skip_exited_task tid={} original_cgroup={}",
                            tid,
                            record.original_cgroup.display()
                        );
                    }
                    RestoreWriteError::PermissionDenied(err)
                    | RestoreWriteError::InvalidValue(err)
                    | RestoreWriteError::Io(err) => {
                        summary.record_failure();
                        failures.push(format!(
                            "tid={} original_cgroup={} error={err:#}",
                            tid,
                            original_abs.display()
                        ));
                    }
                }
            } else {
                summary.record_restored();
            }
        }

        if let Some(cpuset) = cpuset
            && let Err(err) = restore_cpuset_record(&self.cgroup_root, cpuset)
        {
            summary.record_failure();
            failures.push(format!("cpuset_restore_error={err:#}"));
        }

        if summary.has_failures() {
            anyhow::bail!(
                "failed to rollback cgroup placement after attempting all records: restored={} skipped_missing={} skipped_identity_mismatch={} failed={} errors={}",
                summary.restored,
                summary.skipped_missing,
                summary.skipped_identity_mismatch,
                summary.failed,
                failures.join("; ")
            );
        }

        Ok(())
    }

    fn apply_at(&self, proc_root: &Path, policy: &CgroupPlacementPolicy) -> ApplyResult {
        let mut writer = FsCgroupFileWriter;
        self.apply_at_with_writer(proc_root, policy, &mut writer)
    }

    fn apply_at_with_writer<W: CgroupFileWriter>(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
        writer: &mut W,
    ) -> ApplyResult {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let target_abs = self.target_cgroup_abs()?;
        let cpuset = self.cpuset_restore_record(&target_abs)?;
        let mut cpuset_changed = false;

        if let Some(cpuset_cpus) = &self.cpuset_cpus {
            writer
                .write_trimmed(&target_abs.join("cpuset.cpus"), cpuset_cpus)
                .with_context(|| {
                    format!(
                        "failed to write {}",
                        target_abs.join("cpuset.cpus").display()
                    )
                })?;
            cpuset_changed = true;
        }

        if let Some(cpuset_mems) = &self.cpuset_mems
            && let Err(err) = writer
                .write_trimmed(&target_abs.join("cpuset.mems"), cpuset_mems)
                .with_context(|| {
                    format!(
                        "failed to write {}",
                        target_abs.join("cpuset.mems").display()
                    )
                })
        {
            return Err(self.partial_apply_error_after_rollback(
                proc_root,
                err,
                Vec::new(),
                cpuset_changed,
                &cpuset,
            ));
        }
        if self.cpuset_mems.is_some() {
            cpuset_changed = true;
        }

        let mut records = Vec::new();
        for (snapshot, _) in snapshots {
            let current_target = self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&snapshot.original_cgroup));
            if current_target == target_abs {
                continue;
            }

            let identity = TaskRestoreIdentity::observed(
                snapshot.tid,
                snapshot.process_pid,
                snapshot.comm.clone(),
                snapshot.starttime_ticks,
                snapshot.exe.clone(),
            );

            if let Err(err) = writer
                .write_trimmed(&target_abs.join("cgroup.procs"), &snapshot.tid.to_string())
                .with_context(|| {
                    format!(
                        "failed to move tid={} to {}",
                        snapshot.tid,
                        target_abs.display()
                    )
                })
            {
                return Err(self.partial_apply_error_after_rollback(
                    proc_root,
                    err,
                    records,
                    cpuset_changed,
                    &cpuset,
                ));
            }

            records.push(CgroupRestoreRecord::new(identity, snapshot.original_cgroup));
        }

        Ok(RollbackToken::CgroupRestore {
            records,
            cpuset: cpuset.filter(|_| cpuset_changed),
        })
    }

    fn partial_apply_error_after_rollback(
        &self,
        proc_root: &Path,
        source: anyhow::Error,
        records: Vec<CgroupRestoreRecord>,
        cpuset_changed: bool,
        cpuset: &Option<CgroupCpusetRestoreRecord>,
    ) -> PartialApplyError {
        let rollback = cgroup_partial_token(records, cpuset_changed, cpuset);
        let source = match rollback.as_ref() {
            Some(token) => match self.rollback_at(proc_root, token) {
                Ok(()) => source,
                Err(rollback_err) => anyhow::anyhow!(
                    "apply failed: {source:#}; partial cgroup rollback failed: {rollback_err:#}"
                ),
            },
            None => source,
        };

        PartialApplyError { source, rollback }
    }
}

trait CgroupFileWriter {
    fn write_trimmed(&mut self, path: &Path, value: &str) -> anyhow::Result<()>;
}

struct FsCgroupFileWriter;

impl CgroupFileWriter for FsCgroupFileWriter {
    fn write_trimmed(&mut self, path: &Path, value: &str) -> anyhow::Result<()> {
        write_trimmed(path, value)
    }
}

impl TuningAction for CgroupPlacementAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "cgroup:place:{}:targets:{}",
            normalize_cgroup_path(&self.target_cgroup)
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| self.target_cgroup.display().to_string()),
            self.targets.len()
        ))
    }

    fn describe(&self) -> String {
        let tids = self
            .targets
            .iter()
            .map(|target| target.identity.tid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "move task(s) [{}] to cgroup {}",
            tids,
            self.target_cgroup.display()
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleMediumRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&CgroupPlacementPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(Path::new("/proc"), &CgroupPlacementPolicy::default())
    }

    fn apply(&self) -> ApplyResult {
        self.apply_at(Path::new("/proc"), &CgroupPlacementPolicy::default())
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &CgroupPlacementPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.rollback_at(Path::new("/proc"), token)
    }
}

fn validate_action_request(
    action: &CgroupPlacementAction,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_cgroup_moves {
        anyhow::bail!("policy does not allow cgroup moves");
    }

    if action.targets.is_empty() {
        anyhow::bail!("cgroup placement requires at least one explicit target task");
    }

    let target_rel = normalize_cgroup_path(&action.target_cgroup)?;
    if !policy.allow_nested_cgroups && target_rel.components().count() > 2 {
        anyhow::bail!(
            "policy does not allow nested cgroups: {}",
            target_rel.display()
        );
    }

    if (action.cpuset_cpus.is_some() || action.cpuset_mems.is_some())
        && !policy.allow_cpuset_changes
    {
        anyhow::bail!("policy does not allow cpuset changes");
    }

    if let Some(cpuset_cpus) = &action.cpuset_cpus {
        validate_cpuset_value("cpuset.cpus", cpuset_cpus)?;
    }

    if let Some(cpuset_mems) = &action.cpuset_mems {
        validate_cpuset_value("cpuset.mems", cpuset_mems)?;
    }

    Ok(())
}

fn validate_target_class(class: TaskClass) -> anyhow::Result<()> {
    if matches!(
        class,
        TaskClass::AudioRealtime
            | TaskClass::Input
            | TaskClass::KernelThread
            | TaskClass::IrqThread
            | TaskClass::Service
            | TaskClass::NetworkDaemon
            | TaskClass::StorageDaemon
            | TaskClass::Unknown
    ) {
        anyhow::bail!("refusing to move system/critical task class {class}");
    }

    Ok(())
}

fn preflight_cgroup_files(
    action: &CgroupPlacementAction,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    let target_abs = action.target_cgroup_abs()?;

    if !target_abs.is_dir() {
        anyhow::bail!("target cgroup does not exist: {}", target_abs.display());
    }

    ensure_path_under_root(&action.cgroup_root, &target_abs)?;
    ensure_writable_file(&target_abs.join("cgroup.procs"))?;

    if action.cpuset_cpus.is_some() {
        ensure_cpuset_available(&target_abs, "cpuset.cpus", policy)?;
    }

    if action.cpuset_mems.is_some() {
        ensure_cpuset_available(&target_abs, "cpuset.mems", policy)?;
    }

    Ok(())
}

fn ensure_cpuset_available(
    target_abs: &Path,
    file_name: &str,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_cpuset_changes {
        anyhow::bail!("policy does not allow cpuset changes");
    }

    ensure_writable_file(&target_abs.join(file_name))
}

fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        anyhow::bail!("required cgroup file does not exist: {}", path.display());
    }

    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("required cgroup file is not writable: {}", path.display()))?;

    Ok(())
}

fn ensure_path_under_root(root: &Path, path: &Path) -> anyhow::Result<()> {
    if !path.starts_with(root) {
        anyhow::bail!(
            "target cgroup {} is outside cgroup root {}",
            path.display(),
            root.display()
        );
    }

    Ok(())
}

fn normalize_cgroup_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut normalized = PathBuf::from("/");

    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                anyhow::bail!(
                    "cgroup path must not contain parent traversal: {}",
                    path.display()
                )
            }
            Component::Prefix(_) => {
                anyhow::bail!(
                    "cgroup path must not contain platform prefix: {}",
                    path.display()
                )
            }
        }
    }

    Ok(normalized)
}

fn strip_cgroup_leading_slash(path: &Path) -> &Path {
    path.strip_prefix("/").unwrap_or(path)
}

fn validate_cpuset_value(name: &str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{name} must not be empty");
    }

    if value.trim() != value {
        anyhow::bail!("{name} must not contain leading or trailing whitespace");
    }

    for ch in value.chars() {
        if !(ch.is_ascii_digit() || ch == ',' || ch == '-') {
            anyhow::bail!("{name} contains invalid character {ch:?}");
        }
    }

    Ok(())
}

fn read_target_snapshot_at(
    proc_root: &Path,
    target: &TaskIdentity,
) -> anyhow::Result<CgroupTargetSnapshot> {
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

    let original_cgroup = read_proc_cgroup_path_at(proc_root, target.tid)
        .with_context(|| format!("failed to read cgroup path for tid={}", target.tid))?;

    Ok(CgroupTargetSnapshot {
        tid: target.tid,
        process_pid: target.process_pid,
        comm,
        starttime_ticks: Some(starttime_ticks),
        exe,
        original_cgroup,
    })
}

fn identity_warnings(target: &TaskIdentity, snapshot: &CgroupTargetSnapshot) -> Vec<ActionWarning> {
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

fn read_proc_cgroup_path_at(proc_root: &Path, tid: u32) -> anyhow::Result<PathBuf> {
    let cgroup_path = proc_root.join(tid.to_string()).join("cgroup");
    let contents = fs::read_to_string(&cgroup_path)
        .with_context(|| format!("failed to read {}", cgroup_path.display()))?;

    let path = contents
        .lines()
        .filter_map(|line| line.rsplit_once(':').map(|(_, path)| path.trim()))
        .filter(|path| !path.is_empty())
        .max_by_key(|path| path.len())
        .context("proc cgroup file did not contain a cgroup v2 path")?;

    normalize_cgroup_path(Path::new(path))
}

fn task_exists(proc_root: &Path, tid: u32) -> bool {
    proc_root.join(tid.to_string()).join("stat").is_file()
}

fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}

fn read_optional_trimmed(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn cgroup_partial_token(
    records: Vec<CgroupRestoreRecord>,
    cpuset_changed: bool,
    cpuset: &Option<CgroupCpusetRestoreRecord>,
) -> Option<RollbackToken> {
    if records.is_empty() && !cpuset_changed {
        None
    } else {
        Some(RollbackToken::CgroupRestore {
            records,
            cpuset: if cpuset_changed { cpuset.clone() } else { None },
        })
    }
}

fn restore_cpuset_record(
    cgroup_root: &Path,
    record: &CgroupCpusetRestoreRecord,
) -> anyhow::Result<usize> {
    let cgroup_path = if record.cgroup_path.starts_with(cgroup_root) {
        record.cgroup_path.clone()
    } else {
        cgroup_root.join(strip_cgroup_leading_slash(&record.cgroup_path))
    };

    let mut restored = 0usize;
    if let Some(original) = &record.original_cpuset_cpus {
        write_trimmed(&cgroup_path.join("cpuset.cpus"), original).with_context(|| {
            format!(
                "failed to restore {}",
                cgroup_path.join("cpuset.cpus").display()
            )
        })?;
        restored += 1;
    }

    if let Some(original) = &record.original_cpuset_mems {
        write_trimmed(&cgroup_path.join("cpuset.mems"), original).with_context(|| {
            format!(
                "failed to restore {}",
                cgroup_path.join("cpuset.mems").display()
            )
        })?;
        restored += 1;
    }

    Ok(restored)
}

#[cfg(test)]
fn is_dead_task_io_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|io_err| {
            io_err.kind() == io::ErrorKind::NotFound || io_err.raw_os_error() == Some(libc::ESRCH)
        })
    })
}

fn write_trimmed(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value.trim())
        .with_context(|| format!("failed to write {} to {}", value.trim(), path.display()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Context as _;

    use super::*;

    fn target(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
        TaskIdentity {
            tid,
            process_pid: Some(tid),
            comm: Some(comm.to_owned()),
            starttime_ticks: Some(starttime_ticks),
        }
    }

    fn placement_target(tid: u32, comm: &str, class: TaskClass) -> CgroupPlacementTarget {
        CgroupPlacementTarget {
            identity: target(tid, comm, 12345),
            class,
        }
    }

    fn action_for(root: &Path, tid: u32) -> CgroupPlacementAction {
        CgroupPlacementAction {
            cgroup_root: root.to_path_buf(),
            target_cgroup: PathBuf::from("/stutter/game.slice"),
            targets: vec![placement_target(tid, "game-thread", TaskClass::Game)],
            cpuset_cpus: Some("2-3".to_owned()),
            cpuset_mems: Some("0".to_owned()),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-cgroup-action-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fake_task(proc_root: &Path, tid: u32, comm: &str, starttime_ticks: u64, cgroup: &str) {
        let task_dir = proc_root.join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(
            task_dir.join("status"),
            format!("Name:\t{comm}\nTgid:\t{tid}\nPid:\t{tid}\n"),
        )
        .unwrap();
        fs::write(
            task_dir.join("stat"),
            fake_stat_line(tid, comm, starttime_ticks),
        )
        .unwrap();
        fs::write(task_dir.join("cgroup"), format!("0::{cgroup}\n")).unwrap();
    }

    fn fake_stat_line(tid: u32, comm: &str, starttime_ticks: u64) -> String {
        let mut fields = vec!["0".to_owned(); 20];
        fields[0] = "S".to_owned();
        fields[19] = starttime_ticks.to_string();

        format!("{tid} ({comm}) {}", fields.join(" "))
    }

    fn write_fake_cgroup(root: &Path, relative: &str) -> PathBuf {
        let path = root.join(relative.trim_start_matches('/'));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("cgroup.procs"), "").unwrap();
        fs::write(path.join("cpuset.cpus"), "0-7\n").unwrap();
        fs::write(path.join("cpuset.mems"), "0\n").unwrap();
        path
    }

    struct FailingCgroupWriter {
        writes: usize,
        fail_on_write: usize,
    }

    impl FailingCgroupWriter {
        fn fail_on_write(fail_on_write: usize) -> Self {
            Self {
                writes: 0,
                fail_on_write,
            }
        }
    }

    impl CgroupFileWriter for FailingCgroupWriter {
        fn write_trimmed(&mut self, path: &Path, value: &str) -> anyhow::Result<()> {
            self.writes += 1;
            if self.writes == self.fail_on_write {
                anyhow::bail!("injected cgroup write failure for {}", path.display());
            }

            super::write_trimmed(path, value)
        }
    }

    #[test]
    fn cgroup_partial_token_keeps_cpuset_restore_state() {
        let cpuset = CgroupCpusetRestoreRecord {
            cgroup_path: PathBuf::from("/stutter/game.slice"),
            original_cpuset_cpus: Some("0-7".to_owned()),
            original_cpuset_mems: Some("0".to_owned()),
        };

        let token = cgroup_partial_token(Vec::new(), true, &Some(cpuset.clone()))
            .expect("cpuset mutation should produce a rollback token");

        let RollbackToken::CgroupRestore {
            records,
            cpuset: restored_cpuset,
        } = token
        else {
            panic!("expected cgroup rollback token");
        };
        assert!(records.is_empty());
        assert_eq!(restored_cpuset, Some(cpuset));
    }

    #[test]
    fn rollback_restores_cpuset_files_from_token() {
        let proc_root = temp_dir("proc-cpuset-rollback");
        let cgroup_root = temp_dir("cgroup-cpuset-rollback");
        let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        fs::write(target.join("cpuset.cpus"), "2-3\n").unwrap();
        fs::write(target.join("cpuset.mems"), "1\n").unwrap();
        let action = action_for(&cgroup_root, 42);
        let token = RollbackToken::CgroupRestore {
            records: Vec::new(),
            cpuset: Some(CgroupCpusetRestoreRecord {
                cgroup_path: PathBuf::from("/stutter/game.slice"),
                original_cpuset_cpus: Some("0-7".to_owned()),
                original_cpuset_mems: Some("0".to_owned()),
            }),
        };

        action.rollback_at(&proc_root, &token).unwrap();

        assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
        assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn detects_dead_task_io_errors_through_anyhow_context() {
        let missing = Err::<(), _>(io::Error::new(io::ErrorKind::NotFound, "task gone"))
            .context("failed to write cgroup.procs")
            .unwrap_err();
        assert!(is_dead_task_io_error(&missing));

        let esrch = Err::<(), _>(io::Error::from_raw_os_error(libc::ESRCH))
            .context("failed to move task")
            .unwrap_err();
        assert!(is_dead_task_io_error(&esrch));

        let permission_denied = Err::<(), _>(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "cgroup.procs not writable",
        ))
        .context("failed to write cgroup.procs")
        .unwrap_err();
        assert!(!is_dead_task_io_error(&permission_denied));
    }

    #[test]
    fn safety_class_is_reversible_medium_risk() {
        let root = temp_dir("safety");
        write_fake_cgroup(&root, "/stutter/game.slice");

        assert_eq!(
            action_for(&root, 42).safety_class(),
            SafetyClass::ReversibleMediumRisk
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn action_id_and_description_include_target_cgroup() {
        let root = temp_dir("id");
        write_fake_cgroup(&root, "/stutter/game.slice");
        let action = action_for(&root, 42);

        assert_eq!(
            action.id(),
            ActionId::new("cgroup:place:/stutter/game.slice:targets:1".to_owned())
        );
        assert_eq!(
            action.describe(),
            "move task(s) [42] to cgroup /stutter/game.slice"
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_accepts_fake_nested_cgroup_files() {
        let proc_root = temp_dir("proc-nested");
        let cgroup_root = temp_dir("cgroup-nested");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

        let warnings = action_for(&cgroup_root, 42)
            .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
            .unwrap();

        assert!(warnings.is_empty());
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn preflight_rejects_missing_target_cgroup() {
        let proc_root = temp_dir("proc-missing-cgroup");
        let cgroup_root = temp_dir("cgroup-missing-cgroup");
        write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

        let err = action_for(&cgroup_root, 42)
            .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("target cgroup does not exist"));
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn preflight_rejects_nested_cgroup_when_policy_disallows_it() {
        let proc_root = temp_dir("proc-no-nested");
        let cgroup_root = temp_dir("cgroup-no-nested");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

        let policy = CgroupPlacementPolicy {
            allow_nested_cgroups: false,
            ..CgroupPlacementPolicy::default()
        };

        let err = action_for(&cgroup_root, 42)
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("policy does not allow nested cgroups"));
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn preflight_rejects_cpuset_when_policy_disallows_it() {
        let proc_root = temp_dir("proc-no-cpuset");
        let cgroup_root = temp_dir("cgroup-no-cpuset");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

        let policy = CgroupPlacementPolicy {
            allow_cpuset_changes: false,
            ..CgroupPlacementPolicy::default()
        };

        let err = action_for(&cgroup_root, 42)
            .preflight_at(&proc_root, &policy)
            .unwrap_err()
            .to_string();

        assert!(err.contains("policy does not allow cpuset changes"));
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn preflight_rejects_missing_cgroup_procs_permission_file() {
        let proc_root = temp_dir("proc-no-procs");
        let cgroup_root = temp_dir("cgroup-no-procs");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        let target = cgroup_root.join("stutter/game.slice");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("cpuset.cpus"), "0-7\n").unwrap();
        fs::write(target.join("cpuset.mems"), "0\n").unwrap();
        write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

        let err = action_for(&cgroup_root, 42)
            .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("required cgroup file does not exist"));
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn preflight_rejects_system_critical_task_classes() {
        for class in [
            TaskClass::AudioRealtime,
            TaskClass::Input,
            TaskClass::KernelThread,
            TaskClass::IrqThread,
            TaskClass::Service,
            TaskClass::NetworkDaemon,
            TaskClass::StorageDaemon,
            TaskClass::Unknown,
        ] {
            let proc_root = temp_dir("proc-critical");
            let cgroup_root = temp_dir("cgroup-critical");
            write_fake_cgroup(&cgroup_root, "/old.slice");
            write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
            write_fake_task(&proc_root, 42, "critical", 12345, "/old.slice");

            let mut action = action_for(&cgroup_root, 42);
            action.targets = vec![placement_target(42, "critical", class)];

            let err = action
                .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
                .unwrap_err()
                .to_string();

            assert!(err.contains("refusing to move system/critical task class"));
            fs::remove_dir_all(proc_root).ok();
            fs::remove_dir_all(cgroup_root).ok();
        }
    }

    #[test]
    fn preflight_rejects_starttime_mismatch() {
        let proc_root = temp_dir("proc-starttime");
        let cgroup_root = temp_dir("cgroup-starttime");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 42, "game-thread", 99999, "/old.slice");

        let err = action_for(&cgroup_root, 42)
            .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
            .unwrap_err();

        assert!(format!("{:#}", err).contains("starttime mismatch"));
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn dry_run_counts_pending_move_and_cpuset_changes_without_mutating() {
        let proc_root = temp_dir("proc-dry");
        let cgroup_root = temp_dir("cgroup-dry");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

        let state = action_for(&cgroup_root, 42)
            .dry_run_at(&proc_root, &CgroupPlacementPolicy::default())
            .unwrap();

        assert!(!state.applied);
        assert_eq!(state.checked_tasks, 1);
        assert_eq!(state.affected_tasks, 2);
        assert_eq!(state.pending_changes, 2);
        assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
        assert_eq!(read_trimmed(&target.join("cgroup.procs")).unwrap(), "");
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn apply_restores_cpuset_cpus_when_cpuset_mems_write_fails() {
        let proc_root = temp_dir("proc-apply-mems-fail");
        let cgroup_root = temp_dir("cgroup-apply-mems-fail");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");
        let action = action_for(&cgroup_root, 42);
        let mut writer = FailingCgroupWriter::fail_on_write(2);

        let err = action
            .apply_at_with_writer(&proc_root, &CgroupPlacementPolicy::default(), &mut writer)
            .unwrap_err();

        assert!(format!("{:#}", err.source).contains("cpuset.mems"));
        assert!(err.rollback.is_some());
        assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
        assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
        assert_eq!(read_trimmed(&target.join("cgroup.procs")).unwrap(), "");
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn apply_restores_moved_tasks_and_cpuset_when_second_task_move_fails() {
        let proc_root = temp_dir("proc-apply-second-move-fail");
        let cgroup_root = temp_dir("cgroup-apply-second-move-fail");
        write_fake_cgroup(&cgroup_root, "/old-first.slice");
        write_fake_cgroup(&cgroup_root, "/old-second.slice");
        let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 41, "first", 12345, "/old-first.slice");
        write_fake_task(&proc_root, 42, "second", 12345, "/old-second.slice");

        let mut action = action_for(&cgroup_root, 41);
        action.targets = vec![
            placement_target(41, "first", TaskClass::Game),
            placement_target(42, "second", TaskClass::Game),
        ];
        let mut writer = FailingCgroupWriter::fail_on_write(4);

        let err = action
            .apply_at_with_writer(&proc_root, &CgroupPlacementPolicy::default(), &mut writer)
            .unwrap_err();

        assert!(format!("{:#}", err.source).contains("failed to move tid=42"));
        assert!(err.rollback.is_some());
        assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
        assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
        assert_eq!(
            read_trimmed(&cgroup_root.join("old-first.slice/cgroup.procs")).unwrap(),
            "41"
        );
        assert_eq!(
            read_trimmed(&cgroup_root.join("old-second.slice/cgroup.procs")).unwrap(),
            ""
        );
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn verify_reports_exited_task_as_warning_not_crash() {
        let proc_root = temp_dir("proc-exited-verify");
        let cgroup_root = temp_dir("cgroup-exited-verify");
        write_fake_cgroup(&cgroup_root, "/stutter/game.slice");

        let state = action_for(&cgroup_root, 42)
            .verify_at(&proc_root, &CgroupPlacementPolicy::default())
            .unwrap();

        assert!(!state.applied);
        assert_eq!(state.checked_tasks, 0);
        assert_eq!(state.affected_tasks, 0);
        assert!(state.warnings.iter().any(|warning| {
            warning
                .message
                .contains("exited before cgroup placement verify")
        }));

        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn rollback_skips_exited_tasks_cleanly() {
        let proc_root = temp_dir("proc-rollback-exited");
        let cgroup_root = temp_dir("cgroup-rollback-exited");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        let action = action_for(&cgroup_root, 42);

        let token = RollbackToken::CgroupRestore {
            records: vec![CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    42,
                    None,
                    Some("test".to_owned()),
                    None,
                    None,
                ),
                PathBuf::from("/old.slice"),
            )],
            cpuset: None,
        };

        action.rollback_at(&proc_root, &token).unwrap();

        assert_eq!(
            read_trimmed(&cgroup_root.join("old.slice/cgroup.procs")).unwrap(),
            ""
        );
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn rollback_skips_identity_mismatch_without_restoring_reused_tid() {
        let proc_root = temp_dir("proc-rollback-reused");
        let cgroup_root = temp_dir("cgroup-rollback-reused");
        write_fake_task(&proc_root, 42, "other-thread", 99999, "/stutter/game.slice");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        let action = action_for(&cgroup_root, 42);

        let token = RollbackToken::CgroupRestore {
            records: vec![CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    42,
                    None,
                    Some("game-thread".to_owned()),
                    Some(12345),
                    None,
                ),
                PathBuf::from("/old.slice"),
            )],
            cpuset: None,
        };

        action.rollback_at(&proc_root, &token).unwrap();

        assert_eq!(
            read_trimmed(&cgroup_root.join("old.slice/cgroup.procs")).unwrap(),
            ""
        );
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn rollback_skips_dead_middle_task_and_restores_remaining_records() {
        let proc_root = temp_dir("proc-rollback-dead-middle");
        let cgroup_root = temp_dir("cgroup-rollback-dead-middle");
        write_fake_task(&proc_root, 41, "first", 4100, "/stutter/game.slice");
        write_fake_task(&proc_root, 43, "third", 4300, "/stutter/game.slice");
        write_fake_cgroup(&cgroup_root, "/old-first.slice");
        write_fake_cgroup(&cgroup_root, "/old-second.slice");
        write_fake_cgroup(&cgroup_root, "/old-third.slice");
        let action = action_for(&cgroup_root, 41);

        let token = RollbackToken::CgroupRestore {
            records: vec![
                CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        41,
                        None,
                        Some("first".to_owned()),
                        Some(4100),
                        None,
                    ),
                    PathBuf::from("/old-first.slice"),
                ),
                CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        42,
                        None,
                        Some("second".to_owned()),
                        Some(4200),
                        None,
                    ),
                    PathBuf::from("/old-second.slice"),
                ),
                CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        43,
                        None,
                        Some("third".to_owned()),
                        Some(4300),
                        None,
                    ),
                    PathBuf::from("/old-third.slice"),
                ),
            ],
            cpuset: None,
        };

        action.rollback_at(&proc_root, &token).unwrap();

        assert_eq!(
            read_trimmed(&cgroup_root.join("old-first.slice/cgroup.procs")).unwrap(),
            "41"
        );
        assert_eq!(
            read_trimmed(&cgroup_root.join("old-second.slice/cgroup.procs")).unwrap(),
            ""
        );
        assert_eq!(
            read_trimmed(&cgroup_root.join("old-third.slice/cgroup.procs")).unwrap(),
            "43"
        );
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn rollback_reports_real_write_failure_after_attempting_remaining_records() {
        let proc_root = temp_dir("proc-rollback-write-failure");
        let cgroup_root = temp_dir("cgroup-rollback-write-failure");
        write_fake_task(&proc_root, 41, "first", 4100, "/stutter/game.slice");
        write_fake_task(&proc_root, 43, "third", 4300, "/stutter/game.slice");
        write_fake_cgroup(&cgroup_root, "/old-third.slice");
        let action = action_for(&cgroup_root, 41);

        let token = RollbackToken::CgroupRestore {
            records: vec![
                CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        41,
                        None,
                        Some("first".to_owned()),
                        Some(4100),
                        None,
                    ),
                    PathBuf::from("/missing-old.slice"),
                ),
                CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        43,
                        None,
                        Some("third".to_owned()),
                        Some(4300),
                        None,
                    ),
                    PathBuf::from("/old-third.slice"),
                ),
            ],
            cpuset: None,
        };

        let err = action.rollback_at(&proc_root, &token).unwrap_err();

        assert!(format!("{err:#}").contains("after attempting all records"));
        assert_eq!(
            read_trimmed(&cgroup_root.join("old-third.slice/cgroup.procs")).unwrap(),
            "43"
        );
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn rollback_rejects_wrong_token_kind() {
        let cgroup_root = temp_dir("cgroup-wrong-token");
        write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        let action = action_for(&cgroup_root, 42);

        let err = action
            .rollback(&RollbackToken::NiceRestore {
                records: Vec::new(),
            })
            .unwrap_err()
            .to_string();

        assert!(err.contains("rollback token is not a cgroup restore token"));
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn cgroup_restore_token_reports_affected_tasks_and_no_restore_path() {
        let token = RollbackToken::CgroupRestore {
            records: vec![
                CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        1,
                        None,
                        Some("test".to_owned()),
                        None,
                        None,
                    ),
                    PathBuf::from("/a.slice"),
                ),
                CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        2,
                        None,
                        Some("test".to_owned()),
                        None,
                        None,
                    ),
                    PathBuf::from("/b.slice"),
                ),
            ],
            cpuset: None,
        };

        assert_eq!(token.affected_tasks(), 2);
        assert!(token.restore_path().is_none());
    }

    #[test]
    fn rejects_parent_traversal_in_target_cgroup() {
        let cgroup_root = temp_dir("cgroup-traversal");
        let mut action = action_for(&cgroup_root, 42);
        action.target_cgroup = PathBuf::from("../bad");

        let err = validate_action_request(&action, &CgroupPlacementPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("must not contain parent traversal"));
        fs::remove_dir_all(cgroup_root).ok();
    }

    #[test]
    fn rejects_invalid_cpuset_values() {
        let cgroup_root = temp_dir("cgroup-invalid-cpuset");
        let mut action = action_for(&cgroup_root, 42);
        action.cpuset_cpus = Some("0;rm -rf".to_owned());

        let err = validate_action_request(&action, &CgroupPlacementPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("cpuset.cpus contains invalid character"));
        fs::remove_dir_all(cgroup_root).ok();
    }
}
