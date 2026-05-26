use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::actions::{model::ActionPhase, token::RollbackTokenKindError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionError {
    failure: ActionFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionFailure {
    Phase(PhaseFailure),
    Timeout(ActionTimeout),
    Rollback(RollbackOutcome),
    InvalidRollbackToken { expected: String, actual: String },
    ScopeLimitExceeded(ScopeLimitExceeded),
    PolicyRejected { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum PhaseFailure {
    Preflight { message: String },
    DryRun { message: String },
    Apply { message: String },
    Verify { message: String },
    Rollback { message: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionTimeout {
    pub phase: ActionPhase,
    #[serde(
        serialize_with = "serialize_u128_as_u64",
        deserialize_with = "deserialize_u128_from_u64"
    )]
    pub elapsed_ms: u128,
    #[serde(
        serialize_with = "serialize_u128_as_u64",
        deserialize_with = "deserialize_u128_from_u64"
    )]
    pub timeout_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RollbackOutcome {
    RollbackFailure {
        message: String,
    },
    VerifyRollbackCompleted {
        verify_error: String,
    },
    EmergencyRollbackFailure {
        verify_error: String,
        rollback_error: String,
    },
    TimeoutRollbackCompleted {
        timeout: ActionTimeout,
    },
    TimeoutRollbackFailure {
        timeout: ActionTimeout,
        rollback_error: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopeLimitExceeded {
    pub affected_tasks: usize,
    pub max_affected_tasks: usize,
}

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

fn serialize_u128_as_u64<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Ok(v) = u64::try_from(*value) {
        serializer.serialize_u64(v)
    } else {
        serializer.serialize_u128(*value)
    }
}

fn deserialize_u128_from_u64<'de, D>(deserializer: D) -> Result<u128, D::Error>
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

impl ActionError {
    pub fn from_failure(failure: ActionFailure) -> Self {
        Self { failure }
    }

    pub fn failure(&self) -> &ActionFailure {
        &self.failure
    }

    pub fn into_failure(self) -> ActionFailure {
        self.failure
    }

    pub fn preflight(error: impl std::fmt::Display) -> Self {
        Self::from_failure(ActionFailure::Phase(PhaseFailure::Preflight {
            message: error.to_string(),
        }))
    }

    pub fn dry_run(error: impl std::fmt::Display) -> Self {
        Self::from_failure(ActionFailure::Phase(PhaseFailure::DryRun {
            message: error.to_string(),
        }))
    }

    pub fn apply(error: impl std::fmt::Display) -> Self {
        Self::from_failure(ActionFailure::Phase(PhaseFailure::Apply {
            message: error.to_string(),
        }))
    }

    pub fn verify(error: impl std::fmt::Display) -> Self {
        Self::from_failure(ActionFailure::Phase(PhaseFailure::Verify {
            message: error.to_string(),
        }))
    }

    pub fn verify_rollback_completed(verify_error: impl std::fmt::Display) -> Self {
        Self::from_failure(ActionFailure::Rollback(
            RollbackOutcome::VerifyRollbackCompleted {
                verify_error: verify_error.to_string(),
            },
        ))
    }

    pub fn rollback(error: impl std::fmt::Display) -> Self {
        Self::from_failure(ActionFailure::Rollback(RollbackOutcome::RollbackFailure {
            message: error.to_string(),
        }))
    }

    pub fn invalid_rollback_token(expected: &'static str, actual: &'static str) -> Self {
        Self::from_failure(ActionFailure::InvalidRollbackToken {
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        })
    }

    pub fn invalid_rollback_token_kind(error: RollbackTokenKindError) -> Self {
        Self::invalid_rollback_token(error.expected(), error.actual())
    }

    pub fn from_rollback_error(error: anyhow::Error) -> Self {
        if let Some(action_error) = error.downcast_ref::<ActionError>() {
            return action_error.clone();
        }
        if let Some(kind_error) = error.downcast_ref::<RollbackTokenKindError>() {
            return Self::invalid_rollback_token_kind(*kind_error);
        }
        Self::rollback(error)
    }

    pub fn emergency_rollback(
        verify_error: impl std::fmt::Display,
        rollback_error: impl std::fmt::Display,
    ) -> Self {
        Self::from_failure(ActionFailure::Rollback(
            RollbackOutcome::EmergencyRollbackFailure {
                verify_error: verify_error.to_string(),
                rollback_error: rollback_error.to_string(),
            },
        ))
    }

    pub fn policy_rejected(error: impl std::fmt::Display) -> Self {
        Self::from_failure(ActionFailure::PolicyRejected {
            message: error.to_string(),
        })
    }

    pub fn scope_limit_exceeded(affected_tasks: usize, max_affected_tasks: usize) -> Self {
        Self::from_failure(ActionFailure::ScopeLimitExceeded(ScopeLimitExceeded {
            affected_tasks,
            max_affected_tasks,
        }))
    }

    pub fn timeout(phase: ActionPhase, elapsed_ms: u128, timeout_ms: u128) -> Self {
        Self::from_failure(ActionFailure::Timeout(ActionTimeout {
            phase,
            elapsed_ms,
            timeout_ms,
        }))
    }

    pub fn timeout_rollback_completed(
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
    ) -> Self {
        Self::from_failure(ActionFailure::Rollback(
            RollbackOutcome::TimeoutRollbackCompleted {
                timeout: ActionTimeout {
                    phase,
                    elapsed_ms,
                    timeout_ms,
                },
            },
        ))
    }

    pub fn timeout_rollback_failure(
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
        rollback_error: impl std::fmt::Display,
    ) -> Self {
        Self::from_failure(ActionFailure::Rollback(
            RollbackOutcome::TimeoutRollbackFailure {
                timeout: ActionTimeout {
                    phase,
                    elapsed_ms,
                    timeout_ms,
                },
                rollback_error: rollback_error.to_string(),
            },
        ))
    }

    pub fn timeout_details(&self) -> Option<ActionTimeout> {
        match &self.failure {
            ActionFailure::Timeout(timeout) => Some(*timeout),
            _ => None,
        }
    }

    pub fn phase(&self) -> ActionPhase {
        self.failure.phase()
    }

    pub fn category(&self) -> &'static str {
        self.failure.category()
    }

    pub fn human_message(&self) -> String {
        self.failure.human_message()
    }
}

impl ActionFailure {
    pub fn phase(&self) -> ActionPhase {
        match self {
            Self::Phase(failure) => failure.phase(),
            Self::Timeout(timeout) => timeout.phase,
            Self::Rollback(outcome) => outcome.phase(),
            Self::InvalidRollbackToken { .. } => ActionPhase::Rollback,
            Self::ScopeLimitExceeded(_) => ActionPhase::DryRun,
            Self::PolicyRejected { .. } => ActionPhase::Preflight,
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::Phase(failure) => failure.category(),
            Self::Timeout { .. } => "timeout",
            Self::Rollback(outcome) => outcome.category(),
            Self::InvalidRollbackToken { .. } => "invalid_rollback_token",
            Self::ScopeLimitExceeded(_) => "scope_limit_exceeded",
            Self::PolicyRejected { .. } => "policy_rejected",
        }
    }

    pub fn human_message(&self) -> String {
        match self {
            Self::Phase(failure) => failure.human_message(),
            Self::Timeout(timeout) => timeout.human_message(),
            Self::Rollback(outcome) => outcome.human_message(),
            Self::InvalidRollbackToken { expected, actual } => {
                format!("invalid rollback token: expected {expected}, actual {actual}")
            }
            Self::ScopeLimitExceeded(scope) => scope.human_message(),
            Self::PolicyRejected { message } => format!("policy rejected: {message}"),
        }
    }
}

impl PhaseFailure {
    fn phase(&self) -> ActionPhase {
        match self {
            Self::Preflight { .. } => ActionPhase::Preflight,
            Self::DryRun { .. } => ActionPhase::DryRun,
            Self::Apply { .. } => ActionPhase::Apply,
            Self::Verify { .. } => ActionPhase::Verify,
            Self::Rollback { .. } => ActionPhase::Rollback,
        }
    }

    fn category(&self) -> &'static str {
        match self {
            Self::Preflight { .. } => "preflight_failure",
            Self::DryRun { .. } => "dry_run_failure",
            Self::Apply { .. } => "apply_failure",
            Self::Verify { .. } => "verify_failure",
            Self::Rollback { .. } => "rollback_failure",
        }
    }

    fn human_message(&self) -> String {
        match self {
            Self::Preflight { message } => format!("preflight failed: {message}"),
            Self::DryRun { message } => format!("dry run failed: {message}"),
            Self::Apply { message } => format!("apply failed: {message}"),
            Self::Verify { message } => format!("verify failed: {message}"),
            Self::Rollback { message } => format!("rollback failed: {message}"),
        }
    }
}

impl ActionTimeout {
    fn human_message(self) -> String {
        format!(
            "action timed out during {}: elapsed_ms={} timeout_ms={}",
            self.phase.as_str(),
            self.elapsed_ms,
            self.timeout_ms
        )
    }
}

impl RollbackOutcome {
    fn phase(&self) -> ActionPhase {
        match self {
            Self::RollbackFailure { .. } => ActionPhase::Rollback,
            Self::VerifyRollbackCompleted { .. } => ActionPhase::Verify,
            Self::EmergencyRollbackFailure { .. } => ActionPhase::EmergencyRollback,
            Self::TimeoutRollbackCompleted { timeout } => timeout.phase,
            Self::TimeoutRollbackFailure { .. } => ActionPhase::EmergencyRollback,
        }
    }

    fn category(&self) -> &'static str {
        match self {
            Self::RollbackFailure { .. } => "rollback_failure",
            Self::VerifyRollbackCompleted { .. } => "verify_failure_rollback_completed",
            Self::EmergencyRollbackFailure { .. } => "emergency_rollback_failure",
            Self::TimeoutRollbackCompleted { .. } => "timeout_rollback_completed",
            Self::TimeoutRollbackFailure { .. } => "timeout_rollback_failure",
        }
    }

    fn human_message(&self) -> String {
        match self {
            Self::RollbackFailure { message } => format!("rollback failed: {message}"),
            Self::VerifyRollbackCompleted { verify_error } => {
                format!("verify failed; rollback completed: {verify_error}")
            }
            Self::EmergencyRollbackFailure {
                verify_error,
                rollback_error,
            } => format!(
                "verify failed; emergency rollback failed: verify error: {verify_error}; rollback error: {rollback_error}"
            ),
            Self::TimeoutRollbackCompleted { timeout } => format!(
                "action timed out during {}; rollback completed: elapsed_ms={} timeout_ms={}",
                timeout.phase.as_str(),
                timeout.elapsed_ms,
                timeout.timeout_ms
            ),
            Self::TimeoutRollbackFailure {
                timeout,
                rollback_error,
            } => format!(
                "action timed out during {}; emergency rollback failed: elapsed_ms={} timeout_ms={}; rollback error: {}",
                timeout.phase.as_str(),
                timeout.elapsed_ms,
                timeout.timeout_ms,
                rollback_error
            ),
        }
    }
}

impl ScopeLimitExceeded {
    fn human_message(self) -> String {
        format!(
            "dry run affected {} task(s), exceeding scope limit {}",
            self.affected_tasks, self.max_affected_tasks
        )
    }
}

impl From<ActionFailure> for ActionError {
    fn from(failure: ActionFailure) -> Self {
        Self::from_failure(failure)
    }
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

impl Serialize for ActionError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.failure.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ActionError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::from_failure(ActionFailure::deserialize(
            deserializer,
        )?))
    }
}

impl Serialize for ActionFailure {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let wire = ActionErrorWire::from(self.clone());
        wire.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ActionFailure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(ActionErrorWire::deserialize(deserializer)?.into())
    }
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.human_message())
    }
}

impl std::error::Error for ActionError {}

pub type ActionResult<T> = anyhow::Result<T>;

#[derive(Debug)]
pub struct PartialApplyError {
    pub source: anyhow::Error,
    pub rollback: Option<crate::actions::RollbackToken>,
}

impl std::fmt::Display for PartialApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl std::error::Error for PartialApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl From<anyhow::Error> for PartialApplyError {
    fn from(source: anyhow::Error) -> Self {
        Self {
            source,
            rollback: None,
        }
    }
}

pub type ApplyResult = Result<crate::actions::RollbackToken, PartialApplyError>;

