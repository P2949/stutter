//! The one mutation an acceptance suite drives against its owned disposable process tree.
//! Every checkpoint, failure and suspend/resume case is expressed against this type, so a new
//! operation joins the fault matrix by adding a variant rather than a copy of the harness.
use std::path::PathBuf;

use anyhow::Context;

use super::planning::{
    candidate::{CandidateAction, CandidateEvidence},
    executable_plan::{
        CgroupPlacementActionPlan, CpuPowerActionPlan, NiceActionPlan, UclampActionPlan,
    },
};
use crate::{
    actions::{
        SafetyClass, TaskIdentity, TuningAction,
        cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
        cpu_affinity::CpuAffinityProfileAction,
        cpu_power::CpuPowerAction,
        nice::{NiceAction, NicePolicy},
        runner::ActionRunPolicy,
        uclamp::{UclampAction, UclampValues},
    },
    daemon::policy::DaemonMode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcceptanceMutation {
    Affinity,
    Uclamp,
    Nice,
    Cgroup,
    CpuPowerEpp,
}

/// Where a CPU-power acceptance case keeps the sysfs fixture it mutates. The fault matrix drives
/// the real transaction, journal and recovery code against a faithful per-task `cpufreq` tree
/// (never `/sys`), so every checkpoint can be killed and re-observed without root.
pub const ACCEPTANCE_SYSFS_ENV: &str = "STUTTER_ACCEPTANCE_SYSFS_ROOT";

/// Creates the `cpu<id>/cpufreq` fixture for each id; the owned tree's task ids stand in for CPUs.
pub fn prepare_acceptance_sysfs(root: &std::path::Path, ids: &[u32]) -> anyhow::Result<()> {
    for id in ids {
        let dir = root.join(format!("devices/system/cpu/cpu{id}/cpufreq"));
        std::fs::create_dir_all(&dir)?;
        for (file, value) in [
            ("energy_performance_preference", "balance_performance"),
            (
                "energy_performance_available_preferences",
                "default performance balance_performance balance_power power",
            ),
            ("scaling_governor", "powersave"),
            ("scaling_driver", "acceptance-pstate"),
        ] {
            std::fs::write(dir.join(file), format!("{value}\n"))?;
        }
        std::fs::write(dir.join("related_cpus"), format!("{id}\n"))?;
    }
    Ok(())
}

fn acceptance_sysfs_root() -> anyhow::Result<PathBuf> {
    std::env::var_os(ACCEPTANCE_SYSFS_ENV)
        .map(PathBuf::from)
        .with_context(|| format!("{ACCEPTANCE_SYSFS_ENV} is not set"))
}

/// The medium-risk policy of the validation harness: the explicit medium-risk unlock and, for a
/// system-wide operation, the scope permission plus the exact operation on the allowlist.
pub fn acceptance_policy(mutation: AcceptanceMutation) -> crate::daemon::DaemonPolicy {
    let source = crate::daemon_policy::ActionSource::AutotuneRuntime;
    let mut config = crate::daemon::DaemonConfig {
        mode: mutation.mode(),
        source,
        ..crate::daemon::DaemonConfig::default()
    };
    config.autotune.allow_medium_risk_apply = mutation.mode() == DaemonMode::ApplyMediumRisk;
    if mutation == AcceptanceMutation::CpuPowerEpp {
        config.safety.allow_system_wide_apply = true;
        config
            .safety
            .system_wide_allowlist
            .operations
            .insert("cpu_power_epp".to_owned());
    }
    crate::daemon_policy::build_daemon_policy(crate::daemon_policy::DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

impl AcceptanceMutation {
    pub const ALL: [Self; 5] = [
        Self::Affinity,
        Self::Uclamp,
        Self::Nice,
        Self::Cgroup,
        Self::CpuPowerEpp,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Affinity => "affinity",
            Self::Uclamp => "uclamp",
            Self::Nice => "nice",
            Self::Cgroup => "cgroup",
            Self::CpuPowerEpp => "cpu_power_epp",
        }
    }

    /// The syscall the harness tracer injects faults and stalls into.
    pub fn syscall(self) -> &'static str {
        match self {
            Self::Affinity => "sched_setaffinity",
            Self::Uclamp => "sched_setattr",
            Self::Nice => "setpriority",
            // Moving a task is a write of its id to the destination cgroup's `cgroup.procs`; the
            // tracer filters on that path.
            Self::Cgroup | Self::CpuPowerEpp => "write",
        }
    }

    /// How many writes one apply of the owned tree performs (a cgroup move is one write per
    /// process, the three processes of the tree; every other mutation writes once per task).
    pub fn apply_writes(self) -> usize {
        match self {
            Self::Cgroup => 3,
            _ => 5,
        }
    }

    pub fn mode(self) -> DaemonMode {
        match self {
            Self::Affinity => DaemonMode::ApplyLowRisk,
            Self::Uclamp | Self::Nice | Self::Cgroup | Self::CpuPowerEpp => {
                DaemonMode::ApplyMediumRisk
            }
        }
    }

    pub fn safety_class(self) -> SafetyClass {
        match self {
            Self::Affinity => SafetyClass::ReversibleLowRisk,
            Self::Uclamp | Self::Nice | Self::Cgroup | Self::CpuPowerEpp => {
                SafetyClass::ReversibleMediumRisk
            }
        }
    }

    /// The default value the mutation applies: a CPU number, a `uclamp.min` or a niceness.
    pub fn default_value(self) -> u32 {
        match self {
            Self::Affinity => 0,
            Self::Uclamp => 256,
            Self::Nice => 10,
            // The cgroup is chosen by the harness, not by a number.
            Self::Cgroup | Self::CpuPowerEpp => 0,
        }
    }

    pub fn candidate_name(self) -> &'static str {
        match self {
            Self::Affinity => "crash-acceptance-affinity",
            Self::Uclamp => "crash-acceptance-uclamp",
            Self::Nice => "crash-acceptance-nice",
            Self::Cgroup => "crash-acceptance-cgroup",
            Self::CpuPowerEpp => "crash-acceptance-cpu-power-epp",
        }
    }
}

