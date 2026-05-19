//! Shared fixtures for `autotune::planner` test modules.
//!
//! Owns synthetic planner inputs, candidate builders, dry-run fakes, and common assertions.
//! Does not own production planner behavior or test assertions for a specific planner behavior area.

use super::super::*;
pub(crate) use crate::{
    actions::{
        RollbackToken, SafetyClass, TaskIdentity,
        cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
        cpu_power::CpuPowerAction,
        gpu_power::GpuPowerAction,
        ioprio::{IoPrioAction, IoPrioPolicy, IoPrioValue},
        irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
        nice::{NiceAction, NicePolicy},
        uclamp::{UclampAction, UclampValues},
        vm_knobs::{VmKnobAction, VmKnobChange},
    },
    affinity::CpuMask,
    autotune::{
        candidate::{
            CandidateAction, CandidateDryRunRecord, CandidateDryRunner, CandidateEvidence,
            CgroupPlacementActionPlan, CpuPowerActionPlan, IoPrioActionPlan, IrqAffinityActionPlan,
            NiceActionPlan, UclampActionPlan,
        },
        candidate_memory::CandidateMemoryResult,
        controller::{ActiveExperiment, ControllerRuntimeState},
        experiment::{ExperimentId, WindowScore},
        kept::{ActiveProfileState, KeptCandidateState},
        objective::{ObjectiveKind, ObjectiveSignalQuality, ObjectiveSignals},
        observation::{
            ActiveAffinitySnapshot, ActiveConfigSnapshot, ActiveNiceSnapshot, ActiveTaskSnapshot,
            AutotuneObservation, WorkloadIdentity,
        },
        providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput, CandidateProviderRegistry,
        },
        quality::OnlineDataQuality,
        state::SituationKind,
        system_context::SystemContextSnapshot,
        workload_policy::WorkloadPolicyMatrix,
    },
    daemon::{
        DaemonPolicy,
        capabilities::DaemonCapabilities,
        health::SystemHealthSnapshot,
        policy::{ActionSource, DaemonMode},
    },
    daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
    focus::FocusGroupKind,
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
    scorer::StutterScore,
};

