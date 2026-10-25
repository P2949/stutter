//! The typed record of one objective comparison, kept with the experiment's history so a
//! campaign recomputes keep/revert from the recorded populations instead of parsing text.
use serde::{Deserialize, Serialize};

use super::{ObjectiveKind, ObjectiveSignals, decision::ObjectiveDecision};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectiveEvidence {
    pub objective: ObjectiveKind,
    pub baseline_signals: ObjectiveSignals,
    pub candidate_signals: ObjectiveSignals,
    pub decision: ObjectiveDecision,
}
