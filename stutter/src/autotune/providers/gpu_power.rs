use crate::{
    actions::gpu_power::GpuPowerAction,
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, GpuPowerActionPlan},
        objective::ObjectiveKind,
        providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput,
            signal_quality_confidence_weight,
        },
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

const MIN_MULTI_GPU_FOCUS_CONFIDENCE: f32 = 0.70;

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

        let Some(structured_evidence) = gpu_power_evidence(input) else {
            return Vec::new();
        };

        let confidence = gpu_power_confidence(input, &structured_evidence);
        let candidate = CandidateAction::GpuPower {
            plan: GpuPowerActionPlan {
                name: format!("gpu-power-{}-profile", structured_evidence.drm_card),
                action: GpuPowerAction {
                    sysfs_root: std::path::PathBuf::from("/sys"),
                    drm_card: structured_evidence.drm_card.clone(),
                    power_dpm_force_performance_level: None,
                    pp_power_profile_mode: Some("3D_FULL_SCREEN".to_owned()),
                },
                evidence: vec![CandidateEvidence::new(
                    "gpu_power_structured",
                    format!(
                        "drm_card={} render_node={:?} pci_id={:?} vendor={:?} current_dpm={:?} current_profile={:?} active_for_focus={} busy={:?} clock_mhz={:?}",
                        structured_evidence.drm_card,
                        structured_evidence.render_node,
                        structured_evidence.pci_id,
                        structured_evidence.vendor,
                        structured_evidence.current_dpm,
                        structured_evidence.current_profile,
                        structured_evidence.active_for_focus,
                        input.observation.objective_signals.gpu_busy_percent,
                        input.observation.objective_signals.gpu_clock_mhz
                    ),
                    confidence,
                )],
                objective: ObjectiveKind::GameFramePacing,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence,
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::GameFramePacing,
            rank_hint: 85,
        }]
    }
}

fn gpu_power_evidence(input: &CandidateProviderInput<'_>) -> Option<GpuPowerCandidateEvidence> {
    let signals = &input.observation.objective_signals;
    if signals.gpu_power_limited != Some(true) && signals.gpu_busy_percent.unwrap_or(0) < 90 {
        return None;
    }

    let card = selected_gpu(input)?;
    let runtime_state = input
        .observation
        .active_config_snapshot
        .as_ref()
        .or(Some(&input.system_context.active_config))
        .and_then(|snapshot| {
            snapshot
                .gpu_power
                .devices
                .iter()
                .find(|device| device.device == card.name)
        });

    if runtime_state.and_then(|state| state.pp_power_profile_mode.as_deref())
        == Some("3D_FULL_SCREEN")
    {
        return None;
    }

    Some(GpuPowerCandidateEvidence {
        drm_card: card.name.clone(),
        render_node: card.render_node.clone(),
        pci_id: card.pci_id.clone(),
        vendor: card.vendor.clone(),
        current_dpm: runtime_state
            .and_then(|state| state.power_dpm_force_performance_level.clone()),
        current_profile: runtime_state.and_then(|state| state.pp_power_profile_mode.clone()),
        active_for_focus: true,
    })
}

fn selected_gpu<'a>(
    input: &'a CandidateProviderInput<'_>,
) -> Option<&'a crate::system_inventory::DrmDeviceInventory> {
    if let Some(render_node) = input
        .observation
        .objective_signals
        .gpu_active_render_node
        .as_deref()
    {
        if input.system_context.inventory.drm_devices.len() > 1 {
            let confidence = input
                .observation
                .objective_signals
                .gpu_focus_confidence
                .unwrap_or(1.0);
            if confidence < MIN_MULTI_GPU_FOCUS_CONFIDENCE {
                return None;
            }
        }

        return input
            .system_context
            .inventory
            .drm_devices
            .iter()
            .find(|device| device.render_node.as_deref() == Some(render_node));
    }

    match input.system_context.inventory.drm_devices.as_slice() {
        [single] => Some(single),
        _ => None,
    }
}

