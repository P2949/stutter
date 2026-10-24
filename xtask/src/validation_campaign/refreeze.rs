//! A defect found by exact-RC acceptance needs a new release candidate, not a new campaign:
//! collection evidence is bound to the evidence base and stays current. The failed candidate's
//! attempts remain as history; `freeze-rc` and everything downstream run again at the fixed `main`.
use super::{
    executor::RESTART_KEY,
    graph::{Graph, SourceBinding},
    model::{
        AttemptOutcome, CampaignDecision, CampaignMode, CampaignState, FailureClass, TaskStatus,
    },
    state::unix_ms,
};

const FREEZE: &str = "freeze-rc";
const DECISION: &str = "new release candidate required";
/// Candidates a campaign may burn before the failure is a real blocker.
pub const MAX_REFREEZES: usize = 5;

impl CampaignState {
    /// Returns the reason when a new release candidate was prepared.
    pub fn refreeze_after_candidate_defect(&mut self, graph: &Graph) -> Option<String> {
        if self.mode != CampaignMode::ReleaseCandidate {
            return None;
        }
        let defect = graph
            .specs()
            .filter(|spec| spec.binding == SourceBinding::ReleaseCandidate)
            .find_map(|spec| {
                let task = self.tasks.get(spec.id)?;
                match task.attempts.last().map(|a| &a.outcome) {
                    Some(AttemptOutcome::Failed {
                        class: FailureClass::SoftwareDefect,
                        summary,
                        ..
                    }) if task.status == TaskStatus::Failed => {
                        Some(format!("{}: {summary}", spec.id))
                    }
                    _ => None,
                }
            })?;
        let collection_intact = graph
            .specs()
            .filter(|spec| spec.binding == SourceBinding::EvidenceBase)
            .all(|spec| self.effective_status(spec).satisfies_dependents());
        let used = self
            .decisions
            .iter()
            .filter(|d| d.task == FREEZE && d.decision.starts_with(DECISION))
            .count();
        if !collection_intact || used >= MAX_REFREEZES {
            return None;
        }
        for id in std::iter::once(FREEZE).chain(graph.dependents(FREEZE)) {
            if let Some(task) = self.tasks.get_mut(id)
                && task.status != TaskStatus::SkippedByPolicy
            {
                task.status = TaskStatus::Invalidated;
            }
        }
        self.release_candidate = None;
        self.rc_stutter_sha256 = None;
        self.rc_xtask_sha256 = None;
        self.frozen.remove(RESTART_KEY);
        self.mode = CampaignMode::Collector;
        self.decisions.push(CampaignDecision {
            task: FREEZE.to_owned(),
            decision: format!("{DECISION}: {defect}"),
            limitations: Vec::new(),
            unix_ms: unix_ms(),
        });
        Some(defect)
    }
}