pub(crate) fn policy(mode: DaemonMode) -> DaemonPolicy {
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        mode,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    config.safety.allow_system_wide_suggestions = true;
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

pub(crate) fn rollback_token() -> RollbackToken {
    RollbackToken::CpuAffinityRestoreFile {
        path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
        affected_tasks: 1,
    }
}

pub(crate) fn active_profile_state_with_kept(kept: KeptCandidateState) -> ActiveProfileState {
    let mut state = ActiveProfileState::default();
    state
        .record_kept_candidate(
            kept,
            crate::autotune::comparison::ExperimentResult::Improved {
                improvement_percent: 10.0,
            },
        )
        .unwrap();
    state
}

pub(crate) fn active_nice_snapshot(tid: u32, nice: i32) -> ActiveConfigSnapshot {
    ActiveConfigSnapshot {
        nice: ActiveNiceSnapshot {
            per_tid: std::collections::BTreeMap::from([(tid, nice)]),
        },
        ..ActiveConfigSnapshot::default()
    }
}

pub(crate) fn observation_with_active_nice(
    situation: SituationKind,
    focus_kind: FocusGroupKind,
    tid: u32,
    nice: i32,
) -> AutotuneObservation {
    let mut observation = observation_for_situation(situation, focus_kind);
    observation.active_config_snapshot = Some(active_nice_snapshot(tid, nice));
    observation
}

pub(crate) fn observation_with_cpu_affinity(mask: &str) -> AutotuneObservation {
    let mut observation = observation_for_situation(
        SituationKind::GameCpuSchedulerPressure,
        FocusGroupKind::Game,
    );
    observation.active_tasks = vec![ActiveTaskSnapshot {
        tid: 1234,
        process_pid: 1234,
        comm: "game-main".to_owned(),
        class: TaskClass::Game,
        process_starttime_ticks: Some(10),
        task_starttime_ticks: Some(10),
        cgroup_path: Some("/user.slice/game.scope".to_owned()),
    }];
    observation.active_config_snapshot = Some(ActiveConfigSnapshot {
        affinity: ActiveAffinitySnapshot {
            per_tid: std::collections::BTreeMap::from([(1234, mask.to_owned())]),
        },
        ..ActiveConfigSnapshot::default()
    });
    observation
}

pub(crate) fn suggest_policy_with_compile_cgroup() -> DaemonPolicy {
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        DaemonMode::Suggest,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(
        "/user.slice/stutter-compile.slice",
    ));
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

pub(crate) fn observation() -> AutotuneObservation {
    let mut observation = AutotuneObservation {
        now_unix_nanos: 1_000,
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        scored_task_count: 1,
        interval_count: 3,
        scored_samples: 100,
        score: StutterScore {
            total: 100,
            over_1ms: 10,
            ..StutterScore::default()
        },
        data_quality: OnlineDataQuality::High,
        primary_situation: SituationKind::GameCpuSchedulerPressure,
        focus_kind: Some(FocusGroupKind::Game),
        focus_confidence: 0.95,
        focus_roots: vec![1234],
        workload_identity: Some(WorkloadIdentity {
            root_pid: 1234,
            process_starttime_ticks: Some(10),
            exe_dev: Some(1),
            exe_ino: Some(2),
            cgroup_path: Some("/user.slice/app.scope".to_owned()),
            focus_kind: Some(FocusGroupKind::Game),
            class_distribution: std::collections::BTreeMap::from([("game".to_owned(), 1)]),
            stable_hash: "test-workload".to_owned(),
        }),
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation
}

#[derive(Clone)]
pub(crate) struct StaticProvider {
    pub(crate) candidate: CandidateAction,
}

impl CandidateProvider for StaticProvider {
    fn family(&self) -> &'static str {
        self.candidate.action_kind()
    }

    fn propose(&self, _input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        vec![CandidateProposal {
            candidate: self.candidate.clone(),
            provider: self.family(),
            confidence: 1.0,
            deny_reasons: Vec::new(),
            objective: self.candidate.objective(),
            rank_hint: 1,
        }]
    }
}

pub(crate) fn cpu_affinity_candidate(name: &str) -> CandidateAction {
    CandidateAction::cpu_affinity_profile(
        Profile {
            name: name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        },
        1234,
    )
}

pub(crate) fn cgroup_candidate(target_cgroup: &str) -> CandidateAction {
    CandidateAction::CgroupPlacement {
        plan: CgroupPlacementActionPlan {
            name: format!("cgroup-to-{target_cgroup}"),
            action: CgroupPlacementAction {
                cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
                target_cgroup: std::path::PathBuf::from(target_cgroup),
                targets: vec![CgroupPlacementTarget {
                    identity: TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: Some("rustc".to_owned()),
                        starttime_ticks: None,
                    },
                    class: TaskClass::Compiler,
                }],
                cpuset_cpus: None,
                cpuset_mems: None,
            },
            target_root_pid: Some(1234),
            evidence: Vec::new(),
            objective: ObjectiveKind::CompileThroughputWithForegroundProtection,
        },
    }
}

pub(crate) fn nice_candidate() -> CandidateAction {
    CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "nice-background-compile".to_owned(),
            action: NiceAction {
                targets: vec![TaskIdentity {
                    tid: 1234,
                    process_pid: Some(1234),
                    comm: Some("rustc".to_owned()),
                    starttime_ticks: None,
                }],
                nice: 5,
                policy: NicePolicy::default(),
            },
            target_root_pid: Some(1234),
            evidence: Vec::new(),
            objective: ObjectiveKind::DesktopInteractivity,
        },
    }
}

pub(crate) fn task_identity() -> TaskIdentity {
    TaskIdentity {
        tid: 1234,
        process_pid: Some(1234),
        comm: Some("stutter-test".to_owned()),
        starttime_ticks: None,
    }
}

pub(crate) fn uclamp_candidate(name: &str) -> CandidateAction {
    CandidateAction::Uclamp {
        plan: UclampActionPlan {
            name: name.to_owned(),
            action: UclampAction {
                targets: vec![task_identity()],
                values: UclampValues {
                    sched_util_min: Some(128),
                    sched_util_max: Some(1024),
                },
            },
            target_root_pid: Some(1234),
            evidence: Vec::new(),
            objective: ObjectiveKind::DesktopInteractivity,
        },
    }
}

pub(crate) fn ioprio_candidate(name: &str) -> CandidateAction {
    CandidateAction::IoPrio {
        plan: IoPrioActionPlan {
            name: name.to_owned(),
            action: IoPrioAction {
                targets: vec![task_identity()],
                ioprio: IoPrioValue::idle(),
                policy: IoPrioPolicy {
                    allow_ioprio_changes: true,
                    require_strong_block_io_evidence: false,
                    strong_block_io_evidence: true,
                    ..IoPrioPolicy::default()
                },
            },
            target_root_pid: Some(1234),
            evidence: Vec::new(),
            objective: ObjectiveKind::IoLatency,
        },
    }
}

