use super::*;
use crate::{
    actions::ActionId,
    autotune::{
        controller::ControllerRuntimeState,
        observation::{ActiveTaskSnapshot, AutotuneObservation, ProtectedTask},
        providers::{
            cgroup::CgroupProvider, ioprio::IoPrioProvider, nice::NiceProvider,
            uclamp::UclampProvider, vm_knob::VmKnobProvider,
        },
        quality::OnlineDataQuality,
        state::SituationKind,
    },
    daemon::{
        health::SystemHealthSnapshot,
        policy::{ActionSource, DaemonMode},
    },
    daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
    focus::FocusGroupKind,
    process_tree::TaskClass,
    system_inventory::DrmDeviceInventory,
};

fn policy(mode: DaemonMode) -> DaemonPolicy {
    let config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        mode,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

fn policy_with_system_wide_suggestions(mode: DaemonMode) -> DaemonPolicy {
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

fn policy_with_compile_cgroup() -> DaemonPolicy {
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

fn apply_medium_policy_with_compile_cgroup() -> DaemonPolicy {
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        DaemonMode::ApplyMediumRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    config.autotune.allow_medium_risk_apply = true;
    config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(
        "/user.slice/stutter-compile.slice",
    ));
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

fn system_context_for_observation(observation: &AutotuneObservation) -> SystemContextSnapshot {
    SystemContextSnapshot::from_observation(observation)
}

fn calibration_proposal(provider: &'static str, confidence: f32) -> CandidateProposal {
    CandidateProposal {
        candidate: CandidateAction::fake(
            ActionId::new(format!("{provider}:calibration")),
            SafetyClass::HighRisk,
        ),
        provider,
        confidence,
        deny_reasons: Vec::new(),
        rank_hint: 1,
    }
}

fn active_task_snapshot() -> ActiveTaskSnapshot {
    ActiveTaskSnapshot {
        tid: (1234).into(),
        process_pid: (1234).into(),
        comm: "game".to_owned(),
        class: TaskClass::Game,
        process_starttime_ticks: Some(10),
        task_starttime_ticks: Some(10),
        cgroup_path: None,
    }
}
fn provider_task(tid: u32, process_pid: u32, comm: &str, class: TaskClass) -> ActiveTaskSnapshot {
    let process_starttime_ticks = Some(10);
    let task_starttime_ticks = if tid == process_pid {
        process_starttime_ticks
    } else {
        Some(u64::from(tid))
    };

    ActiveTaskSnapshot {
        tid: tid.into(),
        process_pid: process_pid.into(),
        comm: comm.to_owned(),
        class,
        process_starttime_ticks,
        task_starttime_ticks,
        cgroup_path: Some("/user.slice/provider-test.scope".to_owned()),
    }
}

fn provider_observation(
    situation: SituationKind,
    focus_kind: FocusGroupKind,
) -> AutotuneObservation {
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        primary_situation: situation,
        focus_kind: Some(focus_kind),
        focus_confidence: 0.95,
        system_health: SystemHealthSnapshot {
            ok_for_apply: true,
            ..SystemHealthSnapshot::default()
        },
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = situation;
    observation
}

mod registry;

mod confidence;

mod vm_knob;

mod cgroup;

mod cpu_affinity;

mod nice;

mod ioprio;

mod uclamp;

#[test]
fn scheduler_pressure_affinity_candidates_use_the_games_runnable_latency() {
    let mut observation = provider_observation(
        SituationKind::GameCpuSchedulerPressure,
        FocusGroupKind::Game,
    );
    observation.active_tasks = vec![active_task_snapshot()];
    let profiles = vec![crate::profiles::Profile {
        name: "game-isolate-render".to_owned(),
        rules: vec![crate::profiles::ProfileRule {
            affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    }];
    let policy = policy(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let context = system_context_for_observation(&observation);
    let state = ControllerRuntimeState::default();
    let capabilities = crate::daemon::capabilities::DaemonCapabilities::default();
    let proposals = crate::autotune::providers::cpu_affinity::CpuAffinityProvider.propose(
        &CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &health,
            system_context: &context,
            controller_state: &state,
            profiles: &profiles,
        },
    );
    assert!(!proposals.is_empty());
    for proposal in proposals {
        assert_eq!(
            proposal.candidate.objective(),
            ObjectiveKind::GameRunnableLatency
        );
    }
}

#[test]
fn a_candidates_objective_is_the_same_at_every_stage() {
    use crate::{
        autotune::planning::plan_io::CandidatePlanFile, daemon::privilege::CandidatePlanRequest,
    };
    let candidate = CandidateAction::cpu_affinity_profile_for(
        crate::profiles::Profile {
            name: "game".to_owned(),
            rules: Vec::new(),
        },
        1234,
        ObjectiveKind::GameRunnableLatency,
    );
    let plan_file = CandidatePlanFile::from_candidate_with_policy(
        &candidate,
        None,
        &policy(DaemonMode::Suggest),
        crate::daemon::policy::PolicyIntent::Apply,
        None,
    );
    let request = CandidatePlanRequest::from_candidate(candidate.clone(), 0);
    assert_eq!(candidate.objective(), ObjectiveKind::GameRunnableLatency);
    assert_eq!(plan_file.objective, candidate.objective());
    assert_eq!(request.objective, candidate.objective());
}

#[test]
fn direct_runnable_pressure_makes_affinity_candidates_eligible_without_a_diagnosis() {
    let mut observation = provider_observation(SituationKind::Unknown, FocusGroupKind::Game);
    observation.active_tasks = vec![active_task_snapshot()];
    observation.recent_diagnoses = Vec::new();
    observation.objective_signals.runnable_latency_p99_ns = Some(3_000_000);
    observation.objective_signals.runnable_samples =
        Some(crate::autotune::objective::MINIMUM_RUNNABLE_SAMPLES);
    observation.refresh_situation_classification();
    assert_eq!(
        observation.primary_situation,
        SituationKind::GameCpuSchedulerPressure
    );

    let profiles = vec![crate::profiles::Profile {
        name: "game-isolate-render".to_owned(),
        rules: vec![crate::profiles::ProfileRule {
            affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    }];
    let policy = policy(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let context = system_context_for_observation(&observation);
    let state = ControllerRuntimeState::default();
    let capabilities = crate::daemon::capabilities::DaemonCapabilities::default();
    let proposals = crate::autotune::providers::cpu_affinity::CpuAffinityProvider.propose(
        &CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &health,
            system_context: &context,
            controller_state: &state,
            profiles: &profiles,
        },
    );
    assert!(!proposals.is_empty());
    assert!(
        proposals
            .iter()
            .all(|proposal| proposal.candidate.objective() == ObjectiveKind::GameRunnableLatency)
    );
}
