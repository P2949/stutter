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

#[test]
fn an_affinity_profile_that_also_changes_priority_is_not_the_graduated_operation() {
    use crate::profiles::{Profile, ProfileRule};
    let rule = |nice: Option<i32>| ProfileRule {
        affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
        nice,
        ionice: None,
        match_class: Vec::new(),
        match_comm: Vec::new(),
    };
    let affinity_only = CandidateAction::cpu_affinity_profile(
        Profile {
            name: "a".to_owned(),
            rules: vec![rule(None)],
        },
        1,
    );
    let with_priority = CandidateAction::cpu_affinity_profile(
        Profile {
            name: "b".to_owned(),
            rules: vec![rule(Some(5))],
        },
        1,
    );
    assert_eq!(
        affinity_only.operation_kind(),
        MutationOperationKind::CpuAffinityProfile
    );
    assert_eq!(
        with_priority.operation_kind(),
        MutationOperationKind::Unclassified
    );
}

#[test]
fn only_operations_with_a_prepared_transaction_can_be_registered() {
    use crate::autotune::autonomous_support::AUTONOMOUS_CONTRACTS;
    for contract in AUTONOMOUS_CONTRACTS {
        assert!(
            contract.operation.prepared_transaction_implemented(),
            "{contract:?}"
        );
    }
    for operation in MutationOperationKind::ALL {
        assert_eq!(
            operation.prepared_transaction_implemented(),
            matches!(
                operation,
                MutationOperationKind::CpuAffinityProfile
                    | MutationOperationKind::Uclamp
                    | MutationOperationKind::Nice
                    | MutationOperationKind::CgroupPlacement
                    | MutationOperationKind::CpuPowerEpp
            ),
            "{operation:?}"
        );
    }
}

#[test]
fn a_pm_qos_candidate_is_its_own_manual_only_operation_and_never_an_executable_plan() {
    use crate::autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::CandidateAction,
            executable_plan::{CandidateExecutablePlan, PmQosActionPlan},
        },
    };
    let candidate = CandidateAction::PmQos {
        plan: PmQosActionPlan::manual(50, ObjectiveKind::GameRunnableLatency),
    };
    assert_eq!(
        candidate.operation_kind(),
        MutationOperationKind::PmQosCpuLatency
    );
    assert!(MutationOperationKind::PmQosCpuLatency.is_manual_only());
    assert!(!MutationOperationKind::PmQosCpuLatency.prepared_transaction_implemented());
    assert!(candidate.is_mutation_provider());
    assert!(
        CandidateExecutablePlan::from_candidate(&candidate).is_none(),
        "a HighRisk system-wide request can never become an executable autotune plan"
    );
    assert_eq!(
        MutationOperationKind::PmQosCpuLatency.capability_id(),
        Some("pm_qos_cpu_latency")
    );
}

#[test]
fn a_resctrl_candidate_is_its_own_manual_only_operation_and_never_an_executable_plan() {
    use crate::autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::CandidateAction,
            executable_plan::{CandidateExecutablePlan, ResctrlActionPlan},
        },
    };
    let candidate = CandidateAction::Resctrl {
        plan: ResctrlActionPlan::manual("trial", 0xf0, ObjectiveKind::GameRunnableLatency),
    };
    assert_eq!(
        candidate.operation_kind(),
        MutationOperationKind::ResctrlCacheAllocation
    );
    assert!(MutationOperationKind::ResctrlCacheAllocation.is_manual_only());
    assert!(CandidateExecutablePlan::from_candidate(&candidate).is_none());
    assert_eq!(
        MutationOperationKind::ResctrlCacheAllocation.capability_id(),
        Some("resctrl_cache_allocation")
    );
}

