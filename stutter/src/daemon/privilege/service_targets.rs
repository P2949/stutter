//! A desktop socket peer cannot confer authority over another account's tasks.
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, ensure};

use crate::{
    actions::RollbackToken, autotune::planning::candidate::CandidateAction,
    daemon::privilege::error::PrivilegedWorkerError,
};

#[derive(Debug)]
pub(super) struct ServiceTargets {
    uid: u32,
    cgroup: Option<PathBuf>,
    /// The validation fence file, if this service honours one, and the account that must own it.
    fence: Option<(PathBuf, u32)>,
}

impl ServiceTargets {
    pub(super) fn new(uid: u32, cgroup: Option<PathBuf>) -> Self {
        Self {
            uid,
            cgroup,
            fence: None,
        }
    }

    /// Honours a validation fence file (see `validation_fence`), which may only narrow the targets,
    /// and which must be owned by `trusted_uid` (root in production).
    pub(super) fn with_fence(mut self, path: PathBuf, trusted_uid: u32) -> Self {
        self.fence = Some((path, trusted_uid));
        self
    }

    /// What this service enforces right now, read from the fence file by the same code that enforces it.
    pub(super) fn fence_report(&self) -> super::FenceReport {
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
        let Some((path, trusted_uid)) = &self.fence else {
            return super::FenceReport {
                protocol: 1,
                state: "not_honoured".to_owned(),
                reason: None,
                root_pid: None,
                start_ticks: None,
                uid: self.uid,
                boot_id: boot.trim().to_owned(),
                fence_sha256: None,
            };
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(u64::MAX, |elapsed| elapsed.as_secs());
        super::validation_fence::report(path, now, boot.trim(), self.uid, *trusted_uid)
    }

    fn fence_check(&self, proc_root: &Path, status: &str) -> anyhow::Result<()> {
        let Some((path, trusted_uid)) = &self.fence else {
            return Ok(());
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(u64::MAX, |elapsed| elapsed.as_secs());
        let boot =
            fs::read_to_string(proc_root.join("sys/kernel/random/boot_id")).unwrap_or_default();
        let state = super::validation_fence::load(path, now, boot.trim(), self.uid, *trusted_uid)?;
        // Without a fence nothing is checked; with one, the task's process must be the fenced one.
        if state == super::validation_fence::FenceState::Absent {
            return Ok(());
        }
        let tgid = status
            .lines()
            .find_map(|line| line.strip_prefix("Tgid:"))
            .and_then(|value| value.trim().parse::<u32>().ok())
            .context("validation_fence_target_not_fenced: the task's process cannot be read")?;
        super::validation_fence::check_task(&state, proc_root, tgid)
    }

    pub(super) fn candidate(&self, candidate: &CandidateAction) -> anyhow::Result<()> {
        for identity in super::revalidate::candidate_task_identities(candidate) {
            self.task(Path::new("/proc"), identity.tid.as_u32())?;
        }
        Ok(())
    }

    pub(super) fn policy(
        candidate: &CandidateAction,
        policy: &crate::daemon_policy::DaemonPolicy,
    ) -> anyhow::Result<()> {
        if let CandidateAction::CgroupPlacement { plan } = candidate {
            ensure!(
                plan.action.cgroup_root == Path::new("/sys/fs/cgroup"),
                "bounded_service_cgroup_mount_denied"
            );
            ensure!(
                policy
                    .cgroup_targets
                    .contains_path(&plan.action.target_cgroup),
                "bounded_service_cgroup_destination_not_configured"
            );
            // The current registry contract moves tasks to an existing administrator cpuset.
            // A socket client cannot turn that into mutation of the destination's resource policy.
            ensure!(
                plan.action.cpuset_cpus.is_none() && plan.action.cpuset_mems.is_none(),
                "bounded_service_cpuset_policy_change_denied"
            );
        }
        Ok(())
    }

    pub(super) fn prepared(
        &self,
        candidate: &CandidateAction,
        token: &RollbackToken,
    ) -> anyhow::Result<()> {
        self.candidate(candidate)?;
        // The affinity executor expands the process tree during preparation. Inspect that
        // actual snapshot as well as the requested root, including threads and descendants.
        match token {
            RollbackToken::CpuAffinityRestoreFile { path, .. } => {
                let state = crate::profile_restore::load_restore_state(path)?;
                ensure!(
                    state.nice_records.is_empty() && state.ionice_records.is_empty(),
                    "bounded_service_unexpected_profile_actions"
                );
                for record in state.affinity_records {
                    self.task(Path::new("/proc"), record.tid.as_u32())?;
                }
            }
            RollbackToken::CgroupRestore { records, .. } => {
                for record in records {
                    // Automatic recovery restores only what Stutter demonstrably still owns, so a
                    // token that cannot name the placement it applied is refused up front.
                    ensure!(
                        record.applied_cgroup.is_some() && record.applied_cgroup_id.is_some(),
                        "bounded_service_cgroup_applied_identity_missing"
                    );
                    self.task(Path::new("/proc"), record.tid())?;
                }
            }
            // Other operation families require their own registry-qualified resource boundary.
            _ => {
                return Err(
                    PrivilegedWorkerError::BoundedServiceTargetBoundaryUnimplemented.into(),
                );
            }
        }
        Ok(())
    }

    fn task(&self, proc_root: &Path, tid: u32) -> anyhow::Result<()> {
        let root = proc_root.join(tid.to_string());
        let status = read_task_file(&root, "status", tid)?;
        let ids = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .context("bounded_service_target_uid_missing")?
            .split_whitespace()
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()?;
        ensure!(
            self.uid != 0 && ids.len() == 4 && ids.iter().all(|uid| *uid == self.uid),
            "bounded_service_target_uid_denied"
        );
        // A validation fence can only narrow: the task still had to pass the checks above.
        self.fence_check(proc_root, &status)?;
        if let Some(permitted) = &self.cgroup {
            let base = Path::new("/sys/fs/cgroup");
            ensure!(
                permitted.starts_with(base)
                    && permitted != base
                    && permitted
                        .components()
                        .all(|part| matches!(part, Component::RootDir | Component::Normal(_))),
                "bounded_service_invalid_workload_cgroup"
            );
            let membership = read_task_file(&root, "cgroup", tid)?;
            let relative = membership
                .lines()
                .find_map(|line| line.strip_prefix("0::"))
                .context("bounded_service_requires_unified_cgroup")?;
            ensure!(
                relative.starts_with('/') && !relative.contains(".."),
                "bounded_service_invalid_task_cgroup"
            );
            ensure!(
                base.join(relative.trim_start_matches('/'))
                    .starts_with(permitted),
                "bounded_service_target_outside_owned_cgroup"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cgroup_placement_cannot_expand_into_another_mount_destination_or_cpuset_mutation() {
        let mut policy = crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        );
        policy.cgroup_targets.game_cgroup = Some("/owned/game".into());
        let plan = crate::autotune::planning::executable_plan::CgroupPlacementActionPlan {
            name: "owned-game".into(),
            target_root_pid: Some(42),
            target_root_starttime_ticks: Some(100),
            evidence: vec![],
            objective: crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
            action: crate::actions::cgroup::CgroupPlacementAction {
                cgroup_root: "/sys/fs/cgroup".into(),
                target_cgroup: "/owned/game".into(),
                targets: vec![],
                cpuset_cpus: None,
                cpuset_mems: None,
            },
        };
        let candidate = CandidateAction::CgroupPlacement { plan: plan.clone() };
        ServiceTargets::policy(&candidate, &policy).unwrap();
        let mut changed = plan.clone();
        changed.action.cgroup_root = "/etc".into();
        assert!(
            ServiceTargets::policy(&CandidateAction::CgroupPlacement { plan: changed }, &policy)
                .is_err()
        );
        let mut changed = plan.clone();
        changed.action.target_cgroup = "/system.slice".into();
        assert!(
            ServiceTargets::policy(&CandidateAction::CgroupPlacement { plan: changed }, &policy)
                .is_err()
        );
        let mut changed = plan;
        changed.action.cpuset_cpus = Some("0".into());
        assert!(
            ServiceTargets::policy(&CandidateAction::CgroupPlacement { plan: changed }, &policy)
                .is_err()
        );
    }
    #[test]
    fn uid_and_component_bounded_cgroup_are_required_at_each_boundary() {
        let root = crate::test_support::TestRoot::new("service-target-owner");
        fs::create_dir_all(root.join("42")).unwrap();
        let status = root.join("42/status");
        let membership = root.join("42/cgroup");
        fs::write(&status, "Uid:\t1000\t1000\t1000\t1000\n").unwrap();
        fs::write(&membership, "0::/owned/game\n").unwrap();
        let boundary = ServiceTargets::new(1000, Some("/sys/fs/cgroup/owned".into()));
        boundary.task(root.path(), 42).unwrap();
        fs::write(&membership, "0::/owned-other/game\n").unwrap();
        assert!(boundary.task(root.path(), 42).is_err());
        fs::write(&membership, "0::/owned/game\n").unwrap();
        fs::write(&status, "Uid:\t1000\t0\t1000\t1000\n").unwrap();
        assert!(boundary.task(root.path(), 42).is_err());
        assert!(ServiceTargets::new(0, None).task(root.path(), 42).is_err());
    }
}

/// A task that vanished is process-tree churn, not a fault: nothing has been mutated for it, so
/// the candidate is stale and is abandoned. A live task that cannot be read is a real error.
fn read_task_file(root: &Path, name: &str, tid: u32) -> anyhow::Result<String> {
    match fs::read_to_string(root.join(name)) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(PrivilegedWorkerError::StaleTargetBeforeMutation {
                detail: format!("task {tid} exited before its first mutable write"),
            }
            .into())
        }
        Err(error) => Err(anyhow::Error::new(error).context(format!(
            "bounded_service_target_unreadable: task {tid} {name}"
        ))),
    }
}

#[cfg(test)]
mod churn_tests {
    use super::*;

    #[test]
    fn a_vanished_task_is_a_stale_target_and_a_foreign_one_is_still_denied() {
        let root = crate::test_support::TestRoot::new("service-target-churn");
        let targets = ServiceTargets::new(1000, None);
        // The task exited: no status file at all.
        let gone = targets.task(root.path(), 4242).unwrap_err();
        assert!(matches!(
            gone.downcast_ref::<PrivilegedWorkerError>(),
            Some(PrivilegedWorkerError::StaleTargetBeforeMutation { .. })
        ));
        // A live task of another account is a hard denial, never a stale target.
        let alive = root.join("4243");
        fs::create_dir_all(&alive).unwrap();
        fs::write(alive.join("status"), "Uid:\t0\t0\t0\t0\n").unwrap();
        let denied = targets.task(root.path(), 4243).unwrap_err();
        assert!(denied.downcast_ref::<PrivilegedWorkerError>().is_none());
        assert!(format!("{denied:#}").contains("bounded_service_target_uid_denied"));
    }
}

#[cfg(test)]
mod fence_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const NOW_OFFSET: u64 = 3600;

    fn me() -> u32 {
        crate::syscall::geteuid()
    }

    /// A fake `/proc` with two processes of the service account (500 with a thread 502, and 501),
    /// and a fence file in a directory only this user can write.
    struct World {
        root: crate::test_support::TestRoot,
        fence: PathBuf,
    }

    fn unix_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn world(name: &str) -> World {
        let root = crate::test_support::TestRoot::new(name);
        for (tid, tgid, ticks) in [(500, 500, 777), (501, 501, 800), (502, 500, 777)] {
            let dir = root.join(tid.to_string());
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("status"),
                format!("Uid:\t1000\t1000\t1000\t1000\nTgid:\t{tgid}\n"),
            )
            .unwrap();
            fs::write(
                dir.join("stat"),
                format!(
                    "{tid} (app) S 1 {tid} {tid} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0 0\n"
                ),
            )
            .unwrap();
        }
        fs::create_dir_all(root.join("sys/kernel/random")).unwrap();
        fs::write(root.join("sys/kernel/random/boot_id"), "boot-1\n").unwrap();
        let dir = root.join("state");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let fence = dir.join("validation-fence.json");
        World { root, fence }
    }

