use std::path::PathBuf;

use crate::{
    actions::vm_knobs::{VmKnobAction, VmKnobChange},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, VmKnobActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub struct VmKnobCandidateEvidence {
    pub knob: String,
    pub current_value: String,
    pub proposed_value: String,
    pub memory_pressure: Option<f32>,
    pub swap_activity: Option<u64>,
    pub dirty_writeback_pressure: Option<u64>,
}

#[derive(Default)]
pub struct VmKnobProvider;

impl CandidateProvider for VmKnobProvider {
    fn family(&self) -> &'static str {
        "vm_knob"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !matches!(
            input.observation.primary_situation,
            SituationKind::IoPressure | SituationKind::BrowserIoPressure
        ) {
            return Vec::new();
        }

        let Some(evidence_model) = vm_knob_evidence(input) else {
            return Vec::new();
        };

        let confidence = vm_knob_confidence(input, &evidence_model);

        let candidate = CandidateAction::VmKnob {
            plan: VmKnobActionPlan {
                name: "vm-swappiness-investigate-10".to_owned(),
                action: VmKnobAction {
                    root: PathBuf::from("/"),
                    changes: vec![VmKnobChange {
                        path: PathBuf::from("proc/sys/vm/swappiness"),
                        value: "10".to_owned(),
                    }],
                },
                evidence: vec![CandidateEvidence::new(
                    "vm_knob",
                    format!(
                        "knob={} current={} proposed={} memory_pressure={:?} swap_activity={:?} dirty_writeback_pressure={:?}",
                        evidence_model.knob,
                        evidence_model.current_value,
                        evidence_model.proposed_value,
                        evidence_model.memory_pressure,
                        evidence_model.swap_activity,
                        evidence_model.dirty_writeback_pressure
                    ),
                    confidence,
                )],
                objective: ObjectiveKind::IoLatency,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence,
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::IoLatency,
            rank_hint: 90,
        }]
    }
}

fn vm_knob_evidence(input: &CandidateProviderInput<'_>) -> Option<VmKnobCandidateEvidence> {
    let current_value = input
        .system_context
        .inventory
        .vm_knobs
        .get("proc/sys/vm/swappiness")
        .or_else(|| {
            input
                .system_context
                .inventory
                .vm_knobs
                .get("sys/vm/swappiness")
        })?
        .clone();

    let signals = &input.observation.objective_signals;
    let memory_pressure = signals.memory_pressure_some_avg10_percent;
    let swap_activity = signals.swap_activity_events;
    let dirty_writeback_pressure = signals.dirty_writeback_events;

    let has_memory_pressure = memory_pressure.is_some_and(|value| value > 0.0);
    let has_swap_activity = swap_activity.is_some_and(|value| value > 0);
    let has_dirty_writeback = dirty_writeback_pressure.is_some_and(|value| value > 0);

    if !has_memory_pressure && !has_swap_activity && !has_dirty_writeback {
        return None;
    }

    if current_value == "10" {
        return None;
    }

    Some(VmKnobCandidateEvidence {
        knob: "vm.swappiness".to_owned(),
        current_value,
        proposed_value: "10".to_owned(),
        memory_pressure,
        swap_activity,
        dirty_writeback_pressure,
    })
}

fn vm_knob_confidence(
    input: &CandidateProviderInput<'_>,
    evidence: &VmKnobCandidateEvidence,
) -> f32 {
    let completeness = [
        true,
        evidence.memory_pressure.is_some(),
        evidence.swap_activity.is_some(),
        evidence.dirty_writeback_pressure.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 4.0;

    (input.observation.situation.confidence * completeness).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        autotune::{
            controller::ControllerRuntimeState, observation::AutotuneObservation,
            system_context::SystemContextSnapshot,
        },
        daemon::{ActionSource, DaemonCapabilities, DaemonMode, SystemHealthSnapshot},
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        system_inventory::SystemInventory,
    };

    #[test]
    fn vm_knob_provider_requires_memory_swap_or_writeback_evidence() {
        let provider = VmKnobProvider;
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
    fn vm_knob_provider_emits_candidate_with_dirty_writeback_evidence() {
        let provider = VmKnobProvider;
        let mut observation = observation();
        observation.objective_signals.dirty_writeback_events = Some(5);
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

        assert_eq!(proposals.len(), 1);
        assert!(proposals[0].confidence > 0.0);
        let CandidateAction::VmKnob { plan } = &proposals[0].candidate else {
            panic!("expected vm knob candidate");
        };
        assert_eq!(plan.name, "vm-swappiness-investigate-10");
    }

    fn observation() -> AutotuneObservation {
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::IoPressure,
            focus_kind: Some(FocusGroupKind::Desktop),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::IoPressure;
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
                vm_knobs: BTreeMap::from([("sys/vm/swappiness".to_owned(), "60".to_owned())]),
                inventory_hash: "vm-test".to_owned(),
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
