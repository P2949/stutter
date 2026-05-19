use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};

use crate::{
    actions::uclamp::UclampValues,
    affinity::CpuMask,
    autotune::{
        candidate::{CandidateAction, CpuAffinityProfilePlan},
        observation::{
            ActiveAffinitySnapshot, ActiveCgroupSnapshot, ActiveConfigSnapshot,
            ActiveCpuPowerSnapshot, ActiveGpuPowerSnapshot, ActiveIoPrioSnapshot,
            ActiveIrqSnapshot, ActiveNiceSnapshot, ActiveTaskSnapshot, ActiveUclampSnapshot,
            ActiveVmSnapshot, CpuPolicyRuntimeState, GpuPowerRuntimeState,
        },
    },
    daemon::capabilities::DaemonCapabilities,
    profiles::{ProfileEvaluationInput, evaluate_profile_for_tasks},
    system_inventory::SystemInventory,
};

#[derive(Clone, Copy, Debug)]
pub struct ActiveConfigCollectorInput<'a> {
    pub proc_root: &'a Path,
    pub sys_root: &'a Path,
    pub active_tasks: &'a [ActiveTaskSnapshot],
    pub capabilities: &'a DaemonCapabilities,
    pub inventory: &'a SystemInventory,
}

#[derive(Clone, Debug, Default)]
pub struct ActiveConfigCollector;

impl ActiveConfigCollector {
    pub fn collect(&self, input: ActiveConfigCollectorInput<'_>) -> ActiveConfigSnapshot {
        let tids = sorted_active_tids(input.active_tasks);

        ActiveConfigSnapshot {
            affinity: collect_affinity(input.proc_root, &tids),
            nice: collect_nice(input.proc_root, &tids),
            ionice: if input.capabilities.ionice_available {
                collect_ioprio(input.proc_root, &tids)
            } else {
                ActiveIoPrioSnapshot::default()
            },
            uclamp: if input.capabilities.uclamp_available {
                collect_uclamp(input.proc_root, &tids)
            } else {
                ActiveUclampSnapshot::default()
            },
            cgroup: collect_cgroups(input.proc_root, &tids),
            irq: if input.capabilities.irq_affinity_available {
                collect_irq(input.proc_root)
            } else {
                ActiveIrqSnapshot::default()
            },
            cpu_power: collect_cpu_power(input.inventory),
            gpu_power: if input.capabilities.gpu_sysfs_available {
                collect_gpu_power(input.sys_root, input.inventory)
            } else {
                ActiveGpuPowerSnapshot::default()
            },
            vm: collect_vm(input.inventory),
        }
    }
}