    fn write_fence(world: &World, pid: u32, ticks: u64, expires: u64) {
        fs::write(
            &world.fence,
            format!(
                r#"{{"version":1,"uid":1000,"root_pid":{pid},"start_ticks":{ticks},"boot_id":"boot-1","expires_unix":{expires}}}"#
            ),
        )
        .unwrap();
        fs::set_permissions(&world.fence, fs::Permissions::from_mode(0o644)).unwrap();
    }

    fn targets(world: &World) -> ServiceTargets {
        ServiceTargets::new(1000, None).with_fence(world.fence.clone(), me())
    }

    fn reason(error: anyhow::Error) -> String {
        format!("{error:#}")
    }

    #[test]
    fn without_a_fence_every_task_of_the_service_account_is_targetable_as_before() {
        let world = world("fence-none");
        // Not configured at all, and configured but with no file.
        ServiceTargets::new(1000, None)
            .task(world.root.path(), 501)
            .unwrap();
        targets(&world).task(world.root.path(), 501).unwrap();
        targets(&world).task(world.root.path(), 500).unwrap();
    }

    #[test]
    fn a_fence_narrows_the_targets_to_the_fenced_process_and_its_threads() {
        let world = world("fence-narrow");
        write_fence(&world, 500, 777, unix_now() + NOW_OFFSET);
        let targets = targets(&world);
        targets.task(world.root.path(), 500).unwrap();
        targets.task(world.root.path(), 502).unwrap();
        let stranger = reason(targets.task(world.root.path(), 501).unwrap_err());
        assert!(
            stranger.contains("validation_fence_target_not_fenced"),
            "{stranger}"
        );
    }