impl std::str::FromStr for AcceptanceMutation {
    type Err = anyhow::Error;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mutation| mutation.as_str() == value)
            .with_context(|| format!("unknown acceptance mutation {value}"))
    }
}

/// A mutation and the value it applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcceptanceTarget {
    pub mutation: AcceptanceMutation,
    pub value: u32,
}

impl AcceptanceTarget {
    pub fn policy(self) -> ActionRunPolicy {
        let source = crate::daemon_policy::ActionSource::AutotuneRuntime;
        let mut policy = match self.mutation.mode() {
            DaemonMode::ApplyMediumRisk => {
                let mut policy = ActionRunPolicy::apply_medium_risk(source, false);
                // The explicit medium-risk unlock the validation harness is granted.
                policy.policy = acceptance_policy(self.mutation);
                policy
            }
            _ => ActionRunPolicy::apply_low_risk(source, false),
        };
        policy.max_affected_tasks = Some(8);
        policy
    }

    /// The action itself, bound to the tasks of the owned tree.
    pub fn action(
        self,
        tree_pid: u32,
        restore_path: Option<PathBuf>,
    ) -> anyhow::Result<Box<dyn TuningAction>> {
        Ok(match self.mutation {
            AcceptanceMutation::Affinity => Box::new(CpuAffinityProfileAction {
                tree_pid,
                restore_path,
                force_restore_overwrite: false,
                profile: affinity_profile(self.value)?,
            }),
            AcceptanceMutation::Uclamp => Box::new(self.uclamp_action(tree_pid)),
            AcceptanceMutation::Nice => Box::new(self.nice_action(tree_pid)),
            AcceptanceMutation::Cgroup => Box::new(self.cgroup_action(tree_pid)?),
            AcceptanceMutation::CpuPowerEpp => crate::actions::default_action_factory_registry()
                .build(&self.candidate(tree_pid)?)
                .map_err(anyhow::Error::new)?,
        })
    }

    fn tree_targets(tree_pid: u32) -> Vec<TaskIdentity> {
        crate::process_tree::target_snapshot(
            crate::process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
        )
        .tasks
        .values()
        .map(TaskIdentity::from_task_info)
        .collect()
    }

    /// Moves the owned tree into a sibling `target` of the leaf it runs in. The runner delegated the
    /// parent subtree to this user, so the harness may create the target and the user may move
    /// tasks between the two.
    fn cgroup_action(self, tree_pid: u32) -> anyhow::Result<CgroupPlacementAction> {
        let cgroup_root = std::path::PathBuf::from("/sys/fs/cgroup");
        let current = std::fs::read_to_string(format!("/proc/{tree_pid}/cgroup"))?;
        let original = current
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .context("the tree has no cgroup v2 path")?
            .trim()
            .to_owned();
        let parent = std::path::Path::new(&original)
            .parent()
            .filter(|parent| parent.components().count() > 1)
            .context("the tree does not run in a delegated cgroup subtree")?
            .to_path_buf();
        let target = parent.join("target");
        std::fs::create_dir_all(cgroup_root.join(target.strip_prefix("/")?))
            .context("cannot create the target cgroup inside the delegated subtree")?;
        Ok(CgroupPlacementAction {
            cgroup_root,
            target_cgroup: target,
            targets: Self::tree_targets(tree_pid)
                .into_iter()
                .map(|identity| CgroupPlacementTarget {
                    identity,
                    class: crate::process_tree::TaskClass::Game,
                })
                .collect(),
            cpuset_cpus: None,
            cpuset_mems: None,
        })
    }