#[test]
fn a_damos_candidate_is_its_own_manual_only_operation_and_never_an_executable_plan() {
    use crate::{
        actions::damos_scheme::{DamosAction, DamosQuota, DamosRequest},
        autotune::{
            objective::ObjectiveKind,
            planning::{
                candidate::CandidateAction,
                executable_plan::{CandidateExecutablePlan, DamosActionPlan},
            },
        },
    };
    let request = DamosRequest {
        task: 1,
        start_ticks: 1,
        action: DamosAction::Stat,
        quota: DamosQuota {
            ms: 1,
            bytes: 1,
            reset_interval_ms: 1000,
        },
        acknowledge_irreversible: false,
        min_region_bytes: 1,
        max_region_bytes: 2,
    };
    let candidate = CandidateAction::Damos {
        plan: DamosActionPlan::manual(request, ObjectiveKind::GameRunnableLatency),
    };
    assert_eq!(
        candidate.operation_kind(),
        MutationOperationKind::DamosScheme
    );
    assert!(MutationOperationKind::DamosScheme.is_manual_only());
    assert!(CandidateExecutablePlan::from_candidate(&candidate).is_none());
}

#[test]
fn each_cgroup_knob_is_its_own_manual_only_operation_with_its_own_capability() {
    use crate::{
        actions::cgroup::control::CgroupControl,
        autotune::{
            objective::ObjectiveKind,
            planning::{
                candidate::CandidateAction,
                executable_plan::{CandidateExecutablePlan, CgroupControlActionPlan},
            },
        },
    };
    let knobs = [
        (
            CgroupControl::CpuWeight(100),
            MutationOperationKind::CgroupCpuWeight,
        ),
        (
            CgroupControl::CpuIdle(true),
            MutationOperationKind::CgroupCpuIdle,
        ),
        (
            CgroupControl::CpuMax {
                quota_us: Some(1000),
                period_us: 100_000,
            },
            MutationOperationKind::CgroupCpuMax,
        ),
        (
            CgroupControl::CpusetCpus(vec![0]),
            MutationOperationKind::CgroupCpusetCpus,
        ),
        (
            CgroupControl::CpusetMems(vec![0]),
            MutationOperationKind::CgroupCpusetMems,
        ),
        (
            CgroupControl::MemoryMin(1),
            MutationOperationKind::CgroupMemoryMin,
        ),
        (
            CgroupControl::MemoryLow(1),
            MutationOperationKind::CgroupMemoryLow,
        ),
        (
            CgroupControl::MemoryHigh(None),
            MutationOperationKind::CgroupMemoryHigh,
        ),
        (
            CgroupControl::IoWeight(100),
            MutationOperationKind::CgroupIoWeight,
        ),
    ];
    let mut capabilities = std::collections::BTreeSet::new();
    for (control, expected) in knobs {
        let candidate = CandidateAction::CgroupControl {
            plan: CgroupControlActionPlan::manual(
                std::path::PathBuf::from("/nonexistent"),
                control,
                ObjectiveKind::GameRunnableLatency,
            ),
        };
        assert_eq!(candidate.operation_kind(), expected);
        assert_ne!(expected, MutationOperationKind::CgroupPlacement);
        assert!(expected.is_manual_only());
        assert!(CandidateExecutablePlan::from_candidate(&candidate).is_none());
        assert!(capabilities.insert(expected.capability_id().unwrap()));
    }
    assert_eq!(capabilities.len(), 9);
}

#[test]
fn a_block_scheduler_candidate_is_its_own_manual_only_operation_and_never_an_executable_plan() {
    use crate::{
        actions::block_scheduler::BlockSchedulerRequest,
        autotune::{
            objective::ObjectiveKind,
            planning::{
                candidate::CandidateAction,
                executable_plan::{BlockSchedulerActionPlan, CandidateExecutablePlan},
            },
        },
    };
    let candidate = CandidateAction::BlockScheduler {
        plan: BlockSchedulerActionPlan::manual(
            BlockSchedulerRequest {
                device: "disk1".into(),
                scheduler: "bfq".into(),
                authorized: Default::default(),
                protected: Default::default(),
            },
            ObjectiveKind::GameRunnableLatency,
        ),
    };
    assert_eq!(
        candidate.operation_kind(),
        MutationOperationKind::BlockIoScheduler
    );
    assert!(MutationOperationKind::BlockIoScheduler.is_manual_only());
    assert!(CandidateExecutablePlan::from_candidate(&candidate).is_none());
    assert_eq!(
        MutationOperationKind::BlockIoScheduler.capability_id(),
        Some("block_io_scheduler")
    );
}

