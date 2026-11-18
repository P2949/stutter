//! Typed terminal classes for resident research attempts.

/// Failure classification is chosen by the observing host, never inferred from log text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Incompatible,
    Environment,
    Resource,
    Integrity,
    Health,
    OwnershipUnresolved,
}

impl FailureClass {
    pub fn terminal_code(self) -> &'static str {
        match self {
            Self::Incompatible => "Incompatible",
            Self::Environment => "EnvironmentInterrupted",
            Self::Resource => "EvidenceBudgetExceeded",
            Self::Integrity => "AbortedIntegrity",
            Self::Health => "BlockedGpuStability",
            Self::OwnershipUnresolved => "SettlementUnproven",
        }
    }
}