pub(crate) fn irq_affinity_candidate(name: &str) -> CandidateAction {
    irq_affinity_candidate_with_risk(name, IrqAffinityRisk::ReversibleMediumRisk)
}

pub(crate) fn high_risk_irq_affinity_candidate(name: &str) -> CandidateAction {
    irq_affinity_candidate_with_risk(name, IrqAffinityRisk::HighRisk)
}

pub(crate) fn irq_affinity_candidate_with_risk(
    name: &str,
    risk: IrqAffinityRisk,
) -> CandidateAction {
    let evidence = IrqAffinityEvidence {
        strong_irq_evidence: true,
        stable_irq_identity: true,
        known_device_mapping: true,
        observed_irq: Some(42),
        observed_device_hint: Some("test-device".to_owned()),
        reason: "test IRQ evidence".to_owned(),
    };

    CandidateAction::IrqAffinity {
        plan: IrqAffinityActionPlan {
            name: name.to_owned(),
            action: IrqAffinityAction::new(
                42,
                "test-device".to_owned(),
                "1".to_owned(),
                risk,
                evidence,
            ),
            evidence: Vec::new(),
            objective: ObjectiveKind::IrqOverlapReduction,
        },
    }
}

pub(crate) fn cpu_power_candidate(name: &str) -> CandidateAction {
    CandidateAction::CpuPower {
        plan: CpuPowerActionPlan {
            name: name.to_owned(),
            action: CpuPowerAction {
                sysfs_root: std::path::PathBuf::from("/sys"),
                cpus: vec![0],
                scaling_governor: Some("performance".to_owned()),
                energy_performance_preference: Some("performance".to_owned()),
            },
            evidence: Vec::new(),
            objective: ObjectiveKind::GameRunnableLatency,
        },
    }
}

pub(crate) fn gpu_power_candidate(name: &str) -> CandidateAction {
    CandidateAction::GpuPower {
        plan: crate::autotune::candidate::GpuPowerActionPlan {
            name: name.to_owned(),
            action: GpuPowerAction {
                sysfs_root: std::path::PathBuf::from("/sys"),
                drm_card: "card0".to_owned(),
                power_dpm_force_performance_level: Some("high".to_owned()),
                pp_power_profile_mode: None,
            },
            evidence: vec![CandidateEvidence::new("gpu_bound", "true", 0.9)],
            objective: ObjectiveKind::ThermalRecovery,
        },
    }
}

pub(crate) fn vm_knob_candidate(name: &str) -> CandidateAction {
    CandidateAction::VmKnob {
        plan: crate::autotune::candidate::VmKnobActionPlan {
            name: name.to_owned(),
            action: VmKnobAction {
                root: std::path::PathBuf::from("/proc/sys"),
                changes: vec![VmKnobChange {
                    path: std::path::PathBuf::from("vm/swappiness"),
                    value: "10".to_owned(),
                }],
            },
            evidence: vec![CandidateEvidence::new("swap_activity", "high", 0.9)],
            objective: ObjectiveKind::IoLatency,
        },
    }
}

pub(crate) fn candidate_for_action_kind(action_kind: &str) -> CandidateAction {
    match action_kind {
        "cpu_affinity_profile" => cpu_affinity_candidate("fixture-cpu-affinity"),
        "nice" => nice_candidate(),
        "ionice" => ioprio_candidate("fixture-ionice"),
        "uclamp" => uclamp_candidate("fixture-uclamp"),
        "cgroup_placement" => cgroup_candidate("/user.slice/stutter-compile.slice"),
        "irq_affinity" => irq_affinity_candidate("fixture-irq"),
        "cpu_power" => cpu_power_candidate("fixture-cpu-power"),
        "gpu_power" => gpu_power_candidate("fixture-gpu-power"),
        "vm_knob" => vm_knob_candidate("fixture-vm-knob"),
        other => panic!("unsupported fixture action_kind {other}"),
    }
}

pub(crate) fn observation_for_situation(
    situation: SituationKind,
    focus_kind: FocusGroupKind,
) -> AutotuneObservation {
    let mut observation = observation();
    observation.primary_situation = situation;
    observation.focus_kind = Some(focus_kind);
    observation
}

