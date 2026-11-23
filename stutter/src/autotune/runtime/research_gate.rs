//! The runtime side of the research action gate: while a research campaign owns the action slot
//! the normal controller still plans, but a candidate proceeds only when the gate permits it (an
//! authorized game action on the frozen game process, one at a time), and after every event the
//! gate is told what the experiment manager shows so it can settle only with a proven restoration.

use std::sync::{Arc, Mutex};

use stutter_core::ids::{ExperimentId, Pid};

use super::AutotuneRuntime;
use crate::autotune::{
    planning::candidate::CandidateAction,
    research::{
        ResearchAction,
        action_gate::{ActionGate, Holding, process_identity},
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

    /// Install (or remove) the observer the funnel reads each attempt's telemetry receipt from.
    pub fn set_research_telemetry(
        &mut self,
        telemetry: Option<crate::autotune::research::telemetry_observer::SharedTelemetry>,
    ) {
        self.research_telemetry = telemetry;
    }

    /// Install (or remove) the observer the funnel reads each attempt's observed window from.
    pub fn set_research_resident(
        &mut self,
        resident: Option<crate::autotune::research::resident_observer::SharedResident>,
    ) {
        self.research_resident = resident;
    }

    /// Report an interval event the runtime just handled: the records it carried and the events its sources
    /// dropped. Reading only; the observer counts what concerns the attested game process.
    pub(super) fn observe_research_intervals(
        &self,
        records: &[crate::metrics::IntervalRecord],
        lost_events: u64,
    ) {
        if let Some(resident) = self.research_resident.as_ref()
            && let Ok(mut observer) = resident.lock()
        {
            observer.observe_intervals(records, lost_events);
        }
    }

    /// Report the planning decision the runtime just emitted (the planner's own account of what it offered).
    pub(super) fn observe_research_decision(&self, entry: &super::AutotuneDecisionStreamEntry) {
        if let Some(resident) = self.research_resident.as_ref()
            && let Ok(mut observer) = resident.lock()
        {
            observer.observe_decision(entry.target_root_pid, entry.planner.as_ref());
        }
    }

    /// Report a GPU sample event the runtime just handled.
    pub(super) fn observe_research_gpu_sample(&self, sample: &crate::recorder::GpuSample) {
        if let Some(resident) = self.research_resident.as_ref()
            && let Ok(mut observer) = resident.lock()
        {
            observer.observe_gpu(sample);
        }
    }

    /// Report a frame event the runtime just handled (held for the foreground frame-tail guard).
    pub(super) fn observe_research_frame(&self, frame: &crate::recorder::FrameEvent) {
        if let Some(resident) = self.research_resident.as_ref()
            && let Ok(mut observer) = resident.lock()
        {
            observer.observe_frame(frame);
        }
    }

    /// Report a CPU frequency event the runtime just handled (held for the power guard).
    pub(super) fn observe_research_cpu_freq(&self, event: &crate::recorder::CpuFreqRecord) {
        if let Some(resident) = self.research_resident.as_ref()
            && let Ok(mut observer) = resident.lock()
        {
            observer.observe_cpu_freq(event);
        }
    }

    /// Report what the runtime just saw: a frame event, and the frame source's own status. Reading
    /// only; nothing here changes what the controller does.
    pub(super) fn observe_research_telemetry(&mut self, frame_event: bool) {
        let Some(telemetry) = self.research_telemetry.as_ref() else {
            return;
        };
        let snapshot = self.frame_source.as_ref().and_then(|source| {
            source.lock().ok().map(|status| {
                crate::autotune::research::telemetry_observer::SourceSnapshot::of(
                    &status,
                    std::time::Instant::now(),
                )
            })
        });
        if let Ok(mut observer) = telemetry.lock() {
            if frame_event {
                observer.count_frame();
            }
            if let Some(snapshot) = snapshot {
                observer.observe_source(snapshot);
            }
        }
    }

    /// Tell the gate what the experiment manager shows after an event.
    pub(super) fn observe_research_gate(&mut self) {
        let Some(gate) = self.research_action_gate.as_ref() else {
            return;
        };
        let active = self.live_experiments.has_active_experiment();
        let kept = self.controller.active_profile_state.kept_action_count();
        // Every action the manager holds, attributed by experiment, action and process.
        let held = |id: &ExperimentId, candidate: &CandidateAction| Holding {
            experiment_id: id.clone(),
            action: research_action_of(candidate),
            pid: candidate.target_root_pid().map(Pid::new),
        };
        let holdings: Vec<Holding> = self
            .live_experiments
            .current_experiment()
            .map(|current| held(&current.experiment_id, &current.candidate))
            .into_iter()
            .chain(
                self.controller
                    .active_profile_state
                    .kept_actions()
                    .map(|(_, kept)| held(&kept.experiment_id, &kept.candidate)),
            )
            .collect();
        if let Ok(mut gate) = gate.lock() {
            gate.observe_holdings(active, kept, holdings);
        }
    }
}
