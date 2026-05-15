use serde::{Deserialize, Serialize};

pub use super::situation::SituationKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControllerPhase {
    Disabled,
    Observing,
    Planning,
    Applying,
    Measuring,
    Keeping,
    Reverting,
    Cooldown,
    Faulted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutotuneMode {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
    ApplyHighRisk,
}