    fn epp_action(self, tree_pid: u32) -> anyhow::Result<CpuPowerAction> {
        Ok(CpuPowerAction {
            sysfs_root: acceptance_sysfs_root()?,
            cpus: Self::tree_targets(tree_pid)
                .iter()
                .map(|identity| identity.tid.as_u32())
                .collect(),
            scaling_governor: None,
            energy_performance_preference: Some("performance".to_owned()),
        })
    }

    fn nice_action(self, tree_pid: u32) -> NiceAction {
        NiceAction {
            targets: Self::tree_targets(tree_pid),
            nice: i32::try_from(self.value).unwrap_or(10),
            policy: NicePolicy::default(),
        }
    }

    fn uclamp_action(self, tree_pid: u32) -> UclampAction {
        let snapshot = crate::process_tree::target_snapshot(
            crate::process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
        );
        UclampAction {
            targets: snapshot
                .tasks
                .values()
                .map(TaskIdentity::from_task_info)
                .collect(),
            values: UclampValues {
                sched_util_min: Some(self.value),
                sched_util_max: None,
            },
        }
    }

    /// The candidate every acceptance case applies: the final control path, where scheduler
    /// pressure on a game is judged by its runnable latency.
    pub fn candidate(self, tree_pid: u32) -> anyhow::Result<CandidateAction> {
        let objective = super::objective::ObjectiveKind::GameRunnableLatency;
        Ok(match self.mutation {
            AcceptanceMutation::Affinity => CandidateAction::cpu_affinity_profile_for(
                affinity_profile(self.value)?,
                tree_pid,
                objective,
            ),
            AcceptanceMutation::CpuPowerEpp => CandidateAction::CpuPower {
                plan: CpuPowerActionPlan {
                    name: self.mutation.candidate_name().to_owned(),
                    action: self.epp_action(tree_pid)?,
                    evidence: vec![CandidateEvidence::new(
                        "situation",
                        "GameCpuSchedulerPressure",
                        1.0,
                    )],
                    objective,
                },
            },
            AcceptanceMutation::Cgroup => CandidateAction::CgroupPlacement {
                plan: CgroupPlacementActionPlan {
                    name: self.mutation.candidate_name().to_owned(),
                    action: self.cgroup_action(tree_pid)?,
                    target_root_pid: Some(tree_pid),
                    evidence: vec![CandidateEvidence::new(
                        "situation",
                        "GameCpuSchedulerPressure",
                        1.0,
                    )],
                    objective,
                },
            },
            AcceptanceMutation::Nice => CandidateAction::Nice {
                plan: NiceActionPlan {
                    name: self.mutation.candidate_name().to_owned(),
                    action: self.nice_action(tree_pid),
                    target_root_pid: Some(tree_pid),
                    evidence: vec![CandidateEvidence::new(
                        "situation",
                        "GameCpuSchedulerPressure",
                        1.0,
                    )],
                    objective,
                },
            },
            AcceptanceMutation::Uclamp => CandidateAction::Uclamp {
                plan: UclampActionPlan {
                    name: self.mutation.candidate_name().to_owned(),
                    action: self.uclamp_action(tree_pid),
                    target_root_pid: Some(tree_pid),
                    evidence: vec![CandidateEvidence::new(
                        "situation",
                        "GameCpuSchedulerPressure",
                        1.0,
                    )],
                    objective,
                },
            },
        })
    }
}

fn affinity_profile(cpu: u32) -> anyhow::Result<crate::profiles::Profile> {
    Ok(crate::profiles::Profile {
        name: AcceptanceMutation::Affinity.candidate_name().into(),
        rules: vec![crate::profiles::ProfileRule {
            affinity: Some(crate::affinity::CpuMask::parse(&cpu.to_string())?),
            nice: None,
            ionice: None,
            match_class: vec![],
            match_comm: vec![crate::process_tree::CompiledPattern::new("/.*/".into())?],
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unknown_is_rejected() {
        for mutation in AcceptanceMutation::ALL {
            assert_eq!(
                mutation.as_str().parse::<AcceptanceMutation>().unwrap(),
                mutation
            );
        }
        assert!("gpu_power".parse::<AcceptanceMutation>().is_err());
    }

    #[test]
    fn risk_follows_the_operation() {
        assert_eq!(
            AcceptanceMutation::Affinity.mode(),
            DaemonMode::ApplyLowRisk
        );
        assert_eq!(
            AcceptanceMutation::Uclamp.mode(),
            DaemonMode::ApplyMediumRisk
        );
        let uclamp = AcceptanceTarget {
            mutation: AcceptanceMutation::Uclamp,
            value: 256,
        };
        assert_eq!(
            uclamp.candidate(1).unwrap().safety_class(),
            AcceptanceMutation::Uclamp.safety_class()
        );
    }
}
