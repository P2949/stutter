use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::model::{
    ActionFailure, ActionTimeout, PhaseFailure, RollbackOutcome, ScopeLimitExceeded,
};
use crate::actions::model::ActionPhase;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ActionErrorWire {
    PreflightFailure {
        message: String,
    },
    DryRunFailure {
        message: String,
    },
    ApplyFailure {
        message: String,
    },
    VerifyFailure {
        message: String,
    },
    VerifyFailureRollbackCompleted {
        verify_error: String,
    },
    RollbackFailure {
        message: String,
    },
    EmergencyRollbackFailure {
        verify_error: String,
        rollback_error: String,
    },
    ScopeLimitExceeded {
        affected_tasks: usize,
        max_affected_tasks: usize,
    },
    Timeout {
        phase: ActionPhase,
        #[serde(
            serialize_with = "serialize_u128_as_u64",
            deserialize_with = "deserialize_u128_from_u64"
        )]
        elapsed_ms: u128,
        #[serde(
            serialize_with = "serialize_u128_as_u64",
            deserialize_with = "deserialize_u128_from_u64"
        )]
        timeout_ms: u128,
    },
    TimeoutRollbackCompleted {
        phase: ActionPhase,
        #[serde(
            serialize_with = "serialize_u128_as_u64",
            deserialize_with = "deserialize_u128_from_u64"
        )]
        elapsed_ms: u128,
        #[serde(
            serialize_with = "serialize_u128_as_u64",
            deserialize_with = "deserialize_u128_from_u64"
        )]
        timeout_ms: u128,
    },
    TimeoutRollbackFailure {
        phase: ActionPhase,
        #[serde(
            serialize_with = "serialize_u128_as_u64",
            deserialize_with = "deserialize_u128_from_u64"
        )]
        elapsed_ms: u128,
        #[serde(
            serialize_with = "serialize_u128_as_u64",
            deserialize_with = "deserialize_u128_from_u64"
        )]
        timeout_ms: u128,
        rollback_error: String,
    },
    InvalidRollbackToken {
        expected: String,
        actual: String,
    },
    PolicyRejected {
        message: String,
    },
}

pub fn serialize_u128_as_u64<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Ok(v) = u64::try_from(*value) {
        serializer.serialize_u64(v)
    } else {
        serializer.serialize_u128(*value)
    }
}

pub fn deserialize_u128_from_u64<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: Deserializer<'de>,
{
    struct U128Visitor;
    impl<'de> serde::de::Visitor<'de> for U128Visitor {
        type Value = u128;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a 128-bit unsigned integer")
        }
        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v as u128)
        }
        fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v)
        }
        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            v.parse().map_err(serde::de::Error::custom)
        }
    }
    deserializer.deserialize_any(U128Visitor)
}

impl From<ActionErrorWire> for ActionFailure {
    fn from(value: ActionErrorWire) -> Self {
        match value {
            ActionErrorWire::PreflightFailure { message } => {
                Self::Phase(PhaseFailure::Preflight { message })
            }
            ActionErrorWire::DryRunFailure { message } => {
                Self::Phase(PhaseFailure::DryRun { message })
            }
            ActionErrorWire::ApplyFailure { message } => {
                Self::Phase(PhaseFailure::Apply { message })
            }
            ActionErrorWire::VerifyFailure { message } => {
                Self::Phase(PhaseFailure::Verify { message })
            }
            ActionErrorWire::VerifyFailureRollbackCompleted { verify_error } => {
                Self::Rollback(RollbackOutcome::VerifyRollbackCompleted { verify_error })
            }
            ActionErrorWire::RollbackFailure { message } => {
                Self::Rollback(RollbackOutcome::RollbackFailure { message })
            }
            ActionErrorWire::EmergencyRollbackFailure {
                verify_error,
                rollback_error,
            } => Self::Rollback(RollbackOutcome::EmergencyRollbackFailure {
                verify_error,
                rollback_error,
            }),
            ActionErrorWire::ScopeLimitExceeded {
                affected_tasks,
                max_affected_tasks,
            } => Self::ScopeLimitExceeded(ScopeLimitExceeded {
                affected_tasks,
                max_affected_tasks,
            }),
            ActionErrorWire::Timeout {
                phase,
                elapsed_ms,
                timeout_ms,
            } => Self::Timeout(ActionTimeout {
                phase,
                elapsed_ms,
                timeout_ms,
            }),
            ActionErrorWire::TimeoutRollbackCompleted {
                phase,
                elapsed_ms,
                timeout_ms,
            } => Self::Rollback(RollbackOutcome::TimeoutRollbackCompleted {
                timeout: ActionTimeout {
                    phase,
                    elapsed_ms,
                    timeout_ms,
                },
            }),
            ActionErrorWire::TimeoutRollbackFailure {
                phase,
                elapsed_ms,
                timeout_ms,
                rollback_error,
            } => Self::Rollback(RollbackOutcome::TimeoutRollbackFailure {
                timeout: ActionTimeout {
                    phase,
                    elapsed_ms,
                    timeout_ms,
                },
                rollback_error,
            }),
            ActionErrorWire::InvalidRollbackToken { expected, actual } => {
                Self::InvalidRollbackToken { expected, actual }
            }
            ActionErrorWire::PolicyRejected { message } => Self::PolicyRejected { message },
        }
    }
}

