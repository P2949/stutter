use crate::{
    actions::irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, IrqAffinityActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
    irq_inspect::{IrqLine, is_numeric_irq},
};

#[derive(Clone, Debug, PartialEq)]
pub struct IrqCandidateEvidence {
    pub irq: u32,
    pub device: String,
    pub current_mask: String,
    pub suggested_mask: String,
    pub overlap_score: f32,
    pub stable_identity: bool,
    pub known_device_mapping: bool,
}

#[derive(Default)]
pub struct IrqAffinityProvider;

impl CandidateProvider for IrqAffinityProvider {
    fn family(&self) -> &'static str {
        "irq_affinity"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !input.capabilities.irq_affinity_available
            || input.observation.primary_situation != SituationKind::IrqPressure
        {
            return Vec::new();
        }

        let Some(evidence_model) = observed_irq(input) else {
            return Vec::new();
        };

        let evidence = IrqAffinityEvidence {
            strong_irq_evidence: true,
            stable_irq_identity: evidence_model.stable_identity,
            known_device_mapping: evidence_model.known_device_mapping,
            observed_irq: Some(evidence_model.irq),
            observed_device_hint: Some(evidence_model.device.clone()),
            reason: format!(
                "IRQ pressure selected from structured telemetry; current_mask={} suggested_mask={} overlap_score={:.3}",
                evidence_model.current_mask,
                evidence_model.suggested_mask,
                evidence_model.overlap_score
            ),
        };
        let candidate = CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: format!("irq-{}-investigate-affinity", evidence_model.irq),
                action: IrqAffinityAction::new(
                    evidence_model.irq,
                    evidence_model.device.clone(),
                    evidence_model.suggested_mask.clone(),
                    IrqAffinityRisk::HighRisk,
                    evidence,
                ),
                evidence: vec![CandidateEvidence::new(
                    "irq_structured",
                    format!(
                        "irq={} device={} current_mask={} suggested_mask={} overlap_score={:.3} stable_identity={} known_device_mapping={}",
                        evidence_model.irq,
                        evidence_model.device,
                        evidence_model.current_mask,
                        evidence_model.suggested_mask,
                        evidence_model.overlap_score,
                        evidence_model.stable_identity,
                        evidence_model.known_device_mapping
                    ),
                    irq_confidence(input, &evidence_model),
                )],
                objective: ObjectiveKind::IrqOverlapReduction,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: irq_confidence(input, &evidence_model),
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::IrqOverlapReduction,
            rank_hint: 60,
        }]
    }
}

fn observed_irq(input: &CandidateProviderInput<'_>) -> Option<IrqCandidateEvidence> {
    let signals = &input.observation.objective_signals;
    if !signals.has_irq_signal() {
        return None;
    }

    let irq = signals.irq_hot_irq?;
    let current_mask = input
        .observation
        .active_config_snapshot
        .as_ref()
        .or(Some(&input.system_context.active_config))?
        .irq
        .per_irq
        .get(&irq)?
        .clone();

    let irq_line = input
        .system_context
        .inventory
        .irq_lines
        .iter()
        .find(|line| numeric_irq(line) == Some(irq))?;

    let suggested_cpu = least_busy_cpu(irq_line).or(signals.irq_hot_cpu)?;
    let suggested_mask = single_cpu_mask(suggested_cpu)?;
    if suggested_mask.trim() == current_mask.trim() {
        return None;
    }

    let stable_identity = !irq_line.name.trim().is_empty();
    let known_device_mapping = stable_identity && !irq_line.kind.trim().is_empty();
    if !stable_identity {
        return None;
    }

    Some(IrqCandidateEvidence {
        irq,
        device: irq_device_label(irq_line),
        current_mask,
        suggested_mask,
        overlap_score: overlap_score(signals.irq_worst_overlap_ns.unwrap_or(0)),
        stable_identity,
        known_device_mapping,
    })
}

fn numeric_irq(line: &IrqLine) -> Option<u32> {
    is_numeric_irq(&line.irq)
        .then(|| line.irq.parse::<u32>().ok())
        .flatten()
}

fn irq_device_label(line: &IrqLine) -> String {
    let name = line.name.trim();
    if name.is_empty() {
        format!("irq-{}", line.irq)
    } else {
        name.to_owned()
    }
}

