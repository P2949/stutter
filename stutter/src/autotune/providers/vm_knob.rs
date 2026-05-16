use std::path::PathBuf;

use crate::{
    actions::{
        SafetyClass,
        vm_knobs::{VmKnobAction, VmKnobChange},
    },
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, VmKnobActionPlan},
        objective::ObjectiveKind,
        providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput,
            signal_quality_confidence_weight,
        },
        situation::SituationKind,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub struct VmKnobCandidateEvidence {
    pub knob: String,
    pub path: String,
    pub current_value: String,
    pub proposed_value: String,
    pub rollback_value: String,
    pub trigger: VmKnobTrigger,
    pub trigger_evidence: String,
    pub memory_pressure: Option<f32>,
    pub swap_activity: Option<u64>,
    pub dirty_writeback_pressure: Option<u64>,
    pub objective: ObjectiveKind,
    pub safety_class: SafetyClass,
    pub manual_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmKnobPolicy {
    pub knob: &'static str,
    pub path: &'static str,
    pub safe_values: Vec<String>,
    pub trigger: VmKnobTrigger,
    pub objective: ObjectiveKind,
    pub safety_class: SafetyClass,
    pub manual_only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmKnobTrigger {
    SwapPressure,
    DirtyBackgroundWriteback,
    DirtyWritebackPressure,
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

        vm_knob_evidence(input)
            .into_iter()
            .map(|evidence_model| {
                let confidence = vm_knob_confidence(input, &evidence_model);
                let objective = evidence_model.objective();
                let candidate = CandidateAction::VmKnob {
                    plan: VmKnobActionPlan {
                        name: vm_knob_candidate_name(&evidence_model),
                        action: VmKnobAction {
                            root: PathBuf::from("/"),
                            changes: vec![VmKnobChange {
                                path: PathBuf::from(&evidence_model.path),
                                value: evidence_model.proposed_value.clone(),
                            }],
                        },
                        evidence: vec![CandidateEvidence::new(
                            "vm_knob",
                            format!(
                                "knob={} path={} current={} proposed={} rollback={} trigger={:?} trigger_evidence={} memory_pressure={:?} swap_activity={:?} dirty_writeback_pressure={:?} safety={:?} manual_only={}",
                                evidence_model.knob,
                                evidence_model.path,
                                evidence_model.current_value,
                                evidence_model.proposed_value,
                                evidence_model.rollback_value,
                                evidence_model.trigger,
                                evidence_model.trigger_evidence,
                                evidence_model.memory_pressure,
                                evidence_model.swap_activity,
                                evidence_model.dirty_writeback_pressure,
                                evidence_model.safety_class,
                                evidence_model.manual_only,
                            ),
                            confidence,
                        )],
                        objective,
                    },
                };

                CandidateProposal {
                    candidate,
                    provider: self.family(),
                    confidence,
                    deny_reasons: Vec::new(),
                    objective,
                    rank_hint: vm_knob_rank_hint(&evidence_model),
                }
            })
            .collect()
    }
}

fn vm_knob_evidence(input: &CandidateProviderInput<'_>) -> Vec<VmKnobCandidateEvidence> {
    vm_knob_policies()
        .into_iter()
        .filter_map(|policy| vm_knob_evidence_for_policy(input, &policy))
        .collect()
}

fn vm_knob_evidence_for_policy(
    input: &CandidateProviderInput<'_>,
    policy: &VmKnobPolicy,
) -> Option<VmKnobCandidateEvidence> {
    if policy.manual_only && policy.safety_class != SafetyClass::HighRisk {
        return None;
    }

    if policy_conflicts_with_active_bytes_knob(input, policy) {
        return None;
    }

    let current_value = current_vm_knob(input, policy.path)?.to_owned();
    let proposed_value = policy
        .safe_values
        .iter()
        .find(|value| value.as_str() != current_value.as_str())?
        .clone();

    let signals = &input.observation.objective_signals;
    let memory_pressure = signals.memory_pressure_some_avg10_percent;
    let swap_activity = signals.swap_activity_events;
    let dirty_writeback_pressure = signals.dirty_writeback_events;

    let trigger_evidence = trigger_evidence(policy.trigger, signals)?;

    Some(VmKnobCandidateEvidence {
        knob: policy.knob.to_owned(),
        path: policy.path.to_owned(),
        rollback_value: current_value.clone(),
        current_value,
        proposed_value,
        trigger: policy.trigger,
        trigger_evidence,
        memory_pressure,
        swap_activity,
        dirty_writeback_pressure,
        objective: policy.objective,
        safety_class: policy.safety_class.clone(),
        manual_only: policy.manual_only,
    })
}

fn vm_knob_policies() -> Vec<VmKnobPolicy> {
    vec![
        VmKnobPolicy {
            knob: "vm.swappiness",
            path: "proc/sys/vm/swappiness",
            safe_values: vec!["10".to_owned()],
            trigger: VmKnobTrigger::SwapPressure,
            objective: ObjectiveKind::IoLatency,
            safety_class: SafetyClass::HighRisk,
            manual_only: true,
        },
        VmKnobPolicy {
            knob: "vm.dirty_background_ratio",
            path: "proc/sys/vm/dirty_background_ratio",
            safe_values: vec!["5".to_owned()],
            trigger: VmKnobTrigger::DirtyBackgroundWriteback,
            objective: ObjectiveKind::IoLatency,
            safety_class: SafetyClass::HighRisk,
            manual_only: true,
        },
        VmKnobPolicy {
            knob: "vm.dirty_ratio",
            path: "proc/sys/vm/dirty_ratio",
            safe_values: vec!["10".to_owned()],
            trigger: VmKnobTrigger::DirtyWritebackPressure,
            objective: ObjectiveKind::IoLatency,
            safety_class: SafetyClass::HighRisk,
            manual_only: true,
        },
    ]
}

fn current_vm_knob<'a>(
    input: &'a CandidateProviderInput<'_>,
    proc_path: &str,
) -> Option<&'a String> {
    let sys_path = proc_path.strip_prefix("proc/")?;
    input
        .system_context
        .inventory
        .vm_knobs
        .get(proc_path)
        .or_else(|| input.system_context.inventory.vm_knobs.get(sys_path))
}

