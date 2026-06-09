use super::*;

#[test]
fn irq_affinity_gpu_device_is_medium_risk_not_manual_only() {
    let evidence = IrqAffinityEvidence {
        strong_irq_evidence: true,
        stable_irq_identity: true,
        known_device_mapping: true,
        observed_irq: Some(42),
        observed_device_hint: Some("amdgpu".to_owned()),
        reason: "integration test GPU IRQ evidence".to_owned(),
    };
    let candidate = CandidateAction::IrqAffinity {
        plan: IrqAffinityActionPlan {
            name: "irq-gpu-medium".to_owned(),
            action: IrqAffinityAction::new(
                42,
                "amdgpu".to_owned(),
                "1".to_owned(),
                IrqAffinityRisk::ReversibleMediumRisk,
                evidence,
            ),
            evidence: vec![CandidateEvidence::new("irq_device", "amdgpu", 1.0)],
            objective: ObjectiveKind::IrqOverlapReduction,
        },
    };

    assert!(!candidate.is_high_risk_system_adjacent());
    assert!(candidate.manual_only_reason().is_none());
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert!(
        CandidatePlanFile::from_candidate(&candidate, None)
            .executable
            .is_some()
    );
}

#[test]
fn gpu_power_profile_switch_only_is_medium_risk() {
    let candidate = CandidateAction::GpuPower {
        plan: GpuPowerActionPlan {
            name: "gpu-profile-medium".to_owned(),
            action: GpuPowerAction {
                sysfs_root: PathBuf::from("/sys"),
                drm_card: "card0".to_owned(),
                power_dpm_force_performance_level: None,
                pp_power_profile_mode: Some("3D_FULL_SCREEN".to_owned()),
            },
            evidence: vec![CandidateEvidence::new("gpu_profile", "card0", 1.0)],
            objective: ObjectiveKind::GameFramePacing,
        },
    };

    assert!(!candidate.is_high_risk_system_adjacent());
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert!(
        CandidatePlanFile::from_candidate(&candidate, None)
            .executable
            .is_some()
    );
}

#[test]
fn vm_swappiness_candidate_is_medium_risk_and_executable() {
    let candidate = CandidateAction::VmKnob {
        plan: VmKnobActionPlan {
            name: "vm-swappiness-medium".to_owned(),
            action: VmKnobAction {
                root: PathBuf::from("/"),
                changes: vec![VmKnobChange {
                    path: PathBuf::from("proc/sys/vm/swappiness"),
                    value: "10".to_owned(),
                }],
            },
            evidence: vec![CandidateEvidence::new(
                "swap_pressure",
                "mem_stall_spike",
                1.0,
            )],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    };

    assert!(!candidate.is_high_risk_system_adjacent());
    assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    assert!(
        CandidatePlanFile::from_candidate(&candidate, None)
            .executable
            .is_some()
    );
}
