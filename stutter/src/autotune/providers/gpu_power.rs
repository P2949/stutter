use crate::{
    actions::gpu_power::GpuPowerAction,
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, GpuPowerActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuPowerCandidateEvidence {
    pub drm_card: String,
    pub render_node: Option<String>,
    pub pci_id: Option<String>,
    pub vendor: Option<String>,
    pub current_dpm: Option<String>,
    pub current_profile: Option<String>,
    pub active_for_focus: bool,
}

#[derive(Default)]
pub struct GpuPowerProvider;

impl CandidateProvider for GpuPowerProvider {
    fn family(&self) -> &'static str {
        "gpu_power"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !matches!(
            input.observation.primary_situation,
            SituationKind::GameGpuBound | SituationKind::BrowserGpuVideo
        ) || !input.system_health.ok_for_apply
        {
            return Vec::new();
        }

        let inventory = &input.system_context.inventory;
        if inventory.drm_devices.len() != 1 {
            return Vec::new();
        }
        let card = &inventory.drm_devices[0];
        let runtime_state =
            input
                .observation
                .active_config_snapshot
                .as_ref()
                .and_then(|snapshot| {
                    snapshot
                        .gpu_power
                        .devices
                        .iter()
                        .find(|device| device.device == card.name)
                });
        let structured_evidence = GpuPowerCandidateEvidence {
            drm_card: card.name.clone(),
            render_node: card.render_node.clone(),
            pci_id: None,
            vendor: None,
            current_dpm: runtime_state
                .and_then(|state| state.power_dpm_force_performance_level.clone()),
            current_profile: runtime_state.and_then(|state| state.pp_power_profile_mode.clone()),
            active_for_focus: true,
        };
        let candidate = CandidateAction::GpuPower {
            plan: GpuPowerActionPlan {
                name: format!("gpu-power-{}-high", card.name),
                action: GpuPowerAction {
                    sysfs_root: std::path::PathBuf::from("/sys"),
                    drm_card: card.name.clone(),
                    power_dpm_force_performance_level: Some("high".to_owned()),
                    pp_power_profile_mode: None,
                },
                evidence: vec![CandidateEvidence::new(
                    "inventory",
                    format!(
                        "drm_card={} render_node={:?} current_dpm={:?} current_profile={:?}",
                        structured_evidence.drm_card,
                        structured_evidence.render_node,
                        structured_evidence.current_dpm,
                        structured_evidence.current_profile
                    ),
                    0.7,
                )],
                objective: ObjectiveKind::GameFramePacing,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::GameFramePacing,
            rank_hint: 85,
        }]
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        autotune::{
            controller::ControllerRuntimeState, observation::AutotuneObservation,
            system_context::SystemContextSnapshot,
        },
        daemon::{ActionSource, DaemonCapabilities, DaemonMode, SystemHealthSnapshot},
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        system_inventory::{DrmDeviceInventory, SystemInventory},
    };

    #[test]
    fn gpu_power_provider_uses_system_context_inventory() {
        let provider = GpuPowerProvider;
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::GameGpuBound,
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::GameGpuBound;

        let policy = policy();
        let system_context = SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: Vec::new(),
                drm_devices: vec![DrmDeviceInventory {
                    name: "card77".to_owned(),
                    path: PathBuf::from("/fake/sys/class/drm/card77"),
                    render_node: Some("renderD777".to_owned()),
                    hwmon_paths: Vec::new(),
                }],
                irq_default_smp_affinity: None,
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "fake-gpu-inventory".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        };

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
        let CandidateAction::GpuPower { plan } = &proposals[0].candidate else {
            panic!("expected gpu power candidate");
        };
        assert_eq!(plan.name, "gpu-power-card77-high");
        assert_eq!(plan.action.drm_card, "card77");
    }

    fn policy() -> crate::daemon::DaemonPolicy {
        let config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }
}