fn trigger_evidence(
    trigger: VmKnobTrigger,
    signals: &crate::autotune::objective::ObjectiveSignals,
) -> Option<String> {
    match trigger {
        VmKnobTrigger::SwapPressure => signals
            .swap_activity_events
            .filter(|value| *value > 0)
            .map(|value| format!("swap_activity_events={value}")),
        VmKnobTrigger::DirtyBackgroundWriteback => signals
            .dirty_writeback_events
            .filter(|value| *value > 0)
            .map(|value| format!("dirty_writeback_events={value}")),
        VmKnobTrigger::DirtyWritebackPressure => {
            let dirty = signals.dirty_writeback_events.filter(|value| *value > 0)?;
            let memory_pressure = signals
                .memory_pressure_some_avg10_percent
                .filter(|value| *value > 0.0)?;
            Some(format!(
                "dirty_writeback_events={dirty} memory_pressure_some_avg10_percent={memory_pressure:.2}"
            ))
        }
    }
}

fn policy_conflicts_with_active_bytes_knob(
    input: &CandidateProviderInput<'_>,
    policy: &VmKnobPolicy,
) -> bool {
    let bytes_path = match policy.knob {
        "vm.dirty_background_ratio" => Some("proc/sys/vm/dirty_background_bytes"),
        "vm.dirty_ratio" => Some("proc/sys/vm/dirty_bytes"),
        _ => None,
    };
    let Some(bytes_path) = bytes_path else {
        return false;
    };

    current_vm_knob(input, bytes_path)
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|value| value > 0)
}

fn vm_knob_candidate_name(evidence: &VmKnobCandidateEvidence) -> String {
    format!(
        "vm-{}-investigate-{}",
        evidence.knob.trim_start_matches("vm.").replace('_', "-"),
        evidence.proposed_value
    )
}

fn vm_knob_rank_hint(evidence: &VmKnobCandidateEvidence) -> u32 {
    match evidence.trigger {
        VmKnobTrigger::SwapPressure => 90,
        VmKnobTrigger::DirtyBackgroundWriteback => 88,
        VmKnobTrigger::DirtyWritebackPressure => 86,
    }
}

