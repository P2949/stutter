use crate::{
    autotune::{
        candidate::CandidateAction,
        controller::ControllerRuntimeState,
        objective::{ObjectiveKind, ObjectiveSignalQuality},
        observation::AutotuneObservation,
        system_context::SystemContextSnapshot,
    },
    daemon::{DaemonPolicy, capabilities::DaemonCapabilities, health::SystemHealthSnapshot},
    profiles::Profile,
};

pub mod cgroup;
pub mod cpu_affinity;
pub mod cpu_power;
pub mod gpu_power;
pub mod ioprio;
pub mod irq_affinity;
pub mod nice;
pub mod uclamp;
pub mod vm_knob;

#[derive(Clone, Debug)]
pub struct CandidateProposal {
    pub candidate: CandidateAction,
    pub provider: &'static str,
    pub confidence: f32,
    pub deny_reasons: Vec<String>,
    pub objective: ObjectiveKind,
    pub rank_hint: u32,
}

pub trait CandidateProvider {
    fn family(&self) -> &'static str;
    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal>;
}

pub struct CandidateProviderInput<'a> {
    pub observation: &'a AutotuneObservation,
    pub daemon_policy: &'a DaemonPolicy,
    pub capabilities: &'a DaemonCapabilities,
    pub system_health: &'a SystemHealthSnapshot,
    pub system_context: &'a SystemContextSnapshot,
    pub controller_state: &'a ControllerRuntimeState,
    pub profiles: &'a [Profile],
}

pub(crate) fn signal_quality_confidence_weight(quality: ObjectiveSignalQuality) -> f32 {
    match quality {
        ObjectiveSignalQuality::Direct => 1.0,
        ObjectiveSignalQuality::Derived => 1.0,
        ObjectiveSignalQuality::Approximate => 0.85,
        ObjectiveSignalQuality::Missing => 0.50,
    }
}

#[derive(Default)]
pub struct CandidateProviderRegistry {
    providers: Vec<Box<dyn CandidateProvider>>,
}

impl CandidateProviderRegistry {
    pub fn default_for_policy(policy: &DaemonPolicy) -> Self {
        let mut registry = Self::default();
        registry.register(Box::new(cpu_affinity::CpuAffinityProvider));

        if policy.mode == crate::daemon::policy::DaemonMode::Suggest
            || policy.max_safety_class >= crate::actions::SafetyClass::ReversibleMediumRisk
        {
            registry.register(Box::new(nice::NiceProvider));
            registry.register(Box::new(ioprio::IoPrioProvider));
            registry.register(Box::new(uclamp::UclampProvider));
            registry.register(Box::new(cgroup::CgroupProvider));
        }

        if policy.mode == crate::daemon::policy::DaemonMode::Suggest
            && policy.allow_system_wide_suggestions
        {
            registry.register(Box::new(irq_affinity::IrqAffinityProvider));
            registry.register(Box::new(cpu_power::CpuPowerProvider));
            registry.register(Box::new(gpu_power::GpuPowerProvider));
            registry.register(Box::new(vm_knob::VmKnobProvider));
        }

        registry
    }

    pub fn register(&mut self, provider: Box<dyn CandidateProvider>) {
        self.providers.push(provider);
    }

    pub fn families(&self) -> Vec<&'static str> {
        self.providers
            .iter()
            .map(|provider| provider.family())
            .collect()
    }

    pub fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        self.providers
            .iter()
            .filter(|provider| family_enabled(input.daemon_policy, provider.family()))
            .flat_map(|provider| provider.propose(input))
            .collect()
    }
}

fn family_enabled(policy: &DaemonPolicy, family: &str) -> bool {
    let enabled = policy.enabled_action_families.is_empty()
        || policy
            .enabled_action_families
            .iter()
            .any(|enabled| family_matches(family, enabled));
    let denied = policy
        .denied_action_families
        .iter()
        .any(|denied| family_matches(family, denied));

    enabled && !denied
}