pub(crate) fn enable_capability_for_candidate(
    observation: &mut AutotuneObservation,
    candidate: &CandidateAction,
) {
    match candidate.action_kind() {
        "uclamp" => observation.capabilities.uclamp_available = true,
        "ionice" => observation.capabilities.ionice_available = true,
        "irq_affinity" => observation.capabilities.irq_affinity_available = true,
        "gpu_power" => observation.capabilities.gpu_sysfs_available = true,
        _ => {}
    }
}

pub(crate) fn eligible_dry_run(candidate: &CandidateAction) -> CandidateDryRunRecord {
    CandidateDryRunRecord {
        candidate_name: candidate.candidate_name().to_owned(),
        affected_tasks: 1,
        warnings: Vec::new(),
        safety_class: candidate.safety_class(),
        eligible: true,
        reason: None,
    }
}

#[derive(Default)]
pub(crate) struct CountingDryRunner {
    pub(crate) calls: usize,
}

impl CandidateDryRunner for CountingDryRunner {
    fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
        self.calls += 1;
        eligible_dry_run(candidate)
    }
}

pub(crate) fn proposal_for_candidate(
    candidate: CandidateAction,
    confidence: f32,
) -> CandidateProposal {
    CandidateProposal {
        candidate: candidate.clone(),
        provider: candidate.action_kind(),
        confidence,
        deny_reasons: Vec::new(),
        objective: candidate.objective(),
        rank_hint: 1,
    }
}

pub(crate) fn evaluate_candidate_with_runner(
    policy: &DaemonPolicy,
    observation: &AutotuneObservation,
    capabilities: &crate::daemon::capabilities::DaemonCapabilities,
    controller_state: &ControllerRuntimeState,
    candidate: CandidateAction,
    confidence: f32,
    dry_runner: &mut CountingDryRunner,
) -> CandidateEvaluation {
    let proposals = vec![proposal_for_candidate(candidate, confidence)];
    let mut evaluations = super::super::evaluate_proposals_with_runner(
        PlannerInput {
            observation,
            daemon_policy: policy,
            capabilities,
            system_health: &observation.system_health,
            controller_state,
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        },
        proposals,
        dry_runner,
    );

    assert_eq!(evaluations.len(), 1);
    evaluations.remove(0)
}

pub(crate) fn evaluate_static_candidate(
    mode: DaemonMode,
    situation: SituationKind,
    focus_kind: FocusGroupKind,
    candidate: CandidateAction,
) -> CandidateEvaluation {
    evaluate_static_candidate_with_confidence(mode, situation, focus_kind, candidate, 1.0)
}

pub(crate) fn evaluate_static_candidate_with_confidence(
    mode: DaemonMode,
    situation: SituationKind,
    focus_kind: FocusGroupKind,
    candidate: CandidateAction,
    confidence: f32,
) -> CandidateEvaluation {
    let policy = policy(mode);
    evaluate_static_candidate_with_policy(&policy, situation, focus_kind, candidate, confidence)
}

pub(crate) fn evaluate_static_candidate_with_policy(
    policy: &DaemonPolicy,
    situation: SituationKind,
    focus_kind: FocusGroupKind,
    candidate: CandidateAction,
    confidence: f32,
) -> CandidateEvaluation {
    let mut observation = observation_for_situation(situation, focus_kind);
    enable_capability_for_candidate(&mut observation, &candidate);
    let mut dry_runner = CountingDryRunner::default();

    evaluate_candidate_with_runner(
        policy,
        &observation,
        &observation.capabilities,
        &ControllerRuntimeState::default(),
        candidate,
        confidence,
        &mut dry_runner,
    )
}

pub(crate) fn assert_autonomous_denial_mentions_context(
    evaluation: &CandidateEvaluation,
    situation: SituationKind,
    action_kind: &str,
) {
    assert!(!evaluation.eligible);
    assert!(
        evaluation
            .deny_reasons
            .contains(&CandidateDenyReason::NotAutonomousForWorkload)
    );
    assert!(
        evaluation.deny_messages.iter().any(|message| {
            message.contains(&format!("{situation:?}"))
                && message.contains(action_kind)
                && message.contains("autonomous_families=")
        }),
        "missing autonomous denial context in {:?}",
        evaluation.deny_messages
    );
}

pub(crate) fn window_score(total: u64) -> WindowScore {
    WindowScore {
        started_unix_nanos: 1,
        finished_unix_nanos: 2,
        interval_count: 5,
        scored_samples: 100,
        scored_task_count: 1,
        score: StutterScore {
            total,
            ..StutterScore::default()
        },
    }
}