#[test]
fn an_scx_strategy_candidate_is_manual_only_never_executable_and_its_token_is_manual_recovery() {
    use crate::{
        actions::RollbackToken,
        autotune::{
            objective::ObjectiveKind,
            planning::{
                candidate::CandidateAction,
                executable_plan::{CandidateExecutablePlan, ScxStrategyPlan},
            },
        },
        integrations::scx_loader::{Restore, SwitchPlan},
    };
    let switch = SwitchPlan {
        to: "scx_lavd".into(),
        mode: 0,
        method: "SwitchScheduler",
        restore: Restore::SwitchBack {
            name: "scx_bpfland".into(),
            mode: 1,
        },
        args: Vec::new(),
    };
    let token = RollbackToken::ScxSchedulerRestore {
        records: vec![switch.restore_record()],
    };
    let candidate = CandidateAction::ScxStrategy {
        plan: ScxStrategyPlan::manual(switch, ObjectiveKind::GameRunnableLatency),
    };
    assert_eq!(
        candidate.operation_kind(),
        MutationOperationKind::SchedExtSchedulerStrategy
    );
    assert!(MutationOperationKind::SchedExtSchedulerStrategy.is_manual_only());
    assert!(CandidateExecutablePlan::from_candidate(&candidate).is_none());
    assert_eq!(token.kind(), "scx-scheduler-restore");
    let json = serde_json::to_string(&token).unwrap();
    assert_eq!(serde_json::from_str::<RollbackToken>(&json).unwrap(), token);
    let descriptor = crate::actions::rollback::recovery::recovery_descriptor(&token);
    assert_eq!(
        descriptor.automatic_recovery,
        crate::actions::rollback::recovery::AutomaticRecoveryCapability::ManualOnly
    );
    assert_eq!(
        crate::autotune::emergency_restore::manual_restore_command_for_token(&token),
        "scxctl switch --sched scx_bpfland --mode 1"
    );
    let error = crate::autotune::emergency_restore::restore_rollback_token(&token).unwrap_err();
    assert!(error.to_string().contains("offline authorization"));
}

#[test]
fn a_numa_migration_candidate_is_its_own_manual_only_operation_and_never_an_executable_plan() {
    use crate::{
        actions::numa_migration::NumaMigrationAction,
        autotune::{
            objective::ObjectiveKind,
            planning::{
                candidate::CandidateAction,
                executable_plan::{CandidateExecutablePlan, NumaMigrationActionPlan},
            },
        },
    };
    let candidate = CandidateAction::NumaMigration {
        plan: NumaMigrationActionPlan::manual(
            NumaMigrationAction::new(1, 1, vec![0], vec![1]),
            ObjectiveKind::GameRunnableLatency,
        ),
    };
    assert_eq!(
        candidate.operation_kind(),
        MutationOperationKind::NumaMigratePages
    );
    assert!(MutationOperationKind::NumaMigratePages.is_manual_only());
    assert!(CandidateExecutablePlan::from_candidate(&candidate).is_none());
}

#[test]
fn the_three_network_sysctl_operations_are_distinct_manual_only_and_never_executable() {
    use crate::{
        actions::net_sysctl::NetKnob,
        autotune::{
            objective::ObjectiveKind,
            planning::{
                candidate::CandidateAction,
                executable_plan::{CandidateExecutablePlan, NetSysctlActionPlan},
            },
        },
    };
    let mut seen = std::collections::BTreeSet::new();
    for (knob, expected) in [
        (
            NetKnob::CongestionControl("bbr".into()),
            MutationOperationKind::NetTcpCongestionControl,
        ),
        (
            NetKnob::TcpRmem([4096, 8192, 16384]),
            MutationOperationKind::NetTcpBuffers,
        ),
        (
            NetKnob::TcpWmem([4096, 8192, 16384]),
            MutationOperationKind::NetTcpBuffers,
        ),
        (NetKnob::BusyPollUs(10), MutationOperationKind::NetBusyPoll),
    ] {
        let candidate = CandidateAction::NetSysctl {
            plan: NetSysctlActionPlan::manual(knob, ObjectiveKind::GameRunnableLatency),
        };
        assert_eq!(candidate.operation_kind(), expected);
        assert!(expected.is_manual_only());
        assert!(CandidateExecutablePlan::from_candidate(&candidate).is_none());
        seen.insert(expected);
    }
    assert_eq!(seen.len(), 3);
}