pub fn collect_active_config(input: ActiveConfigCollectorInput<'_>) -> ActiveConfigSnapshot {
    ActiveConfigCollector.collect(input)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveConfigMatch {
    Matches { summary: String },
    Differs { expected: String, actual: String },
    Unknown { summary: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackVerification {
    pub verified: bool,
    pub expected: String,
    pub actual: String,
    pub reason_code: String,
}

impl ActiveConfigMatch {
    pub fn is_match(&self) -> bool {
        matches!(self, Self::Matches { .. })
    }

    pub fn is_differs(&self) -> bool {
        matches!(self, Self::Differs { .. })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ActiveConfigMatchInput<'a> {
    pub snapshot: &'a ActiveConfigSnapshot,
    pub active_tasks: &'a [ActiveTaskSnapshot],
}

impl CandidateAction {
    pub fn planned_state_summary(&self) -> String {
        match self {
            CandidateAction::CpuAffinityProfile { plan } => format!(
                "cpu_affinity_profile profile={} tree_pid={}",
                plan.profile_name, plan.tree_pid
            ),
            CandidateAction::Nice { plan } => format!(
                "nice value={} targets={}",
                plan.action.nice,
                plan.action.targets.len()
            ),
            CandidateAction::IoPrio { plan } => format!(
                "ionice value={} targets={}",
                plan.action.ioprio.label(),
                plan.action.targets.len()
            ),
            CandidateAction::Uclamp { plan } => format!(
                "uclamp min={:?} max={:?} targets={}",
                plan.action.values.sched_util_min,
                plan.action.values.sched_util_max,
                plan.action.targets.len()
            ),
            CandidateAction::CgroupPlacement { plan } => format!(
                "cgroup target={} targets={}",
                plan.action.target_cgroup.display(),
                plan.action.targets.len()
            ),
            CandidateAction::IrqAffinity { plan } => format!(
                "irq_affinity irq={} smp_affinity={}",
                plan.action.irq, plan.action.smp_affinity
            ),
            CandidateAction::CpuPower { plan } => format!(
                "cpu_power cpus={:?} governor={:?} epp={:?}",
                plan.action.cpus,
                plan.action.scaling_governor,
                plan.action.energy_performance_preference
            ),
            CandidateAction::GpuPower { plan } => format!(
                "gpu_power drm_card={} dpm={:?} profile={:?}",
                plan.action.drm_card,
                plan.action.power_dpm_force_performance_level,
                plan.action.pp_power_profile_mode
            ),
            CandidateAction::VmKnob { plan } => {
                format!("vm_knob changes={}", plan.action.changes.len())
            }
            CandidateAction::Fake { plan } => {
                format!("fake action_id={}", plan.action_id.as_str())
            }
        }
    }

    pub fn matches_active_config(&self, input: ActiveConfigMatchInput<'_>) -> ActiveConfigMatch {
        let snapshot = input.snapshot;
        match self {
            CandidateAction::CpuAffinityProfile { plan } => {
                cpu_affinity_profile_match(plan, snapshot, input.active_tasks)
            }
            CandidateAction::Nice { plan } => {
                if plan.action.targets.is_empty() {
                    return ActiveConfigMatch::Unknown {
                        summary: "nice candidate has no target tasks".to_owned(),
                    };
                }

                for target in &plan.action.targets {
                    match snapshot.nice.per_tid.get(&target.tid) {
                        Some(current) if *current == plan.action.nice => {}
                        Some(current) => {
                            return ActiveConfigMatch::Differs {
                                expected: format!("tid={} nice={}", target.tid, plan.action.nice),
                                actual: format!("tid={} nice={current}", target.tid),
                            };
                        }
                        None => {
                            return ActiveConfigMatch::Unknown {
                                summary: format!("tid={} active nice value missing", target.tid),
                            };
                        }
                    }
                }

                ActiveConfigMatch::Matches {
                    summary: self.planned_state_summary(),
                }
            }
            CandidateAction::IoPrio { plan } => {
                if plan.action.targets.is_empty() {
                    return ActiveConfigMatch::Unknown {
                        summary: "ionice candidate has no target tasks".to_owned(),
                    };
                }

                let requested = plan.action.ioprio.label();
                for target in &plan.action.targets {
                    match snapshot.ionice.per_tid.get(&target.tid) {
                        Some(current) if current == &requested => {}
                        Some(current) => {
                            return ActiveConfigMatch::Differs {
                                expected: format!("tid={} ionice={requested}", target.tid),
                                actual: format!("tid={} ionice={current}", target.tid),
                            };
                        }
                        None => {
                            return ActiveConfigMatch::Unknown {
                                summary: format!("tid={} active ionice value missing", target.tid),
                            };
                        }
                    }
                }

                ActiveConfigMatch::Matches {
                    summary: self.planned_state_summary(),
                }
            }
            CandidateAction::Uclamp { plan } => {
                if plan.action.targets.is_empty() {
                    return ActiveConfigMatch::Unknown {
                        summary: "uclamp candidate has no target tasks".to_owned(),
                    };
                }

                for target in &plan.action.targets {
                    match snapshot.uclamp.per_tid.get(&target.tid) {
                        Some(current) if uclamp_matches_request(*current, plan.action.values) => {}
                        Some(current) => {
                            return ActiveConfigMatch::Differs {
                                expected: format!(
                                    "tid={} uclamp_min={:?} uclamp_max={:?}",
                                    target.tid,
                                    plan.action.values.sched_util_min,
                                    plan.action.values.sched_util_max
                                ),
                                actual: format!(
                                    "tid={} uclamp_min={:?} uclamp_max={:?}",
                                    target.tid, current.sched_util_min, current.sched_util_max
                                ),
                            };
                        }
                        None => {
                            return ActiveConfigMatch::Unknown {
                                summary: format!("tid={} active uclamp value missing", target.tid),
                            };
                        }
                    }
                }

                ActiveConfigMatch::Matches {
                    summary: self.planned_state_summary(),
                }
            }
            CandidateAction::CgroupPlacement { plan } => {
                if plan.action.targets.is_empty() {
                    return ActiveConfigMatch::Unknown {
                        summary: "cgroup candidate has no target tasks".to_owned(),
                    };
                }

                let requested = normalize_cgroup_path(&plan.action.target_cgroup);
                for target in &plan.action.targets {
                    match snapshot.cgroup.per_tid.get(&target.identity.tid) {
                        Some(current) if normalize_cgroup_str(current) == requested => {}
                        Some(current) => {
                            return ActiveConfigMatch::Differs {
                                expected: format!("tid={} cgroup={requested}", target.identity.tid),
                                actual: format!(
                                    "tid={} cgroup={}",
                                    target.identity.tid,
                                    normalize_cgroup_str(current)
                                ),
                            };
                        }
                        None => {
                            return ActiveConfigMatch::Unknown {
                                summary: format!(
                                    "tid={} active cgroup value missing",
                                    target.identity.tid
                                ),
                            };
                        }
                    }
                }

                ActiveConfigMatch::Matches {
                    summary: self.planned_state_summary(),
                }
            }
            CandidateAction::IrqAffinity { plan } => {
                match snapshot.irq.per_irq.get(&plan.action.irq) {
                    Some(current) if current.trim() == plan.action.smp_affinity.trim() => {
                        ActiveConfigMatch::Matches {
                            summary: self.planned_state_summary(),
                        }
                    }
                    Some(current) => ActiveConfigMatch::Differs {
                        expected: format!(
                            "irq={} smp_affinity={}",
                            plan.action.irq, plan.action.smp_affinity
                        ),
                        actual: format!("irq={} smp_affinity={}", plan.action.irq, current.trim()),
                    },
                    None => ActiveConfigMatch::Unknown {
                        summary: format!("irq={} active smp_affinity missing", plan.action.irq),
                    },
                }
            }
            CandidateAction::CpuPower { plan } => {
                if plan.action.cpus.is_empty() {
                    return ActiveConfigMatch::Unknown {
                        summary: "cpu_power candidate has no CPUs".to_owned(),
                    };
                }

                if plan.action.scaling_governor.is_none()
                    && plan.action.energy_performance_preference.is_none()
                {
                    return ActiveConfigMatch::Unknown {
                        summary: "cpu_power candidate has no requested runtime state".to_owned(),
                    };
                }

                for cpu in &plan.action.cpus {
                    let Some(policy) = cpu_policy_for_cpu(&snapshot.cpu_power.policies, *cpu)
                    else {
                        return ActiveConfigMatch::Unknown {
                            summary: format!("cpu={cpu} active CPU policy missing"),
                        };
                    };

                    if let Some(requested) = &plan.action.scaling_governor
                        && policy.scaling_governor.as_ref() != Some(requested)
                    {
                        return ActiveConfigMatch::Differs {
                            expected: format!("cpu={cpu} scaling_governor={requested}"),
                            actual: format!(
                                "cpu={cpu} scaling_governor={:?}",
                                policy.scaling_governor
                            ),
                        };
                    }

                    if let Some(requested) = &plan.action.energy_performance_preference
                        && policy.energy_performance_preference.as_ref() != Some(requested)
                    {
                        return ActiveConfigMatch::Differs {
                            expected: format!(
                                "cpu={cpu} energy_performance_preference={requested}"
                            ),
                            actual: format!(
                                "cpu={cpu} energy_performance_preference={:?}",
                                policy.energy_performance_preference
                            ),
                        };
                    }
                }

                ActiveConfigMatch::Matches {
                    summary: self.planned_state_summary(),
                }
            }
            CandidateAction::GpuPower { plan } => {
                let Some(device) = snapshot
                    .gpu_power
                    .devices
                    .iter()
                    .find(|device| device.device == plan.action.drm_card)
                else {
                    return ActiveConfigMatch::Unknown {
                        summary: format!("gpu={} active power state missing", plan.action.drm_card),
                    };
                };

                if plan.action.power_dpm_force_performance_level.is_none()
                    && plan.action.pp_power_profile_mode.is_none()
                {
                    return ActiveConfigMatch::Unknown {
                        summary: "gpu_power candidate has no requested runtime state".to_owned(),
                    };
                }

                if let Some(requested) = &plan.action.power_dpm_force_performance_level
                    && device.power_dpm_force_performance_level.as_ref() != Some(requested)
                {
                    return ActiveConfigMatch::Differs {
                        expected: format!(
                            "gpu={} power_dpm_force_performance_level={requested}",
                            plan.action.drm_card
                        ),
                        actual: format!(
                            "gpu={} power_dpm_force_performance_level={:?}",
                            plan.action.drm_card, device.power_dpm_force_performance_level
                        ),
                    };
                }

                if let Some(requested) = &plan.action.pp_power_profile_mode
                    && device.pp_power_profile_mode.as_ref() != Some(requested)
                {
                    return ActiveConfigMatch::Differs {
                        expected: format!(
                            "gpu={} pp_power_profile_mode={requested}",
                            plan.action.drm_card
                        ),
                        actual: format!(
                            "gpu={} pp_power_profile_mode={:?}",
                            plan.action.drm_card, device.pp_power_profile_mode
                        ),
                    };
                }

                ActiveConfigMatch::Matches {
                    summary: self.planned_state_summary(),
                }
            }
            CandidateAction::VmKnob { plan } => {
                if plan.action.changes.is_empty() {
                    return ActiveConfigMatch::Unknown {
                        summary: "vm_knob candidate has no changes".to_owned(),
                    };
                }

                for change in &plan.action.changes {
                    let keys = vm_knob_keys_for_change(&plan.action.root, &change.path);
                    match vm_knob_active_value(&snapshot.vm.knobs, &keys) {
                        Some(current) if current == &change.value => {}
                        Some(current) => {
                            return ActiveConfigMatch::Differs {
                                expected: format!(
                                    "vm_knob {}={}",
                                    change.path.display(),
                                    change.value
                                ),
                                actual: format!("vm_knob {}={current}", change.path.display()),
                            };
                        }
                        None => {
                            return ActiveConfigMatch::Unknown {
                                summary: format!(
                                    "vm_knob {} active value missing",
                                    change.path.display()
                                ),
                            };
                        }
                    }
                }

                ActiveConfigMatch::Matches {
                    summary: self.planned_state_summary(),
                }
            }
            CandidateAction::Fake { .. } => ActiveConfigMatch::Unknown {
                summary: self.planned_state_summary(),
            },
        }
    }
}

#[cfg(test)]
pub fn candidate_is_noop(candidate: &CandidateAction, snapshot: &ActiveConfigSnapshot) -> bool {
    candidate
        .matches_active_config(ActiveConfigMatchInput {
            snapshot,
            active_tasks: &[],
        })
        .is_match()
}

#[cfg(test)]
pub fn candidate_is_noop_with_tasks(
    candidate: &CandidateAction,
    snapshot: &ActiveConfigSnapshot,
    active_tasks: &[ActiveTaskSnapshot],
) -> bool {
    candidate
        .matches_active_config(ActiveConfigMatchInput {
            snapshot,
            active_tasks,
        })
        .is_match()
}

pub fn verify_rollback_restored_baseline(
    candidate: &CandidateAction,
    baseline: &ActiveConfigSnapshot,
    post_rollback: &ActiveConfigSnapshot,
    active_tasks: &[ActiveTaskSnapshot],
) -> RollbackVerification {
    match candidate {
        CandidateAction::CpuAffinityProfile { plan } => {
            verify_cpu_affinity_rollback(plan, baseline, post_rollback, active_tasks)
        }
        CandidateAction::Nice { plan } => {
            if plan.action.targets.is_empty() {
                return RollbackVerification::unavailable("nice rollback has no target tasks");
            }

            for target in &plan.action.targets {
                if let Some(verification) = compare_required_text(
                    format!("tid={} nice", target.tid),
                    baseline.nice.per_tid.get(&target.tid).map(i32::to_string),
                    post_rollback
                        .nice
                        .per_tid
                        .get(&target.tid)
                        .map(i32::to_string),
                ) {
                    return verification;
                }
            }

            RollbackVerification::verified(format!(
                "nice rollback restored {} target(s)",
                plan.action.targets.len()
            ))
        }
        CandidateAction::IoPrio { plan } => {
            if plan.action.targets.is_empty() {
                return RollbackVerification::unavailable("ionice rollback has no target tasks");
            }

            for target in &plan.action.targets {
                if let Some(verification) = compare_required_text(
                    format!("tid={} ionice", target.tid),
                    baseline.ionice.per_tid.get(&target.tid).cloned(),
                    post_rollback.ionice.per_tid.get(&target.tid).cloned(),
                ) {
                    return verification;
                }
            }

            RollbackVerification::verified(format!(
                "ionice rollback restored {} target(s)",
                plan.action.targets.len()
            ))
        }
        CandidateAction::Uclamp { plan } => {
            if plan.action.targets.is_empty() {
                return RollbackVerification::unavailable("uclamp rollback has no target tasks");
            }

            for target in &plan.action.targets {
                if let Some(verification) = compare_required_text(
                    format!("tid={} uclamp", target.tid),
                    baseline
                        .uclamp
                        .per_tid
                        .get(&target.tid)
                        .map(format_debug_value),
                    post_rollback
                        .uclamp
                        .per_tid
                        .get(&target.tid)
                        .map(format_debug_value),
                ) {
                    return verification;
                }
            }

            RollbackVerification::verified(format!(
                "uclamp rollback restored {} target(s)",
                plan.action.targets.len()
            ))
        }
        CandidateAction::CgroupPlacement { plan } => {
            if plan.action.targets.is_empty() {
                return RollbackVerification::unavailable("cgroup rollback has no target tasks");
            }

            for target in &plan.action.targets {
                if let Some(verification) = compare_required_text(
                    format!("tid={} cgroup", target.identity.tid),
                    baseline
                        .cgroup
                        .per_tid
                        .get(&target.identity.tid)
                        .map(|value| normalize_cgroup_str(value)),
                    post_rollback
                        .cgroup
                        .per_tid
                        .get(&target.identity.tid)
                        .map(|value| normalize_cgroup_str(value)),
                ) {
                    return verification;
                }
            }

            RollbackVerification::verified(format!(
                "cgroup rollback restored {} target(s)",
                plan.action.targets.len()
            ))
        }
        CandidateAction::IrqAffinity { plan } => compare_required_text(
            format!("irq={} smp_affinity", plan.action.irq),
            baseline
                .irq
                .per_irq
                .get(&plan.action.irq)
                .map(|value| value.trim().to_owned()),
            post_rollback
                .irq
                .per_irq
                .get(&plan.action.irq)
                .map(|value| value.trim().to_owned()),
        )
        .unwrap_or_else(|| {
            RollbackVerification::verified(format!("irq rollback restored irq={}", plan.action.irq))
        }),
        CandidateAction::CpuPower { plan } => {
            if plan.action.cpus.is_empty() {
                return RollbackVerification::unavailable("cpu_power rollback has no CPUs");
            }
            if plan.action.scaling_governor.is_none()
                && plan.action.energy_performance_preference.is_none()
            {
                return RollbackVerification::unavailable(
                    "cpu_power rollback has no requested runtime state",
                );
            }

            for cpu in &plan.action.cpus {
                let Some(baseline_policy) = cpu_policy_for_cpu(&baseline.cpu_power.policies, *cpu)
                else {
                    return RollbackVerification::unavailable(format!(
                        "cpu={cpu} baseline CPU policy missing"
                    ));
                };
                let Some(actual_policy) =
                    cpu_policy_for_cpu(&post_rollback.cpu_power.policies, *cpu)
                else {
                    return RollbackVerification::mismatch(
                        format!("cpu={cpu} policy={}", baseline_policy.policy),
                        format!("cpu={cpu} policy=missing"),
                        "rollback_target_missing",
                    );
                };

                if plan.action.scaling_governor.is_some()
                    && let Some(verification) = compare_required_text(
                        format!("cpu={cpu} scaling_governor"),
                        baseline_policy.scaling_governor.clone(),
                        actual_policy.scaling_governor.clone(),
                    )
                {
                    return verification;
                }

                if plan.action.energy_performance_preference.is_some()
                    && let Some(verification) = compare_required_text(
                        format!("cpu={cpu} energy_performance_preference"),
                        baseline_policy.energy_performance_preference.clone(),
                        actual_policy.energy_performance_preference.clone(),
                    )
                {
                    return verification;
                }
            }

            RollbackVerification::verified(format!(
                "cpu_power rollback restored {} CPU(s)",
                plan.action.cpus.len()
            ))
        }
        CandidateAction::GpuPower { plan } => {
            let Some(baseline_device) = baseline
                .gpu_power
                .devices
                .iter()
                .find(|device| device.device == plan.action.drm_card)
            else {
                return RollbackVerification::unavailable(format!(
                    "gpu={} baseline power state missing",
                    plan.action.drm_card
                ));
            };
            let Some(actual_device) = post_rollback
                .gpu_power
                .devices
                .iter()
                .find(|device| device.device == plan.action.drm_card)
            else {
                return RollbackVerification::mismatch(
                    format!("gpu={} power_state=present", plan.action.drm_card),
                    format!("gpu={} power_state=missing", plan.action.drm_card),
                    "rollback_target_missing",
                );
            };

            if plan.action.power_dpm_force_performance_level.is_some()
                && let Some(verification) = compare_required_text(
                    format!(
                        "gpu={} power_dpm_force_performance_level",
                        plan.action.drm_card
                    ),
                    baseline_device.power_dpm_force_performance_level.clone(),
                    actual_device.power_dpm_force_performance_level.clone(),
                )
            {
                return verification;
            }

            if plan.action.pp_power_profile_mode.is_some()
                && let Some(verification) = compare_required_text(
                    format!("gpu={} pp_power_profile_mode", plan.action.drm_card),
                    baseline_device.pp_power_profile_mode.clone(),
                    actual_device.pp_power_profile_mode.clone(),
                )
            {
                return verification;
            }

            RollbackVerification::verified(format!(
                "gpu_power rollback restored gpu={}",
                plan.action.drm_card
            ))
        }
        CandidateAction::VmKnob { plan } => {
            if plan.action.changes.is_empty() {
                return RollbackVerification::unavailable("vm_knob rollback has no changes");
            }

            for change in &plan.action.changes {
                let keys = vm_knob_keys_for_change(&plan.action.root, &change.path);
                if let Some(verification) = compare_required_text(
                    format!("vm_knob {}", change.path.display()),
                    vm_knob_active_value(&baseline.vm.knobs, &keys).cloned(),
                    vm_knob_active_value(&post_rollback.vm.knobs, &keys).cloned(),
                ) {
                    return verification;
                }
            }

            RollbackVerification::verified(format!(
                "vm_knob rollback restored {} knob(s)",
                plan.action.changes.len()
            ))
        }
        CandidateAction::Fake { .. } => {
            RollbackVerification::unavailable("fake candidate has no active config verifier")
        }
    }
}

impl RollbackVerification {
    fn verified(summary: impl Into<String>) -> Self {
        let summary = summary.into();
        Self {
            verified: true,
            expected: summary.clone(),
            actual: summary,
            reason_code: "rollback_verified".to_owned(),
        }
    }

    fn mismatch(
        expected: impl Into<String>,
        actual: impl Into<String>,
        reason_code: impl Into<String>,
    ) -> Self {
        Self {
            verified: false,
            expected: expected.into(),
            actual: actual.into(),
            reason_code: reason_code.into(),
        }
    }

    fn unavailable(summary: impl Into<String>) -> Self {
        let summary = summary.into();
        Self {
            verified: false,
            expected: summary,
            actual: "rollback verification unavailable".to_owned(),
            reason_code: "rollback_verification_unavailable".to_owned(),
        }
    }
}

fn verify_cpu_affinity_rollback(
    plan: &CpuAffinityProfilePlan,
    baseline: &ActiveConfigSnapshot,
    post_rollback: &ActiveConfigSnapshot,
    active_tasks: &[ActiveTaskSnapshot],
) -> RollbackVerification {
    if active_tasks.is_empty() {
        return RollbackVerification::unavailable(format!(
            "cpu_affinity_profile profile={} tree_pid={}: active task snapshots missing",
            plan.profile_name, plan.tree_pid
        ));
    }

    let planned_tasks = evaluate_profile_for_tasks(ProfileEvaluationInput {
        profile: &plan.profile,
        active_tasks,
        topology: None,
    });

    if planned_tasks.is_empty() {
        return RollbackVerification::unavailable(format!(
            "cpu_affinity_profile profile={} tree_pid={}: no active tasks matched profile rules",
            plan.profile_name, plan.tree_pid
        ));
    }

    for task in &planned_tasks {
        let Some(expected) = baseline.affinity.per_tid.get(&task.tid) else {
            return RollbackVerification::unavailable(format!(
                "tid={} baseline CPU affinity missing",
                task.tid
            ));
        };
        let Some(actual) = post_rollback.affinity.per_tid.get(&task.tid) else {
            return RollbackVerification::mismatch(
                format!("tid={} cpu_affinity={expected}", task.tid),
                format!("tid={} cpu_affinity=missing", task.tid),
                "rollback_target_missing",
            );
        };
        if !cpu_mask_strings_match(actual, expected) {
            return RollbackVerification::mismatch(
                format!("tid={} cpu_affinity={expected}", task.tid),
                format!("tid={} cpu_affinity={actual}", task.tid),
                "rollback_state_mismatch",
            );
        }
    }

    RollbackVerification::verified(format!(
        "cpu_affinity rollback restored {} target(s)",
        planned_tasks.len()
    ))
}

fn compare_required_text(
    label: impl Into<String>,
    expected: Option<String>,
    actual: Option<String>,
) -> Option<RollbackVerification> {
    let label = label.into();
    match (expected, actual) {
        (Some(expected), Some(actual)) if expected == actual => None,
        (Some(expected), Some(actual)) => Some(RollbackVerification::mismatch(
            format!("{label}={expected}"),
            format!("{label}={actual}"),
            "rollback_state_mismatch",
        )),
        (Some(expected), None) => Some(RollbackVerification::mismatch(
            format!("{label}={expected}"),
            format!("{label}=missing"),
            "rollback_target_missing",
        )),
        (None, _) => Some(RollbackVerification::unavailable(format!(
            "{label} baseline value missing"
        ))),
    }
}

fn format_debug_value<T: fmt::Debug>(value: &T) -> String {
    format!("{value:?}")
}

pub fn cpu_affinity_profile_match(
    plan: &CpuAffinityProfilePlan,
    snapshot: &ActiveConfigSnapshot,
    active_tasks: &[ActiveTaskSnapshot],
) -> ActiveConfigMatch {
    if active_tasks.is_empty() {
        return ActiveConfigMatch::Unknown {
            summary: format!(
                "cpu_affinity_profile profile={} tree_pid={}: active task snapshots missing",
                plan.profile_name, plan.tree_pid
            ),
        };
    }

    let planned_tasks = evaluate_profile_for_tasks(ProfileEvaluationInput {
        profile: &plan.profile,
        active_tasks,
        topology: None,
    });

    if planned_tasks.is_empty() {
        return ActiveConfigMatch::Unknown {
            summary: format!(
                "cpu_affinity_profile profile={} tree_pid={}: no active tasks matched profile rules",
                plan.profile_name, plan.tree_pid
            ),
        };
    }

    for task in &planned_tasks {
        let Some(current) = snapshot.affinity.per_tid.get(&task.tid) else {
            return ActiveConfigMatch::Unknown {
                summary: format!("tid={} active CPU affinity missing", task.tid),
            };
        };

        if !cpu_mask_strings_match(current, &task.requested_mask) {
            return ActiveConfigMatch::Differs {
                expected: format!("tid={} cpu_affinity={}", task.tid, task.requested_mask),
                actual: format!("tid={} cpu_affinity={current}", task.tid),
            };
        }
    }

    ActiveConfigMatch::Matches {
        summary: format!(
            "cpu_affinity_profile profile={} matched_tasks={}",
            plan.profile_name,
            planned_tasks.len()
        ),
    }
}

fn cpu_mask_strings_match(current: &str, requested: &str) -> bool {
    match (CpuMask::parse(current), CpuMask::parse(requested)) {
        (Ok(current), Ok(requested)) => current == requested,
        _ => current.trim() == requested.trim(),
    }
}

fn sorted_active_tids(active_tasks: &[ActiveTaskSnapshot]) -> Vec<u32> {
    active_tasks
        .iter()
        .map(|task| task.tid)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn collect_affinity(proc_root: &Path, tids: &[u32]) -> ActiveAffinitySnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_cpus_allowed_list(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveAffinitySnapshot { per_tid }
}

fn collect_nice(proc_root: &Path, tids: &[u32]) -> ActiveNiceSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_nice(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveNiceSnapshot { per_tid }
}

fn collect_ioprio(proc_root: &Path, tids: &[u32]) -> ActiveIoPrioSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_ioprio(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveIoPrioSnapshot { per_tid }
}

fn collect_uclamp(proc_root: &Path, tids: &[u32]) -> ActiveUclampSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_uclamp(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveUclampSnapshot { per_tid }
}

fn collect_cgroups(proc_root: &Path, tids: &[u32]) -> ActiveCgroupSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_cgroup(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveCgroupSnapshot { per_tid }
}

fn collect_irq(proc_root: &Path) -> ActiveIrqSnapshot {
    let mut per_irq = BTreeMap::new();
    let irq_root = proc_root.join("irq");
    let Ok(entries) = fs::read_dir(irq_root) else {
        return ActiveIrqSnapshot { per_irq };
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Ok(irq) = file_name.parse::<u32>() else {
            continue;
        };
        if let Some(value) = read_trimmed(entry.path().join("smp_affinity")) {
            per_irq.insert(irq, value);
        }
    }

    ActiveIrqSnapshot { per_irq }
}

fn collect_cpu_power(inventory: &SystemInventory) -> ActiveCpuPowerSnapshot {
    ActiveCpuPowerSnapshot {
        policies: inventory
            .cpu_policies
            .iter()
            .map(|policy| CpuPolicyRuntimeState {
                policy: policy.policy.clone(),
                scaling_governor: policy.scaling_governor.clone(),
                energy_performance_preference: policy.energy_performance_preference.clone(),
                related_cpus: policy.related_cpus.clone(),
            })
            .collect(),
    }
}

fn collect_gpu_power(sys_root: &Path, inventory: &SystemInventory) -> ActiveGpuPowerSnapshot {
    ActiveGpuPowerSnapshot {
        devices: inventory
            .drm_devices
            .iter()
            .map(|device| {
                let device_dir = sys_root.join("class/drm").join(&device.name).join("device");
                GpuPowerRuntimeState {
                    device: device.name.clone(),
                    power_dpm_force_performance_level: read_trimmed(
                        device_dir.join("power_dpm_force_performance_level"),
                    ),
                    pp_power_profile_mode: read_trimmed(device_dir.join("pp_power_profile_mode")),
                }
            })
            .collect(),
    }
}

fn collect_vm(inventory: &SystemInventory) -> ActiveVmSnapshot {
    ActiveVmSnapshot {
        knobs: inventory.vm_knobs.clone(),
    }
}

fn read_cpus_allowed_list(proc_root: &Path, tid: u32) -> Option<String> {
    read_status_value(
        proc_root.join(tid.to_string()).join("status"),
        "Cpus_allowed_list",
    )
}

fn read_status_value(path: PathBuf, key: &str) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    contents.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        (left.trim() == key).then(|| right.trim().to_owned())
    })
}

fn read_nice(proc_root: &Path, tid: u32) -> Option<i32> {
    let contents = fs::read_to_string(proc_root.join(tid.to_string()).join("stat")).ok()?;
    let fields = stat_fields_after_comm(&contents)?;
    fields.get(16)?.parse::<i32>().ok()
}

fn read_ioprio(proc_root: &Path, tid: u32) -> Option<String> {
    let fake_file = proc_root.join(tid.to_string()).join("ioprio");
    if let Some(value) = read_trimmed(fake_file) {
        return Some(value);
    }

    if proc_root != Path::new("/proc") {
        return None;
    }

    read_live_ioprio(tid)
}

#[cfg(target_os = "linux")]
fn read_live_ioprio(tid: u32) -> Option<String> {
    const IOPRIO_WHO_PROCESS: libc::c_int = 1;
    let raw = unsafe { libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, tid) };
    if raw < 0 {
        return None;
    }

    crate::actions::ioprio::IoPrioValue::decode(raw as i32)
        .ok()
        .map(|value| value.label())
}

#[cfg(not(target_os = "linux"))]
fn read_live_ioprio(_tid: u32) -> Option<String> {
    None
}

fn read_uclamp(proc_root: &Path, tid: u32) -> Option<UclampValues> {
    let contents = fs::read_to_string(proc_root.join(tid.to_string()).join("sched")).ok()?;
    let mut sched_util_min = None;
    let mut sched_util_max = None;

    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let parsed = value.trim().parse::<u32>().ok();
        match key.trim() {
            "uclamp.min" => sched_util_min = parsed,
            "uclamp.max" => sched_util_max = parsed,
            _ => {}
        }
    }

    (sched_util_min.is_some() || sched_util_max.is_some()).then_some(UclampValues {
        sched_util_min,
        sched_util_max,
    })
}