fn vm_knob_confidence(
    input: &CandidateProviderInput<'_>,
    evidence: &VmKnobCandidateEvidence,
) -> f32 {
    let completeness = [
        true,
        !evidence.current_value.is_empty(),
        !evidence.proposed_value.is_empty(),
        !evidence.rollback_value.is_empty(),
        !evidence.trigger_evidence.is_empty(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 5.0;

    let signal_quality = &input.observation.objective_signals.signal_quality;
    let quality_weights = [
        evidence
            .memory_pressure
            .map(|_| signal_quality_confidence_weight(signal_quality.memory_pressure)),
        evidence
            .swap_activity
            .map(|_| signal_quality_confidence_weight(signal_quality.swap_activity)),
        evidence
            .dirty_writeback_pressure
            .map(|_| signal_quality_confidence_weight(signal_quality.dirty_writeback)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let signal_weight = if quality_weights.is_empty() {
        1.0
    } else {
        quality_weights.iter().sum::<f32>() / quality_weights.len() as f32
    };

    (input.observation.situation.confidence * completeness * signal_weight).clamp(0.0, 1.0)
}

impl VmKnobCandidateEvidence {
    fn objective(&self) -> ObjectiveKind {
        self.objective
    }
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
        assert_eq!(plan.name, "vm-dirty-background-ratio-investigate-5");
        assert_eq!(
            plan.action.changes[0].path,
            PathBuf::from("proc/sys/vm/dirty_background_ratio")
        );
        assert!(plan.evidence[0].value.contains("rollback=10"));
        assert!(plan.evidence[0].value.contains("manual_only=true"));
        assert_eq!(proposals[0].candidate.safety_class(), SafetyClass::HighRisk);
        assert!(proposals[0].candidate.manual_only_reason().is_some());
    }

    #[test]
    fn vm_knob_provider_emits_swappiness_candidate_for_swap_pressure() {
        let provider = VmKnobProvider;
        let mut observation = observation();
        observation.objective_signals.swap_activity_events = Some(12);
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
        let CandidateAction::VmKnob { plan } = &proposals[0].candidate else {
            panic!("expected vm knob candidate");
        };
        assert_eq!(plan.name, "vm-swappiness-investigate-10");
        assert_eq!(
            plan.action.changes[0].path,
            PathBuf::from("proc/sys/vm/swappiness")
        );
        assert_eq!(plan.action.changes[0].value, "10");
        assert!(plan.evidence[0].value.contains("trigger=SwapPressure"));
        assert!(plan.evidence[0].value.contains("current=60"));
        assert!(plan.evidence[0].value.contains("rollback=60"));
    }

    #[test]
    fn vm_knob_provider_emits_dirty_ratio_when_writeback_and_memory_pressure_overlap() {
        let provider = VmKnobProvider;
        let mut observation = observation();
        observation.objective_signals.dirty_writeback_events = Some(7);
        observation
            .objective_signals
            .memory_pressure_some_avg10_percent = Some(11.5);
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

        let names = proposals
            .iter()
            .map(|proposal| match &proposal.candidate {
                CandidateAction::VmKnob { plan } => plan.name.as_str(),
                _ => "",
            })
            .collect::<Vec<_>>();

        assert!(names.contains(&"vm-dirty-background-ratio-investigate-5"));
        assert!(names.contains(&"vm-dirty-ratio-investigate-10"));
    }

    #[test]
    fn vm_knob_provider_rejects_already_target_value() {
        let provider = VmKnobProvider;
        let mut observation = observation();
        observation.objective_signals.swap_activity_events = Some(12);
        let system_context = system_context_with_knobs(BTreeMap::from([(
            "sys/vm/swappiness".to_owned(),
            "10".to_owned(),
        )]));

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
    fn vm_knob_provider_skips_ratio_knobs_when_bytes_knobs_are_active() {
        let provider = VmKnobProvider;
        let mut observation = observation();
        observation.objective_signals.dirty_writeback_events = Some(5);
        observation
            .objective_signals
            .memory_pressure_some_avg10_percent = Some(9.0);
        let system_context = system_context_with_knobs(BTreeMap::from([
            ("sys/vm/dirty_background_ratio".to_owned(), "10".to_owned()),
            (
                "sys/vm/dirty_background_bytes".to_owned(),
                "1048576".to_owned(),
            ),
            ("sys/vm/dirty_ratio".to_owned(), "20".to_owned()),
            ("sys/vm/dirty_bytes".to_owned(), "2097152".to_owned()),
        ]));

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
        system_context_with_knobs(BTreeMap::from([
            ("sys/vm/swappiness".to_owned(), "60".to_owned()),
            ("sys/vm/dirty_background_ratio".to_owned(), "10".to_owned()),
            ("sys/vm/dirty_ratio".to_owned(), "20".to_owned()),
        ]))
    }

    fn system_context_with_knobs(vm_knobs: BTreeMap<String, String>) -> SystemContextSnapshot {
        SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: Vec::new(),
                drm_devices: Vec::new(),
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                power_source: Default::default(),
                sched_ext_available: false,
                vm_knobs,
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
