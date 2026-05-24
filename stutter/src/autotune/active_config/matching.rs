use super::helpers::*;
use crate::{
    autotune::{
        observation::{ActiveConfigSnapshot, ActiveTaskSnapshot},
        planning::{candidate::CandidateAction, executable_plan::CpuAffinityProfilePlan},
    },
    profiles::{ProfileEvaluationInput, evaluate_profile_for_tasks},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveConfigMatch {
    Matches { summary: String },
    Differs { expected: String, actual: String },
    Unknown { summary: String },
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
                    match snapshot.nice.per_tid.get(&target.tid.as_u32()) {
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
                    match snapshot.ionice.per_tid.get(&target.tid.as_u32()) {
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
                    match snapshot.uclamp.per_tid.get(&target.tid.as_u32()) {
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
                    match snapshot.cgroup.per_tid.get(&target.identity.tid.as_u32()) {
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
        let tid = task.tid.as_u32();

        let Some(current) = snapshot.affinity.per_tid.get(&tid) else {
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
