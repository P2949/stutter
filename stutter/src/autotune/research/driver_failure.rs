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

/// A launch admission refusal whose class is supplied at the observing boundary.
#[derive(Debug, thiserror::Error)]
#[error("{code}")]
pub struct LaunchFailure {
    pub class: FailureClass,
    pub code: &'static str,
}

impl LaunchFailure {
    pub fn classify(error: &anyhow::Error, expired: bool) -> (FailureClass, &'static str) {
        error
            .downcast_ref::<Self>()
            .map(|failure| (failure.class, failure.code))
            .unwrap_or(if expired {
                (FailureClass::Resource, "AuthorizationExpiredBeforeLaunch")
            } else {
                (FailureClass::Environment, "LaunchRefused")
            })
    }
    pub fn require(condition: bool, class: FailureClass, code: &'static str) -> anyhow::Result<()> {
        if condition {
            Ok(())
        } else {
            Err(Self { class, code }.into())
        }
    }
}

impl FailureClass {
    pub fn terminal_code(self) -> &'static str {
        match self {
            Self::Incompatible => "Incompatible",
            Self::Environment => "EnvironmentInterrupted",
            Self::Resource => "ResourceExhausted",
            Self::Integrity => "AbortedIntegrity",
            Self::Health => "BlockedGpuStability",
            Self::OwnershipUnresolved => "SettlementUnproven",
        }
    }
}
