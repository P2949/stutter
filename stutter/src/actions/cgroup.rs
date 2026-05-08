use std::{
    fs,
    fs::OpenOptions,
    path::{Component, Path, PathBuf},
};

use anyhow::Context;

use crate::{
    actions::{
        ActionId, ActionState, ActionWarning, CgroupRestoreRecord, RollbackToken, SafetyClass,
        TaskIdentity, TuningAction,
    },
    process_tree::TaskClass,
};

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct CgroupPlacementTarget {
    pub identity: TaskIdentity,
    pub class: TaskClass,
}

#[derive(Debug, Clone)]
pub struct CgroupPlacementAction {
    pub cgroup_root: PathBuf,
    pub target_cgroup: PathBuf,
    pub targets: Vec<CgroupPlacementTarget>,
    pub cpuset_cpus: Option<String>,
    pub cpuset_mems: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CgroupTargetSnapshot {
    tid: u32,
    comm: Option<String>,
    starttime_ticks: Option<u64>,
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
        let RollbackToken::CgroupRestore { records } = token else {
            anyhow::bail!("rollback token is not a cgroup restore token");
        };

        let mut failures = Vec::new();

        for record in records {
            if !task_exists(proc_root, record.pid) {
                log::info!(
                    "cgroup_rollback_skip_exited_task tid={} original_cgroup={}",
                    record.pid,
                    record.original_cgroup.display()
                );
                continue;
            }

            let original_abs = self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&record.original_cgroup));
            let procs = original_abs.join("cgroup.procs");

            if let Err(err) = write_trimmed(&procs, &record.pid.to_string()) {
                failures.push(format!(
                    "tid={} original_cgroup={} error={err:#}",
                    record.pid,
                    original_abs.display()
                ));
            }
        }

        if !failures.is_empty() {
            anyhow::bail!(
                "failed to rollback cgroup placement: {}",
                failures.join("; ")
            );
        }

        Ok(())
    }
}

impl TuningAction for CgroupPlacementAction {
    fn id(&self) -> ActionId {
        ActionId(format!(
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

    fn apply(&self) -> anyhow::Result<RollbackToken> {
        let policy = CgroupPlacementPolicy::default();
        let snapshots = self.collect_target_snapshots_at(Path::new("/proc"), &policy)?;
        let target_abs = self.target_cgroup_abs()?;

        if let Some(cpuset_cpus) = &self.cpuset_cpus {
            write_trimmed(&target_abs.join("cpuset.cpus"), cpuset_cpus)?;
        }

        if let Some(cpuset_mems) = &self.cpuset_mems {
            write_trimmed(&target_abs.join("cpuset.mems"), cpuset_mems)?;
        }

        let mut records = Vec::new();
        for (snapshot, _) in snapshots {
            let current_target = self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&snapshot.original_cgroup));
            if current_target == target_abs {
                continue;
            }

            write_trimmed(&target_abs.join("cgroup.procs"), &snapshot.tid.to_string())
                .with_context(|| {
                    format!(
                        "failed to move tid={} to {}",
                        snapshot.tid,
                        target_abs.display()
                    )
                })?;

            records.push(CgroupRestoreRecord {
                pid: snapshot.tid,
                original_cgroup: snapshot.original_cgroup,
            });
        }

        Ok(RollbackToken::CgroupRestore { records })
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

    let original_cgroup = read_proc_cgroup_path_at(proc_root, target.tid)
        .with_context(|| format!("failed to read cgroup path for tid={}", target.tid))?;

    Ok(CgroupTargetSnapshot {
        tid: target.tid,
        comm,
        starttime_ticks: Some(starttime_ticks),
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

fn write_trimmed(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value.trim())
        .with_context(|| format!("failed to write {} to {}", value.trim(), path.display()))
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
            ActionId("cgroup:place:/stutter/game.slice:targets:1".to_owned())
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
            records: vec![CgroupRestoreRecord {
                pid: 42,
                original_cgroup: PathBuf::from("/old.slice"),
            }],
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
                CgroupRestoreRecord {
                    pid: 1,
                    original_cgroup: PathBuf::from("/a.slice"),
                },
                CgroupRestoreRecord {
                    pid: 2,
                    original_cgroup: PathBuf::from("/b.slice"),
                },
            ],
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
