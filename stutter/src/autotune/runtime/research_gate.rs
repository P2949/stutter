//! The runtime side of the research action gate: while a research campaign owns the action slot
//! the normal controller still plans, but a candidate proceeds only when the gate permits it (an
//! authorized game action on the frozen game process, one at a time), and after every event the
//! gate is told what the experiment manager shows so it can settle only with a proven restoration.

use std::sync::{Arc, Mutex};

use super::AutotuneRuntime;
use crate::autotune::{
    planning::candidate::CandidateAction,
    research::{
        ResearchAction,
        action_gate::{ActionGate, process_identity},
    },
};

pub(crate) type SharedGate = Arc<Mutex<ActionGate>>;

fn research_action_of(candidate: &CandidateAction) -> Option<ResearchAction> {
    match candidate {
        CandidateAction::CpuAffinityProfile { .. } => Some(ResearchAction::CpuAffinityProfile),
        CandidateAction::CgroupPlacement { .. } => Some(ResearchAction::CgroupPlacement),
        _ => None,
    }
}

impl AutotuneRuntime {
    /// Install (or remove) the gate an action child arms. Without one, research suppression is the
    /// blanket one it always was.
    pub fn set_research_action_gate(&mut self, gate: Option<SharedGate>) {
        self.research_action_gate = gate;
    }

    /// Whether the research gate permits this candidate now. Any candidate that is not an
    /// authorized game action on an identifiable process is refused.
    pub(super) fn research_gate_permits(&mut self, candidate: &CandidateAction) -> bool {
        let Some(gate) = self.research_action_gate.as_ref() else {
            return false;
        };
        let Some(target) = candidate.target_root_pid().and_then(process_identity) else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        gate.lock()
            .map(|mut gate| gate.permit(research_action_of(candidate), target, now))
            .unwrap_or(false)
    }

    /// Tell the gate what the experiment manager shows after an event.
    pub(super) fn observe_research_gate(&mut self) {
        let Some(gate) = self.research_action_gate.as_ref() else {
            return;
        };
        let active = self.live_experiments.has_active_experiment();
        let kept = self.controller.active_profile_state.kept_action_count();
        if let Ok(mut gate) = gate.lock() {
            gate.observe_runtime(active, kept);
        }
    }
}