fn read_cgroup(proc_root: &Path, tid: u32) -> Option<String> {
    let contents = fs::read_to_string(proc_root.join(tid.to_string()).join("cgroup")).ok()?;
    contents.lines().find_map(|line| {
        let (_, path) = line.rsplit_once(':')?;
        Some(path.trim().to_owned())
    })
}

fn stat_fields_after_comm(contents: &str) -> Option<Vec<&str>> {
    let end = contents.rfind(") ")?;
    Some(contents[end + 2..].split_whitespace().collect())
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn uclamp_matches_request(current: UclampValues, requested: UclampValues) -> bool {
    requested
        .sched_util_min
        .is_none_or(|value| current.sched_util_min == Some(value))
        && requested
            .sched_util_max
            .is_none_or(|value| current.sched_util_max == Some(value))
}

fn normalize_cgroup_path(path: &Path) -> String {
    normalize_cgroup_str(&path.to_string_lossy())
}

fn normalize_cgroup_str(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/".to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

fn cpu_policy_for_cpu(
    policies: &[CpuPolicyRuntimeState],
    cpu: u32,
) -> Option<&CpuPolicyRuntimeState> {
    policies.iter().find(|policy| {
        policy
            .related_cpus
            .as_deref()
            .is_some_and(|related| cpu_list_contains(related, cpu))
            || policy.policy == format!("policy{cpu}")
    })
}

fn cpu_list_contains(list: &str, cpu: u32) -> bool {
    list.split(',').any(|part| {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let Ok(start) = start.trim().parse::<u32>() else {
                return false;
            };
            let Ok(end) = end.trim().parse::<u32>() else {
                return false;
            };
            (start..=end).contains(&cpu)
        } else {
            part.parse::<u32>().is_ok_and(|value| value == cpu)
        }
    })
}

fn vm_knob_keys_for_change(root: &Path, path: &Path) -> Vec<String> {
    let mut keys = BTreeSet::new();
    keys.insert(path.to_string_lossy().trim_start_matches('/').to_owned());

    if let Ok(relative) = path.strip_prefix(root) {
        keys.insert(
            relative
                .to_string_lossy()
                .trim_start_matches('/')
                .to_owned(),
        );
    }

    if let Ok(relative) = path.strip_prefix("/proc") {
        keys.insert(
            relative
                .to_string_lossy()
                .trim_start_matches('/')
                .to_owned(),
        );
    }

    keys.into_iter().collect()
}

fn vm_knob_active_value<'a>(
    knobs: &'a BTreeMap<String, String>,
    keys: &[String],
) -> Option<&'a String> {
    keys.iter().find_map(|key| knobs.get(key))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;
    use crate::{
        affinity::CpuMask,
        autotune::observation::ActiveTaskSnapshot,
        daemon::capabilities::DaemonCapabilities,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        system_inventory::{SystemInventory, SystemInventoryRoot},
        test_support::TestRoot,
    };

    #[test]
    fn collector_populates_every_active_config_subsnapshot_and_serializes() {
        let root = TestRoot::new("active-config");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        fs::create_dir_all(&proc_root).unwrap();
        fs::create_dir_all(&sys_root).unwrap();

        write_task_fixture(
            &proc_root,
            TaskFixture {
                tid: 42,
                comm: "game",
                cpus_allowed_list: "0-3",
                nice: 5,
                ioprio: "best-effort:4",
                uclamp_min: 128,
                uclamp_max: 1024,
                cgroup: "/game.slice",
            },
        );
        write_irq_fixture(&proc_root, 44, "00000002");
        write_cpu_policy_fixture(&sys_root, "policy0", "performance", "performance", "0-3");
        write_gpu_fixture(&sys_root, "card0", "high", "3D_FULL_SCREEN");
        write_vm_fixture(&proc_root, "10", "20", "5");

        let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: proc_root.clone(),
            sys_root: sys_root.clone(),
        });
        let capabilities = DaemonCapabilities {
            ionice_available: true,
            uclamp_available: true,
            irq_affinity_available: true,
            gpu_sysfs_available: true,
            ..DaemonCapabilities::default()
        };
        let active_tasks = vec![active_task(42)];

        let snapshot = collect_active_config(ActiveConfigCollectorInput {
            proc_root: &proc_root,
            sys_root: &sys_root,
            active_tasks: &active_tasks,
            capabilities: &capabilities,
            inventory: &inventory,
        });

        assert_eq!(snapshot.affinity.per_tid.get(&42).unwrap(), "0-3");
        assert_eq!(snapshot.nice.per_tid.get(&42), Some(&5));
        assert_eq!(snapshot.ionice.per_tid.get(&42).unwrap(), "best-effort:4");
        assert_eq!(
            snapshot.uclamp.per_tid.get(&42).unwrap(),
            &UclampValues {
                sched_util_min: Some(128),
                sched_util_max: Some(1024)
            }
        );
        assert_eq!(snapshot.cgroup.per_tid.get(&42).unwrap(), "/game.slice");
        assert_eq!(snapshot.irq.per_irq.get(&44).unwrap(), "00000002");
        assert_eq!(snapshot.cpu_power.policies.len(), 1);
        assert_eq!(
            snapshot.cpu_power.policies[0].scaling_governor.as_deref(),
            Some("performance")
        );
        assert_eq!(
            snapshot.cpu_power.policies[0]
                .energy_performance_preference
                .as_deref(),
            Some("performance")
        );
        assert_eq!(snapshot.gpu_power.devices.len(), 1);
        assert_eq!(
            snapshot.gpu_power.devices[0]
                .power_dpm_force_performance_level
                .as_deref(),
            Some("high")
        );
        assert_eq!(
            snapshot.gpu_power.devices[0]
                .pp_power_profile_mode
                .as_deref(),
            Some("3D_FULL_SCREEN")
        );
        assert_eq!(snapshot.vm.knobs.get("sys/vm/swappiness").unwrap(), "10");

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"affinity\""));
        assert!(json.contains("\"ionice\""));
        assert!(json.contains("\"cpu_power\""));
        assert!(json.contains("\"gpu_power\""));
        assert!(json.contains("\"sched_util_min\":128"));
    }

    #[test]
    fn candidate_active_config_match_reports_difference_for_task_actions() {
        let root = TestRoot::new("active-config-diff");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        fs::create_dir_all(&proc_root).unwrap();
        fs::create_dir_all(&sys_root).unwrap();

        write_task_fixture(
            &proc_root,
            TaskFixture {
                tid: 77,
                comm: "worker",
                cpus_allowed_list: "2",
                nice: 0,
                ioprio: "idle",
                uclamp_min: 64,
                uclamp_max: 512,
                cgroup: "/compile.slice",
            },
        );

        let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: proc_root.clone(),
            sys_root: sys_root.clone(),
        });
        let capabilities = DaemonCapabilities {
            ionice_available: true,
            uclamp_available: true,
            ..DaemonCapabilities::default()
        };
        let active_tasks = vec![active_task(77)];
        let snapshot = collect_active_config(ActiveConfigCollectorInput {
            proc_root: &proc_root,
            sys_root: &sys_root,
            active_tasks: &active_tasks,
            capabilities: &capabilities,
            inventory: &inventory,
        });

        let candidate = CandidateAction::Nice {
            plan: crate::autotune::candidate::NiceActionPlan {
                name: "nice-diff".to_owned(),
                action: crate::actions::nice::NiceAction {
                    targets: vec![crate::actions::TaskIdentity {
                        tid: 77,
                        process_pid: Some(77),
                        comm: Some("worker".to_owned()),
                        starttime_ticks: Some(1),
                    }],
                    nice: 10,
                    policy: crate::actions::nice::NicePolicy::default(),
                },
                target_root_pid: Some(77),
                evidence: Vec::new(),
                objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
            },
        };

        let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
            snapshot: &snapshot,
            active_tasks: &active_tasks,
        });

        assert!(active_match.is_differs());
        assert!(matches!(active_match, ActiveConfigMatch::Differs { .. }));
        assert!(!candidate_is_noop(&candidate, &snapshot));
    }

    #[test]
    fn rollback_verification_succeeds_when_active_config_returns_to_baseline() {
        let candidate = nice_candidate_for_rollback();
        let baseline = active_nice_snapshot(42, 0);
        let post_rollback = active_nice_snapshot(42, 0);

        let verification =
            verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

        assert!(verification.verified);
        assert_eq!(verification.reason_code, "rollback_verified");
    }

    #[test]
    fn rollback_verification_faults_when_active_config_still_differs() {
        let candidate = nice_candidate_for_rollback();
        let baseline = active_nice_snapshot(42, 0);
        let post_rollback = active_nice_snapshot(42, 5);

        let verification =
            verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

        assert!(!verification.verified);
        assert_eq!(verification.reason_code, "rollback_state_mismatch");
        assert!(verification.expected.contains("tid=42 nice=0"));
        assert!(verification.actual.contains("tid=42 nice=5"));
    }

    #[test]
    fn rollback_verification_faults_when_target_missing_after_rollback() {
        let candidate = nice_candidate_for_rollback();
        let baseline = active_nice_snapshot(42, 0);
        let post_rollback = ActiveConfigSnapshot::default();

        let verification =
            verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

        assert!(!verification.verified);
        assert_eq!(verification.reason_code, "rollback_target_missing");
        assert!(verification.actual.contains("missing"));
    }

    #[test]
    fn rollback_verification_faults_when_verifier_is_unavailable() {
        let candidate = CandidateAction::fake(
            crate::actions::ActionId::new("fake-rollback".to_owned()),
            crate::actions::SafetyClass::ReversibleLowRisk,
        );
        let baseline = ActiveConfigSnapshot::default();
        let post_rollback = ActiveConfigSnapshot::default();

        let verification =
            verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

        assert!(!verification.verified);
        assert_eq!(
            verification.reason_code,
            "rollback_verification_unavailable"
        );
    }

    #[test]
    fn candidate_noop_helper_matches_typed_snapshot_for_task_actions() {
        let root = TestRoot::new("active-config-noop");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        fs::create_dir_all(&proc_root).unwrap();
        fs::create_dir_all(&sys_root).unwrap();

        write_task_fixture(
            &proc_root,
            TaskFixture {
                tid: 77,
                comm: "worker",
                cpus_allowed_list: "2",
                nice: 10,
                ioprio: "idle",
                uclamp_min: 64,
                uclamp_max: 512,
                cgroup: "/compile.slice",
            },
        );

        let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: proc_root.clone(),
            sys_root: sys_root.clone(),
        });
        let capabilities = DaemonCapabilities {
            ionice_available: true,
            uclamp_available: true,
            ..DaemonCapabilities::default()
        };
        let active_tasks = vec![active_task(77)];
        let snapshot = collect_active_config(ActiveConfigCollectorInput {
            proc_root: &proc_root,
            sys_root: &sys_root,
            active_tasks: &active_tasks,
            capabilities: &capabilities,
            inventory: &inventory,
        });

        let candidate = CandidateAction::Nice {
            plan: crate::autotune::candidate::NiceActionPlan {
                name: "nice-noop".to_owned(),
                action: crate::actions::nice::NiceAction {
                    targets: vec![crate::actions::TaskIdentity {
                        tid: 77,
                        process_pid: Some(77),
                        comm: Some("worker".to_owned()),
                        starttime_ticks: Some(1),
                    }],
                    nice: 10,
                    policy: crate::actions::nice::NicePolicy::default(),
                },
                target_root_pid: Some(77),
                evidence: Vec::new(),
                objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
            },
        };

        assert!(candidate_is_noop(&candidate, &snapshot));
    }

    #[test]
    fn cpu_affinity_profile_match_reports_exact_match() {
        let profile = profile_for_class(TaskClass::Game, "0-1");
        let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
        let snapshot = affinity_snapshot([(11, "0-1")]);
        let active_tasks = vec![active_task_with_class(11, TaskClass::Game)];

        let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
            snapshot: &snapshot,
            active_tasks: &active_tasks,
        });

        assert!(active_match.is_match());
        assert!(candidate_is_noop_with_tasks(
            &candidate,
            &snapshot,
            &active_tasks
        ));
    }

    #[test]
    fn cpu_affinity_profile_match_reports_one_differing_tid() {
        let profile = profile_for_class(TaskClass::Game, "0-1");
        let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
        let snapshot = affinity_snapshot([(11, "2")]);
        let active_tasks = vec![active_task_with_class(11, TaskClass::Game)];

        let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
            snapshot: &snapshot,
            active_tasks: &active_tasks,
        });

        assert!(matches!(active_match, ActiveConfigMatch::Differs { .. }));
    }

    #[test]
    fn cpu_affinity_profile_match_is_unknown_when_affinity_data_missing() {
        let profile = profile_for_class(TaskClass::Game, "0-1");
        let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
        let active_tasks = vec![active_task_with_class(11, TaskClass::Game)];

        let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
            snapshot: &ActiveConfigSnapshot::default(),
            active_tasks: &active_tasks,
        });

        assert!(matches!(active_match, ActiveConfigMatch::Unknown { .. }));
    }

    #[test]
    fn cpu_affinity_profile_match_is_unknown_when_no_tasks_match_rules() {
        let profile = profile_for_class(TaskClass::Game, "0-1");
        let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
        let snapshot = affinity_snapshot([(11, "0-1")]);
        let active_tasks = vec![active_task_with_class(11, TaskClass::Service)];

        let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
            snapshot: &snapshot,
            active_tasks: &active_tasks,
        });

        assert!(matches!(active_match, ActiveConfigMatch::Unknown { .. }));
    }

    #[test]
    fn cpu_affinity_profile_match_does_not_target_protected_tasks_unless_rule_matches() {
        let profile = profile_for_class(TaskClass::Game, "0-1");
        let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
        let snapshot = affinity_snapshot([(11, "0-1")]);
        let active_tasks = vec![active_task_with_class(11, TaskClass::Compositor)];

        let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
            snapshot: &snapshot,
            active_tasks: &active_tasks,
        });

        assert!(matches!(active_match, ActiveConfigMatch::Unknown { .. }));
    }

    #[test]
    fn cpu_affinity_profile_match_supports_broad_fallback_rules() {
        let profile = Profile {
            name: "fallback".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("3").unwrap()),
                nice: None,
                ionice: None,
                match_class: Vec::new(),
                match_comm: Vec::new(),
            }],
        };
        let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
        let snapshot = affinity_snapshot([(11, "3")]);
        let active_tasks = vec![active_task_with_class(11, TaskClass::Unknown)];

        let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
            snapshot: &snapshot,
            active_tasks: &active_tasks,
        });

        assert!(active_match.is_match());
    }

    fn active_task(tid: u32) -> ActiveTaskSnapshot {
        active_task_with_class(tid, TaskClass::Unknown)
    }

    fn active_task_with_class(tid: u32, class: TaskClass) -> ActiveTaskSnapshot {
        ActiveTaskSnapshot {
            tid,
            process_pid: tid,
            comm: format!("task-{tid}"),
            class,
            process_starttime_ticks: Some(1),
            task_starttime_ticks: Some(1),
            cgroup_path: Some("/game.slice".to_owned()),
        }
    }

    fn profile_for_class(class: TaskClass, mask: &str) -> Profile {
        Profile {
            name: format!("profile-{class:?}"),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse(mask).unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![class],
                match_comm: Vec::new(),
            }],
        }
    }

    fn affinity_snapshot<const N: usize>(entries: [(u32, &str); N]) -> ActiveConfigSnapshot {
        ActiveConfigSnapshot {
            affinity: ActiveAffinitySnapshot {
                per_tid: entries
                    .into_iter()
                    .map(|(tid, mask)| (tid, mask.to_owned()))
                    .collect(),
            },
            ..ActiveConfigSnapshot::default()
        }
    }

    struct TaskFixture<'a> {
        tid: u32,
        comm: &'a str,
        cpus_allowed_list: &'a str,
        nice: i32,
        ioprio: &'a str,
        uclamp_min: u32,
        uclamp_max: u32,
        cgroup: &'a str,
    }

    fn write_task_fixture(proc_root: &Path, fixture: TaskFixture<'_>) {
        let task_root = proc_root.join(fixture.tid.to_string());
        fs::create_dir_all(&task_root).unwrap();

        fs::write(
            task_root.join("status"),
            format!(
                "Name:\t{}\nCpus_allowed_list:\t{}\n",
                fixture.comm, fixture.cpus_allowed_list
            ),
        )
        .unwrap();
        fs::write(
            task_root.join("stat"),
            fake_stat_line(fixture.tid, fixture.comm, fixture.nice),
        )
        .unwrap();
        fs::write(task_root.join("ioprio"), format!("{}\n", fixture.ioprio)).unwrap();
        fs::write(
            task_root.join("sched"),
            format!(
                "uclamp.min                                   : {}\nuclamp.max                                   : {}\n",
                fixture.uclamp_min, fixture.uclamp_max
            ),
        )
        .unwrap();
        fs::write(task_root.join("cgroup"), format!("0::{}\n", fixture.cgroup)).unwrap();
    }

    fn fake_stat_line(tid: u32, comm: &str, nice: i32) -> String {
        let mut fields = vec!["0".to_owned(); 40];
        fields[0] = "S".to_owned();
        fields[16] = nice.to_string();
        format!("{tid} ({comm}) {}\n", fields.join(" "))
    }

    fn active_nice_snapshot(tid: u32, nice: i32) -> ActiveConfigSnapshot {
        ActiveConfigSnapshot {
            nice: ActiveNiceSnapshot {
                per_tid: std::collections::BTreeMap::from([(tid, nice)]),
            },
            ..ActiveConfigSnapshot::default()
        }
    }

    fn nice_candidate_for_rollback() -> CandidateAction {
        CandidateAction::Nice {
            plan: crate::autotune::candidate::NiceActionPlan {
                name: "nice-rollback-verification".to_owned(),
                action: crate::actions::nice::NiceAction {
                    targets: vec![crate::actions::TaskIdentity {
                        tid: 42,
                        process_pid: Some(42),
                        comm: Some("game".to_owned()),
                        starttime_ticks: Some(1),
                    }],
                    nice: 5,
                    policy: crate::actions::nice::NicePolicy::default(),
                },
                target_root_pid: Some(42),
                evidence: Vec::new(),
                objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
            },
        }
    }

    fn write_irq_fixture(proc_root: &Path, irq: u32, smp_affinity: &str) {
        let irq_root = proc_root.join("irq").join(irq.to_string());
        fs::create_dir_all(&irq_root).unwrap();
        fs::write(irq_root.join("smp_affinity"), format!("{smp_affinity}\n")).unwrap();
    }

    fn write_cpu_policy_fixture(
        sys_root: &Path,
        policy: &str,
        governor: &str,
        epp: &str,
        related_cpus: &str,
    ) {
        let policy_root = sys_root.join("devices/system/cpu/cpufreq").join(policy);
        fs::create_dir_all(&policy_root).unwrap();
        fs::write(
            policy_root.join("scaling_governor"),
            format!("{governor}\n"),
        )
        .unwrap();
        fs::write(
            policy_root.join("energy_performance_preference"),
            format!("{epp}\n"),
        )
        .unwrap();
        fs::write(
            policy_root.join("related_cpus"),
            format!("{related_cpus}\n"),
        )
        .unwrap();
    }

    fn write_gpu_fixture(sys_root: &Path, card: &str, dpm: &str, profile: &str) {
        let device_root = sys_root.join("class/drm").join(card).join("device");
        fs::create_dir_all(device_root.join("drm/renderD128")).unwrap();
        fs::write(
            device_root.join("power_dpm_force_performance_level"),
            format!("{dpm}\n"),
        )
        .unwrap();
        fs::write(
            device_root.join("pp_power_profile_mode"),
            format!("{profile}\n"),
        )
        .unwrap();
    }

    fn write_vm_fixture(proc_root: &Path, swappiness: &str, dirty_ratio: &str, dirty_bg: &str) {
        let vm_root = proc_root.join("sys/vm");
        fs::create_dir_all(&vm_root).unwrap();
        fs::write(vm_root.join("swappiness"), format!("{swappiness}\n")).unwrap();
        fs::write(vm_root.join("dirty_ratio"), format!("{dirty_ratio}\n")).unwrap();
        fs::write(
            vm_root.join("dirty_background_ratio"),
            format!("{dirty_bg}\n"),
        )
        .unwrap();
    }
}