fn gpu_power_confidence(
    input: &CandidateProviderInput<'_>,
    evidence: &GpuPowerCandidateEvidence,
) -> f32 {
    let completeness = [
        true,
        evidence.render_node.is_some(),
        evidence.pci_id.is_some(),
        evidence.vendor.is_some(),
        evidence.current_dpm.is_some(),
        input
            .observation
            .objective_signals
            .gpu_busy_percent
            .is_some(),
        input.observation.objective_signals.gpu_clock_mhz.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 7.0;

    let signal_weight = signal_quality_confidence_weight(
        input.observation.objective_signals.signal_quality.gpu_power,
    );

    (input.observation.situation.confidence * completeness * signal_weight).clamp(0.0, 1.0)
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
        daemon::{
            capabilities::DaemonCapabilities,
            health::SystemHealthSnapshot,
            policy::{ActionSource, DaemonMode},
        },
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        system_inventory::{DrmDeviceInventory, SystemInventory},
    };

    #[test]
    fn gpu_power_provider_rejects_ambiguous_multiple_gpus_without_focus_render_node() {
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
        observation.objective_signals.gpu_power_limited = Some(true);
        observation.objective_signals.gpu_busy_percent = Some(96);
        observation.objective_signals.gpu_clock_mhz = Some(250);

        let policy = policy();
        let system_context = SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: Vec::new(),
                drm_devices: vec![
                    DrmDeviceInventory {
                        name: "card0".to_owned(),
                        path: PathBuf::from("/fake/sys/class/drm/card0"),
                        render_node: Some("renderD128".to_owned()),
                        pci_id: Some("1002:1111".to_owned()),
                        vendor: Some("amd".to_owned()),
                        hwmon_paths: Vec::new(),
                    },
                    DrmDeviceInventory {
                        name: "card1".to_owned(),
                        path: PathBuf::from("/fake/sys/class/drm/card1"),
                        render_node: Some("renderD129".to_owned()),
                        pci_id: Some("8086:2222".to_owned()),
                        vendor: Some("intel".to_owned()),
                        hwmon_paths: Vec::new(),
                    },
                ],
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                power_source: Default::default(),
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

        assert!(proposals.is_empty());
    }

    #[test]
    fn gpu_power_provider_selects_gpu_by_focus_render_node_when_multiple_gpus_exist() {
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
        observation.objective_signals.gpu_power_limited = Some(true);
        observation.objective_signals.gpu_busy_percent = Some(96);
        observation.objective_signals.gpu_clock_mhz = Some(250);
        observation.objective_signals.gpu_active_render_node = Some("renderD129".to_owned());

        let policy = policy();
        let system_context = SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: Vec::new(),
                drm_devices: vec![
                    DrmDeviceInventory {
                        name: "card0".to_owned(),
                        path: PathBuf::from("/fake/sys/class/drm/card0"),
                        render_node: Some("renderD128".to_owned()),
                        pci_id: Some("1002:1111".to_owned()),
                        vendor: Some("amd".to_owned()),
                        hwmon_paths: Vec::new(),
                    },
                    DrmDeviceInventory {
                        name: "card1".to_owned(),
                        path: PathBuf::from("/fake/sys/class/drm/card1"),
                        render_node: Some("renderD129".to_owned()),
                        pci_id: Some("8086:2222".to_owned()),
                        vendor: Some("intel".to_owned()),
                        hwmon_paths: Vec::new(),
                    },
                ],
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                power_source: Default::default(),
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "fake-gpu-focused-inventory".to_owned(),
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
        assert_eq!(plan.name, "gpu-power-card1-profile");
        assert_eq!(plan.action.drm_card, "card1");
        assert_eq!(plan.action.power_dpm_force_performance_level, None);
        assert_eq!(
            plan.action.pp_power_profile_mode.as_deref(),
            Some("3D_FULL_SCREEN")
        );
        assert!(proposals[0].confidence > 0.0);
    }

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
                    pci_id: Some("1002:744c".to_owned()),
                    vendor: Some("amd".to_owned()),
                    hwmon_paths: Vec::new(),
                }],
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                power_source: Default::default(),
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "fake-gpu-inventory".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        };

        observation.objective_signals.gpu_power_limited = Some(true);
        observation.objective_signals.gpu_busy_percent = Some(96);
        observation.objective_signals.gpu_clock_mhz = Some(250);

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
        assert_eq!(plan.name, "gpu-power-card77-profile");
        assert_eq!(plan.action.drm_card, "card77");
        assert_eq!(plan.action.power_dpm_force_performance_level, None);
        assert_eq!(
            plan.action.pp_power_profile_mode.as_deref(),
            Some("3D_FULL_SCREEN")
        );
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
