use crate::{
    autotune::{
        candidate::CandidateAction, controller::ControllerRuntimeState, objective::ObjectiveKind,
        observation::AutotuneObservation,
    },
    daemon::{DaemonCapabilities, DaemonPolicy, SystemHealthSnapshot},
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
    pub controller_state: &'a ControllerRuntimeState,
    pub profiles: &'a [Profile],
}

#[derive(Default)]
pub struct CandidateProviderRegistry {
    providers: Vec<Box<dyn CandidateProvider>>,
}

impl CandidateProviderRegistry {
    pub fn default_for_policy(policy: &DaemonPolicy) -> Self {
        let mut registry = Self::default();
        registry.register(Box::new(cpu_affinity::CpuAffinityProvider));

        if policy.mode == crate::daemon::DaemonMode::Suggest
            || policy.max_safety_class >= crate::actions::SafetyClass::ReversibleMediumRisk
        {
            registry.register(Box::new(nice::NiceProvider));
            registry.register(Box::new(ioprio::IoPrioProvider));
            registry.register(Box::new(uclamp::UclampProvider));
            registry.register(Box::new(cgroup::CgroupProvider));
        }

        registry.register(Box::new(irq_affinity::IrqAffinityProvider));
        registry.register(Box::new(cpu_power::CpuPowerProvider));
        registry.register(Box::new(gpu_power::GpuPowerProvider));
        registry.register(Box::new(vm_knob::VmKnobProvider));

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
        daemon::{ActionSource, DaemonMode},
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

    #[test]
    fn registry_includes_safe_and_suggest_first_provider_families() {
        let registry = CandidateProviderRegistry::default_for_policy(&policy(DaemonMode::Suggest));
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
    fn vm_knob_provider_suggests_only_for_io_situation() {
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

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].candidate.action_kind(), "vm_knob");
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

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
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

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
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
}
