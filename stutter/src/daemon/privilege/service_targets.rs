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
}

impl ServiceTargets {
    pub(super) fn new(uid: u32, cgroup: Option<PathBuf>) -> Self {
        Self { uid, cgroup }
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
        let status = fs::read_to_string(root.join("status"))?;
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
            let membership = fs::read_to_string(root.join("cgroup"))?;
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
