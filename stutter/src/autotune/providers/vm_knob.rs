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
                    input.observation.situation.confidence,
                )],
                objective: ObjectiveKind::IoLatency,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
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

    // Do not emit a VM knob recommendation from generic I/O pressure alone. The
    // current observation model does not yet carry swap/writeback counters, so
    // this provider stays silent until that structured evidence is available.
    let memory_pressure: Option<f32> = None;
    let swap_activity: Option<u64> = None;
    let dirty_writeback_pressure: Option<u64> = None;
    if memory_pressure.is_none() && swap_activity.is_none() && dirty_writeback_pressure.is_none() {
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