impl From<ActionFailure> for ActionErrorWire {
    fn from(value: ActionFailure) -> Self {
        match value {
            ActionFailure::Phase(PhaseFailure::Preflight { message }) => {
                Self::PreflightFailure { message }
            }
            ActionFailure::Phase(PhaseFailure::DryRun { message }) => {
                Self::DryRunFailure { message }
            }
            ActionFailure::Phase(PhaseFailure::Apply { message }) => Self::ApplyFailure { message },
            ActionFailure::Phase(PhaseFailure::Verify { message }) => {
                Self::VerifyFailure { message }
            }
            ActionFailure::Phase(PhaseFailure::Rollback { message }) => {
                Self::RollbackFailure { message }
            }
            ActionFailure::Timeout(ActionTimeout {
                phase,
                elapsed_ms,
                timeout_ms,
            }) => Self::Timeout {
                phase,
                elapsed_ms,
                timeout_ms,
            },
            ActionFailure::Rollback(RollbackOutcome::RollbackFailure { message }) => {
                Self::RollbackFailure { message }
            }
            ActionFailure::Rollback(RollbackOutcome::VerifyRollbackCompleted { verify_error }) => {
                Self::VerifyFailureRollbackCompleted { verify_error }
            }
            ActionFailure::Rollback(RollbackOutcome::EmergencyRollbackFailure {
                verify_error,
                rollback_error,
            }) => Self::EmergencyRollbackFailure {
                verify_error,
                rollback_error,
            },
            ActionFailure::Rollback(RollbackOutcome::TimeoutRollbackCompleted {
                timeout:
                    ActionTimeout {
                        phase,
                        elapsed_ms,
                        timeout_ms,
                    },
            }) => Self::TimeoutRollbackCompleted {
                phase,
                elapsed_ms,
                timeout_ms,
            },
            ActionFailure::Rollback(RollbackOutcome::TimeoutRollbackFailure {
                timeout:
                    ActionTimeout {
                        phase,
                        elapsed_ms,
                        timeout_ms,
                    },
                rollback_error,
            }) => Self::TimeoutRollbackFailure {
                phase,
                elapsed_ms,
                timeout_ms,
                rollback_error,
            },
            ActionFailure::InvalidRollbackToken { expected, actual } => {
                Self::InvalidRollbackToken { expected, actual }
            }
            ActionFailure::ScopeLimitExceeded(ScopeLimitExceeded {
                affected_tasks,
                max_affected_tasks,
            }) => Self::ScopeLimitExceeded {
                affected_tasks,
                max_affected_tasks,
            },
            ActionFailure::PolicyRejected { message } => Self::PolicyRejected { message },
        }
    }
}

pub(super) fn serialize_action_failure<S>(
    failure: &ActionFailure,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let wire = ActionErrorWire::from(failure.clone());
    wire.serialize(serializer)
}

pub(super) fn deserialize_action_failure<'de, D>(deserializer: D) -> Result<ActionFailure, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(ActionErrorWire::deserialize(deserializer)?.into())
}
