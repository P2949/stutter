use std::fmt;

use super::helpers::*;
use crate::{
    autotune::{
        observation::{ActiveConfigSnapshot, ActiveTaskSnapshot},
        planning::{candidate::CandidateAction, executable_plan::CpuAffinityProfilePlan},
    },
    profiles::{ProfileEvaluationInput, evaluate_profile_for_tasks},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackVerification {
    pub verified: bool,
    pub expected: String,
    pub actual: String,
    pub reason_code: String,
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
                    baseline
                        .nice
                        .per_tid
                        .get(&target.tid.as_u32())
                        .map(i32::to_string),
                    post_rollback
                        .nice
                        .per_tid
                        .get(&target.tid.as_u32())
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
                    baseline.ionice.per_tid.get(&target.tid.as_u32()).cloned(),
                    post_rollback
                        .ionice
                        .per_tid
                        .get(&target.tid.as_u32())
                        .cloned(),
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
                        .get(&target.tid.as_u32())
                        .map(format_debug_value),
                    post_rollback
                        .uclamp
                        .per_tid
                        .get(&target.tid.as_u32())
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
                        .get(&target.identity.tid.as_u32())
                        .map(|value| normalize_cgroup_str(value)),
                    post_rollback
                        .cgroup
                        .per_tid
                        .get(&target.identity.tid.as_u32())
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
        let tid = task.tid.as_u32();

        let Some(expected) = baseline.affinity.per_tid.get(&tid) else {
            return RollbackVerification::unavailable(format!(
                "tid={} baseline CPU affinity missing",
                task.tid
            ));
        };
        let Some(actual) = post_rollback.affinity.per_tid.get(&tid) else {
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