fn least_busy_cpu(line: &IrqLine) -> Option<u32> {
    line.counts_by_cpu
        .iter()
        .enumerate()
        .min_by_key(|(_, count)| **count)
        .map(|(cpu, _)| cpu as u32)
}

fn single_cpu_mask(cpu: u32) -> Option<String> {
    1_u128.checked_shl(cpu).map(|mask| format!("{mask:x}"))
}

fn overlap_score(worst_overlap_ns: u64) -> f32 {
    ((worst_overlap_ns as f32) / 5_000_000.0).clamp(0.0, 1.0)
}

fn irq_confidence(input: &CandidateProviderInput<'_>, evidence: &IrqCandidateEvidence) -> f32 {
    let completeness = [
        true,
        evidence.stable_identity,
        evidence.known_device_mapping,
        !evidence.current_mask.is_empty(),
        !evidence.suggested_mask.is_empty(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 5.0;

    (input.observation.situation.confidence * completeness).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        autotune::{
            controller::ControllerRuntimeState,
            observation::{ActiveConfigSnapshot, ActiveIrqSnapshot, AutotuneObservation},
            system_context::SystemContextSnapshot,
        },
        daemon::{ActionSource, DaemonCapabilities, DaemonMode, SystemHealthSnapshot},
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        system_inventory::SystemInventory,
    };

    #[test]
    fn irq_provider_selects_hot_irq_from_structured_telemetry() {
        let provider = IrqAffinityProvider;
        let mut observation = observation();
        observation.objective_signals.irq_overlap_count = Some(2);
        observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
        observation.objective_signals.irq_hot_irq = Some(146);
        observation.objective_signals.irq_hot_cpu = Some(2);
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "4".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });

        let mut system_context = system_context();
        system_context.inventory.irq_default_smp_affinity = Some("f".to_owned());
        system_context.inventory.irq_lines = vec![IrqLine {
            irq: "146".to_owned(),
            counts_by_cpu: vec![10_000, 2, 20_000, 30_000],
            total: 60_002,
            kind: "PCI-MSI".to_owned(),
            name: "amdgpu".to_owned(),
            raw: "146: 10000 2 20000 30000 PCI-MSI amdgpu".to_owned(),
        }];

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        let CandidateAction::IrqAffinity { plan } = &proposals[0].candidate else {
            panic!("expected irq affinity candidate");
        };
        assert_eq!(plan.action.irq, 146);
        assert_eq!(plan.action.device_hint, "amdgpu");
        assert_eq!(plan.action.smp_affinity, "2");
        assert!(proposals[0].confidence > 0.0);
    }

    #[test]
    fn irq_provider_emits_no_proposal_when_structured_irq_evidence_is_missing() {
        let provider = IrqAffinityProvider;
        let observation = observation();
        let system_context = system_context();

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    #[test]
    fn irq_provider_emits_no_proposal_when_suggested_cpu_mask_is_unrepresentable() {
        let provider = IrqAffinityProvider;
        let mut observation = observation();
        observation.objective_signals.irq_overlap_count = Some(1);
        observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
        observation.objective_signals.irq_hot_irq = Some(146);
        observation.objective_signals.irq_hot_cpu = Some(130);
        observation.active_config_snapshot = Some(ActiveConfigSnapshot {
            irq: ActiveIrqSnapshot {
                per_irq: BTreeMap::from([(146, "4".to_owned())]),
            },
            ..ActiveConfigSnapshot::default()
        });

        let mut system_context = system_context();
        system_context.inventory.irq_lines = vec![IrqLine {
            irq: "146".to_owned(),
            counts_by_cpu: Vec::new(),
            total: 60_002,
            kind: "PCI-MSI".to_owned(),
            name: "amdgpu".to_owned(),
            raw: "146: PCI-MSI amdgpu".to_owned(),
        }];

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy(),
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    fn observation() -> AutotuneObservation {
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::IrqPressure,
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence: 0.95,
            capabilities: DaemonCapabilities {
                irq_affinity_available: true,
                ..DaemonCapabilities::default()
            },
            system_health: SystemHealthSnapshot {
                ok_for_apply: true,
                ..SystemHealthSnapshot::default()
            },
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::IrqPressure;
        observation
    }

    fn system_context() -> SystemContextSnapshot {
        SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: Vec::new(),
                drm_devices: Vec::new(),
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "irq-test".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        }
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