fn family_matches(family: &str, configured: &str) -> bool {
    family == configured
        || family.strip_prefix(configured).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        autotune::{
            controller::ControllerRuntimeState,
            observation::{ActiveTaskSnapshot, AutotuneObservation, ProtectedTask},
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

    #[test]
    fn registry_includes_safe_and_suggest_first_provider_families() {
        let registry = CandidateProviderRegistry::default_for_policy(
            &policy_with_system_wide_suggestions(DaemonMode::Suggest),
        );
        let families = registry.families();

        assert!(families.contains(&"cpu_affinity_profile"));
        assert!(families.contains(&"nice"));
        assert!(families.contains(&"ionice"));
        assert!(families.contains(&"uclamp"));
        assert!(families.contains(&"cgroup_placement"));
        assert!(families.contains(&"irq_affinity"));
        assert!(families.contains(&"cpu_power"));
        assert!(families.contains(&"gpu_power"));
        assert!(families.contains(&"vm_knob"));
    }

    #[test]
    fn vm_knob_provider_stays_silent_without_memory_or_swap_evidence() {
        let policy = policy(DaemonMode::Suggest);
        let provider = vm_knob::VmKnobProvider;
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            active_target_count: 1,
            data_quality: OnlineDataQuality::High,
            primary_situation: SituationKind::IoPressure,
            focus_kind: Some(FocusGroupKind::Browser),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::IoPressure;

        let system_context = system_context_for_observation(&observation);
        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    #[test]
    fn cgroup_provider_requires_configured_allowlist() {
        let policy = policy(DaemonMode::Suggest);
        let provider = cgroup::CgroupProvider;
        let observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::CompileCpuBound,
            focus_kind: Some(FocusGroupKind::Compile),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };

        let system_context = system_context_for_observation(&observation);
        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    #[test]
    fn cgroup_provider_moves_only_allowed_non_protected_target_tasks() {
        let policy = policy_with_compile_cgroup();
        let provider = cgroup::CgroupProvider;
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::CompileCpuBound,
            focus_kind: Some(FocusGroupKind::Compile),
            focus_confidence: 0.95,
            active_tasks: vec![
                ActiveTaskSnapshot {
                    tid: 1234,
                    process_pid: 1234,
                    comm: "rustc".to_owned(),
                    class: TaskClass::Compiler,
                    process_starttime_ticks: Some(10),
                    task_starttime_ticks: Some(10),
                    cgroup_path: Some("/user.slice/app.scope".to_owned()),
                },
                ActiveTaskSnapshot {
                    tid: 1235,
                    process_pid: 1234,
                    comm: "ld.lld".to_owned(),
                    class: TaskClass::Linker,
                    process_starttime_ticks: Some(10),
                    task_starttime_ticks: Some(11),
                    cgroup_path: Some("/user.slice/app.scope".to_owned()),
                },
                ActiveTaskSnapshot {
                    tid: 77,
                    process_pid: 77,
                    comm: "pipewire".to_owned(),
                    class: TaskClass::AudioRealtime,
                    process_starttime_ticks: Some(20),
                    task_starttime_ticks: Some(20),
                    cgroup_path: Some("/user.slice/session.scope".to_owned()),
                },
            ],
            protected_tasks: vec![ProtectedTask {
                tid: 77,
                process_pid: 77,
                comm: "pipewire".to_owned(),
                class: TaskClass::AudioRealtime,
                reason: "audio realtime task".to_owned(),
            }],
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();

        let system_context = system_context_for_observation(&observation);
        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        let CandidateAction::CgroupPlacement { plan } = &proposals[0].candidate else {
            panic!("expected cgroup candidate");
        };
        assert_eq!(
            plan.action.target_cgroup,
            std::path::PathBuf::from("/user.slice/stutter-compile.slice")
        );
        let tids = plan
            .action
            .targets
            .iter()
            .map(|target| target.identity.tid)
            .collect::<Vec<_>>();
        assert_eq!(tids, vec![1234, 1235]);
    }

    #[test]
    fn process_local_providers_do_not_fallback_without_active_snapshots_in_apply_modes() {
        let policy = apply_medium_policy_with_compile_cgroup();
        let mut observation =
            provider_observation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
        observation.capabilities.ionice_available = true;
        observation.capabilities.uclamp_available = true;
        let system_context = system_context_for_observation(&observation);
        let input = CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        };

        assert!(nice::NiceProvider.propose(&input).is_empty());
        assert!(uclamp::UclampProvider.propose(&input).is_empty());
        assert!(cgroup::CgroupProvider.propose(&input).is_empty());

        let mut io_observation =
            provider_observation(SituationKind::IoPressure, FocusGroupKind::Desktop);
        io_observation.capabilities.ionice_available = true;
        let io_system_context = system_context_for_observation(&io_observation);
        let io_input = CandidateProviderInput {
            observation: &io_observation,
            daemon_policy: &policy,
            capabilities: &io_observation.capabilities,
            system_health: &io_observation.system_health,
            system_context: &io_system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        };

        assert!(ioprio::IoPrioProvider.propose(&io_input).is_empty());
    }

    #[test]
    fn suggest_mode_marks_fallback_root_target_selection() {
        let policy = policy(DaemonMode::Suggest);
        let observation =
            provider_observation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
        let system_context = system_context_for_observation(&observation);
        let input = CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        };

        let proposals = nice::NiceProvider.propose(&input);

        assert_eq!(proposals.len(), 1);
        assert!(
            proposals[0]
                .deny_reasons
                .iter()
                .any(|reason| reason.contains("target_selection_fallback_root"))
        );
        let CandidateAction::Nice { plan } = &proposals[0].candidate else {
            panic!("expected nice candidate");
        };
        assert!(
            plan.evidence
                .iter()
                .any(|evidence| evidence.signal == "target_selection_fallback_root")
        );
    }

    #[test]
    fn nice_provider_targets_compile_active_tasks_not_only_root_pid() {
        let policy = apply_medium_policy_with_compile_cgroup();
        let provider = nice::NiceProvider;
        let mut observation =
            provider_observation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
        observation.active_tasks = vec![
            provider_task(1234, 1234, "cargo", TaskClass::BuildJob),
            provider_task(1235, 1234, "rustc", TaskClass::Compiler),
            provider_task(1236, 1234, "ld.lld", TaskClass::Linker),
            provider_task(1237, 1234, "pipewire", TaskClass::AudioRealtime),
            provider_task(1238, 1234, "unknown-helper", TaskClass::Unknown),
        ];
        let system_context = system_context_for_observation(&observation);
        let input = CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        };

        let proposals = provider.propose(&input);

        assert_eq!(proposals.len(), 1);
        let CandidateAction::Nice { plan } = &proposals[0].candidate else {
            panic!("expected nice candidate");
        };
        assert_eq!(
            plan.action
                .targets
                .iter()
                .map(|target| target.tid)
                .collect::<Vec<_>>(),
            vec![1234, 1235, 1236]
        );
        assert_eq!(
            plan.action
                .targets
                .iter()
                .map(|target| target.process_pid)
                .collect::<Vec<_>>(),
            vec![Some(1234), Some(1234), Some(1234)]
        );
        assert!(proposals[0].deny_reasons.is_empty());
    }

    #[test]
    fn ionice_provider_targets_active_tree_and_excludes_protected_or_unknown_tasks() {
        let policy = apply_medium_policy_with_compile_cgroup();
        let provider = ioprio::IoPrioProvider;
        let mut observation =
            provider_observation(SituationKind::IoPressure, FocusGroupKind::Desktop);
        observation.capabilities.ionice_available = true;
        observation.active_tasks = vec![
            provider_task(1234, 1234, "game", TaskClass::Game),
            provider_task(1235, 1234, "game-worker", TaskClass::GameWorkerThread),
            provider_task(1236, 1234, "helper", TaskClass::Helper),
            provider_task(1237, 1234, "pipewire", TaskClass::AudioRealtime),
            provider_task(1238, 1234, "compositor", TaskClass::Compositor),
            provider_task(1239, 1234, "unknown", TaskClass::Unknown),
        ];
        let system_context = system_context_for_observation(&observation);
        let input = CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        };

        let proposals = provider.propose(&input);

        assert_eq!(proposals.len(), 1);
        let CandidateAction::IoPrio { plan } = &proposals[0].candidate else {
            panic!("expected ionice candidate");
        };
        assert_eq!(
            plan.action
                .targets
                .iter()
                .map(|target| target.tid)
                .collect::<Vec<_>>(),
            vec![1234, 1235, 1236]
        );
        assert!(proposals[0].deny_reasons.is_empty());
    }

    #[test]
    fn uclamp_provider_targets_game_render_and_worker_active_tasks_not_only_root_pid() {
        let policy = apply_medium_policy_with_compile_cgroup();
        let provider = uclamp::UclampProvider;
        let mut observation = provider_observation(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
        );
        observation.capabilities.uclamp_available = true;
        observation.active_tasks = vec![
            provider_task(1234, 1234, "game", TaskClass::Game),
            provider_task(1235, 1234, "render", TaskClass::GameRenderThread),
            provider_task(1236, 1234, "worker", TaskClass::GameWorkerThread),
            provider_task(1237, 1234, "input", TaskClass::Input),
            provider_task(1238, 1234, "irq", TaskClass::IrqThread),
            provider_task(1239, 1234, "service", TaskClass::Service),
            provider_task(1240, 1234, "unknown", TaskClass::Unknown),
        ];
        let system_context = system_context_for_observation(&observation);
        let input = CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        };

        let proposals = provider.propose(&input);

        assert_eq!(proposals.len(), 1);
        let CandidateAction::Uclamp { plan } = &proposals[0].candidate else {
            panic!("expected uclamp candidate");
        };
        assert_eq!(
            plan.action
                .targets
                .iter()
                .map(|target| target.tid)
                .collect::<Vec<_>>(),
            vec![1234, 1235, 1236]
        );
        assert!(proposals[0].deny_reasons.is_empty());
    }

    fn provider_task(
        tid: u32,
        process_pid: u32,
        comm: &str,
        class: TaskClass,
    ) -> ActiveTaskSnapshot {
        let process_starttime_ticks = Some(10);
        let task_starttime_ticks = if tid == process_pid {
            process_starttime_ticks
        } else {
            Some(u64::from(tid))
        };

        ActiveTaskSnapshot {
            tid,
            process_pid,
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
}