    #[test]
    fn a_fence_never_grants_what_the_ordinary_checks_refuse() {
        let world = world("fence-grants-nothing");
        write_fence(&world, 500, 777, unix_now() + NOW_OFFSET);
        // The fenced process now belongs to another account: still refused by the uid check.
        fs::write(
            world.root.join("500/status"),
            "Uid:\t0\t0\t0\t0\nTgid:\t500\n",
        )
        .unwrap();
        assert!(
            reason(targets(&world).task(world.root.path(), 500).unwrap_err())
                .contains("bounded_service_target_uid_denied")
        );
    }

    #[test]
    fn a_reused_pid_an_expired_fence_and_an_unreadable_one_refuse_even_the_fenced_task() {
        let world = world("fence-refuse");
        // The fence names pid 500 at a start time it does not have: a reused pid.
        write_fence(&world, 500, 111, unix_now() + NOW_OFFSET);
        assert!(
            reason(targets(&world).task(world.root.path(), 500).unwrap_err()).contains("reused")
        );
        // An expired fence is not ignored: it refuses everything until it is removed.
        write_fence(&world, 500, 777, unix_now() - 5);
        assert!(
            reason(targets(&world).task(world.root.path(), 500).unwrap_err())
                .contains("expired_remove_it")
        );
        // A fence that cannot be parsed, or that anyone could have written, refuses everything.
        fs::write(&world.fence, "garbage").unwrap();
        assert!(
            reason(targets(&world).task(world.root.path(), 500).unwrap_err())
                .contains("validation_fence_malformed")
        );
        write_fence(&world, 500, 777, unix_now() + NOW_OFFSET);
        fs::set_permissions(&world.fence, fs::Permissions::from_mode(0o666)).unwrap();
        assert!(
            reason(targets(&world).task(world.root.path(), 500).unwrap_err())
                .contains("validation_fence_untrusted")
        );
        // Removing the fence restores ordinary operation.
        fs::remove_file(&world.fence).unwrap();
        targets(&world).task(world.root.path(), 501).unwrap();
    }
}
