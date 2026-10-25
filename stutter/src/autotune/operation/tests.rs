use std::path::PathBuf;

use super::*;
use crate::{
    actions::{
        ActionId, SafetyClass,
        cpu_power::CpuPowerAction,
        gpu_power::GpuPowerAction,
        irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
        vm_knobs::{VmKnobAction, VmKnobChange},
    },
    autotune::{
        objective::ObjectiveKind,
        planning::executable_plan::{
            CpuPowerActionPlan, GpuPowerActionPlan, IrqAffinityActionPlan, VmKnobActionPlan,
        },
    },
};

fn cpu(governor: Option<&str>, epp: Option<&str>) -> CandidateAction {
    CandidateAction::CpuPower {
        plan: CpuPowerActionPlan {
            name: "cpu".to_owned(),
            action: CpuPowerAction {
                sysfs_root: PathBuf::from("/sys"),
                cpus: vec![0],
                scaling_governor: governor.map(str::to_owned),
                energy_performance_preference: epp.map(str::to_owned),
            },
            evidence: Vec::new(),
            objective: ObjectiveKind::GameRunnableLatency,
        },
    }
}

fn gpu(level: Option<&str>, profile: Option<&str>) -> CandidateAction {
    CandidateAction::GpuPower {
        plan: GpuPowerActionPlan {
            name: "gpu".to_owned(),
            action: GpuPowerAction {
                sysfs_root: PathBuf::from("/sys"),
                drm_card: "card0".to_owned(),
                power_dpm_force_performance_level: level.map(str::to_owned),
                pp_power_profile_mode: profile.map(str::to_owned),
            },
            evidence: Vec::new(),
            objective: ObjectiveKind::GameFramePacing,
        },
    }
}

fn vm(paths: &[&str]) -> CandidateAction {
    CandidateAction::VmKnob {
        plan: VmKnobActionPlan {
            name: "vm".to_owned(),
            action: VmKnobAction {
                root: PathBuf::from("/proc/sys/vm"),
                changes: paths
                    .iter()
                    .map(|name| VmKnobChange {
                        path: PathBuf::from("/proc/sys/vm").join(name),
                        value: "10".to_owned(),
                    })
                    .collect(),
            },
            evidence: Vec::new(),
            objective: ObjectiveKind::IoLatency,
        },
    }
}

fn irq(risk: IrqAffinityRisk, known: bool) -> CandidateAction {
    CandidateAction::IrqAffinity {
        plan: IrqAffinityActionPlan {
            name: "irq".to_owned(),
            action: IrqAffinityAction {
                irq: 30,
                device_hint: "eth0".to_owned(),
                smp_affinity: "2".to_owned(),
                risk,
                evidence: IrqAffinityEvidence {
                    strong_irq_evidence: true,
                    stable_irq_identity: true,
                    known_device_mapping: known,
                    observed_irq: Some(30),
                    observed_device_hint: Some("eth0".to_owned()),
                    reason: String::new(),
                },
                irq_root: PathBuf::from("/proc/irq"),
            },
            evidence: Vec::new(),
            objective: ObjectiveKind::IrqOverlapReduction,
        },
    }
}

#[test]
fn cpu_power_is_classified_by_the_attribute_it_writes() {
    assert_eq!(
        cpu(None, Some("performance")).operation_kind(),
        MutationOperationKind::CpuPowerEpp
    );
    assert_eq!(
        cpu(Some("performance"), None).operation_kind(),
        MutationOperationKind::CpuPowerGovernor
    );
    // A payload that also touches the governor is the riskier operation.
    assert_eq!(
        cpu(Some("performance"), Some("performance")).operation_kind(),
        MutationOperationKind::CpuPowerGovernor
    );
    assert_eq!(
        cpu(None, None).operation_kind(),
        MutationOperationKind::Unclassified
    );
}

#[test]
fn gpu_power_separates_the_profile_from_forced_performance() {
    assert_eq!(
        gpu(None, Some("3")).operation_kind(),
        MutationOperationKind::GpuPowerProfile
    );
    assert_eq!(
        gpu(Some("high"), None).operation_kind(),
        MutationOperationKind::GpuForcePerformance
    );
    assert_eq!(
        gpu(Some("high"), Some("3")).operation_kind(),
        MutationOperationKind::GpuForcePerformance
    );
}

#[test]
fn vm_knobs_are_classified_by_the_riskiest_knob() {
    assert_eq!(
        vm(&["swappiness"]).operation_kind(),
        MutationOperationKind::VmSwappiness
    );
    assert_eq!(
        vm(&["swappiness", "dirty_background_ratio"]).operation_kind(),
        MutationOperationKind::VmDirtyBackgroundRatio
    );
    assert_eq!(
        vm(&["dirty_ratio", "swappiness"]).operation_kind(),
        MutationOperationKind::VmDirtyRatio
    );
    assert_eq!(
        vm(&["overcommit_memory"]).operation_kind(),
        MutationOperationKind::Unclassified
    );
    assert_eq!(
        vm(&[]).operation_kind(),
        MutationOperationKind::Unclassified
    );
}

#[test]
fn only_a_known_medium_risk_irq_is_the_known_device_operation() {
    assert_eq!(
        irq(IrqAffinityRisk::ReversibleMediumRisk, true).operation_kind(),
        MutationOperationKind::IrqKnownDevice
    );
    assert_eq!(
        irq(IrqAffinityRisk::ReversibleMediumRisk, false).operation_kind(),
        MutationOperationKind::IrqStorageOrUnknown
    );
    assert_eq!(
        irq(IrqAffinityRisk::HighRisk, true).operation_kind(),
        MutationOperationKind::IrqStorageOrUnknown
    );
}

#[test]
fn the_fake_candidate_and_every_manual_operation_stay_unauthorised_by_default() {
    let fake = CandidateAction::fake(ActionId::new("fake".to_owned()), SafetyClass::HighRisk);
    assert_eq!(fake.operation_kind(), MutationOperationKind::Unclassified);
    for operation in MutationOperationKind::ALL {
        if operation == MutationOperationKind::Unclassified {
            assert_eq!(operation.capability_id(), None);
        } else {
            assert!(operation.capability_id().is_some(), "{operation:?}");
        }
    }
    for manual in [
        MutationOperationKind::CpuPowerGovernor,
        MutationOperationKind::GpuForcePerformance,
        MutationOperationKind::VmDirtyRatio,
        MutationOperationKind::VmDirtyBackgroundRatio,
        MutationOperationKind::IrqStorageOrUnknown,
    ] {
        assert!(manual.is_manual_only());
    }
}

#[test]
fn operation_names_are_unique_and_round_trip_through_json() {
    let names: std::collections::BTreeSet<_> = MutationOperationKind::ALL
        .iter()
        .map(|operation| operation.as_str())
        .collect();
    assert_eq!(names.len(), MutationOperationKind::ALL.len());
    for operation in MutationOperationKind::ALL {
        let json = serde_json::to_string(&operation).unwrap();
        assert_eq!(json, format!("\"{}\"", operation.as_str()));
        assert_eq!(
            serde_json::from_str::<MutationOperationKind>(&json).unwrap(),
            operation
        );
    }
}
