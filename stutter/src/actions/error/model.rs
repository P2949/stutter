use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::actions::{model::ActionPhase, token::RollbackTokenKindError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionError {
    failure: ActionFailure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionFailure {
    Phase(PhaseFailure),
    Boundary(ActionBoundaryFailure),
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
        serialize_with = "crate::actions::error::serde::serialize_u128_as_u64",
        deserialize_with = "crate::actions::error::serde::deserialize_u128_from_u64"
    )]
    pub elapsed_ms: u128,
    #[serde(
        serialize_with = "crate::actions::error::serde::serialize_u128_as_u64",
        deserialize_with = "crate::actions::error::serde::deserialize_u128_from_u64"
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
pub struct ActionBoundaryFailure {
    pub phase: ActionPhase,
    pub action_kind: String,
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ActionBoundaryError {
    #[error(
        "action_missing_explicit_rollback_token: {handler} requires explicit rollback token kind {expected_token_kind}"
    )]
    MissingExplicitRollbackToken {
        handler: &'static str,
        expected_token_kind: &'static str,
    },

    #[error(
        "action_unsupported_rollback_token: {handler} expected rollback token kind {expected_token_kind}, got {actual_token_kind}"
    )]
    UnsupportedRollbackToken {
        handler: &'static str,
        expected_token_kind: &'static str,
        actual_token_kind: &'static str,
    },

    #[error("action_missing_explicit_targets: {action_kind} requires at least one explicit target")]
    MissingExplicitTargets { action_kind: &'static str },

    #[error("action_policy_denied: {action_kind} denied by policy requirement {requirement}")]
    PolicyDenied {
        action_kind: &'static str,
        requirement: &'static str,
    },

    #[error("action_evidence_required: {action_kind} requires evidence {evidence}")]
    EvidenceRequired {
        action_kind: &'static str,
        evidence: &'static str,
    },

    #[error("action_invalid_target_tid: {action_kind} target tid {tid} must be greater than zero")]
    InvalidTargetTid { action_kind: &'static str, tid: u32 },

    #[error(
        "action_target_identity_mismatch: {action_kind} tid={tid} starttime mismatch: expected={expected_starttime} actual={actual_starttime}"
    )]
    TargetIdentityMismatch {
        action_kind: &'static str,
        tid: u32,
        expected_starttime: u64,
        actual_starttime: u64,
    },

    #[error("action_invalid_request: {action_kind}: {reason}")]
    InvalidRequest {
        action_kind: &'static str,
        reason: String,
    },

    #[error("action_invalid_policy: {action_kind}: {reason}")]
    InvalidPolicy {
        action_kind: &'static str,
        reason: String,
    },

    #[error("action_invalid_value: {action_kind} field {field}: {reason}")]
    InvalidValue {
        action_kind: &'static str,
        field: String,
        reason: String,
    },

    #[error("action_unsupported_value: {action_kind} field {field}: {value}")]
    UnsupportedValue {
        action_kind: &'static str,
        field: &'static str,
        value: String,
    },

    #[error("action_path_not_allowed: {action_kind} path {path}: {reason}")]
    PathNotAllowed {
        action_kind: &'static str,
        path: PathBuf,
        reason: String,
    },

    #[error("action_missing_path: {action_kind} required path does not exist: {path}")]
    MissingPath {
        action_kind: &'static str,
        path: PathBuf,
    },

    #[error("action_restore_failed: {action_kind}: {message}")]
    RestoreFailed {
        action_kind: &'static str,
        message: String,
    },
}

impl ActionBoundaryError {
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::MissingExplicitRollbackToken { .. } => "action_missing_explicit_rollback_token",
            Self::UnsupportedRollbackToken { .. } => "action_unsupported_rollback_token",
            Self::MissingExplicitTargets { .. } => "action_missing_explicit_targets",
            Self::PolicyDenied { .. } => "action_policy_denied",
            Self::EvidenceRequired { .. } => "action_evidence_required",
            Self::InvalidTargetTid { .. } => "action_invalid_target_tid",
            Self::TargetIdentityMismatch { .. } => "action_target_identity_mismatch",
            Self::InvalidRequest { .. } => "action_invalid_request",
            Self::InvalidPolicy { .. } => "action_invalid_policy",
            Self::InvalidValue { .. } => "action_invalid_value",
            Self::UnsupportedValue { .. } => "action_unsupported_value",
            Self::PathNotAllowed { .. } => "action_path_not_allowed",
            Self::MissingPath { .. } => "action_missing_path",
            Self::RestoreFailed { .. } => "action_restore_failed",
        }
    }

    pub const fn action_kind(&self) -> &'static str {
        match self {
            Self::MissingExplicitRollbackToken { handler, .. } => handler,
            Self::UnsupportedRollbackToken { handler, .. } => handler,
            Self::MissingExplicitTargets { action_kind }
            | Self::PolicyDenied { action_kind, .. }
            | Self::EvidenceRequired { action_kind, .. }
            | Self::InvalidTargetTid { action_kind, .. }
            | Self::TargetIdentityMismatch { action_kind, .. }
            | Self::InvalidRequest { action_kind, .. }
            | Self::InvalidPolicy { action_kind, .. }
            | Self::InvalidValue { action_kind, .. }
            | Self::UnsupportedValue { action_kind, .. }
            | Self::PathNotAllowed { action_kind, .. }
            | Self::MissingPath { action_kind, .. }
            | Self::RestoreFailed { action_kind, .. } => action_kind,
        }
    }

    pub fn to_failure(&self, phase: ActionPhase) -> ActionBoundaryFailure {
        ActionBoundaryFailure {
            phase,
            action_kind: self.action_kind().to_owned(),
            reason_code: self.reason_code().to_owned(),
            message: self.to_string(),
        }
    }

    pub fn unsupported_rollback_token(
        handler: &'static str,
        expected_token_kind: &'static str,
        actual_token_kind: &'static str,
    ) -> Self {
        Self::UnsupportedRollbackToken {
            handler,
            expected_token_kind,
            actual_token_kind,
        }
    }

    pub fn missing_explicit_rollback_token(
        handler: &'static str,
        expected_token_kind: &'static str,
    ) -> Self {
        Self::MissingExplicitRollbackToken {
            handler,
            expected_token_kind,
        }
    }

    pub fn restore_failed(action_kind: &'static str, message: impl Into<String>) -> Self {
        Self::RestoreFailed {
            action_kind,
            message: message.into(),
        }
    }
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
        Self::rollback_error(error)
    }

    pub fn from_phase_error(phase: ActionPhase, error: anyhow::Error) -> Self {
        if let Some(boundary_error) = error.downcast_ref::<ActionBoundaryError>() {
            return Self::from_failure(ActionFailure::Boundary(boundary_error.to_failure(phase)));
        }

        if let Some(kind_error) = error.downcast_ref::<RollbackTokenKindError>()
            && phase == ActionPhase::Rollback
        {
            return Self::invalid_rollback_token_kind(*kind_error);
        }

        match phase {
            ActionPhase::Preflight => Self::preflight(error),
            ActionPhase::DryRun => Self::dry_run(error),
            ActionPhase::Apply => Self::apply(error),
            ActionPhase::Verify => Self::verify(error),
            ActionPhase::Rollback | ActionPhase::EmergencyRollback => Self::rollback(error),
        }
    }

    pub fn preflight_error(error: anyhow::Error) -> Self {
        Self::from_phase_error(ActionPhase::Preflight, error)
    }

    pub fn dry_run_error(error: anyhow::Error) -> Self {
        Self::from_phase_error(ActionPhase::DryRun, error)
    }

    pub fn apply_error(error: anyhow::Error) -> Self {
        Self::from_phase_error(ActionPhase::Apply, error)
    }

    pub fn verify_error(error: anyhow::Error) -> Self {
        Self::from_phase_error(ActionPhase::Verify, error)
    }

    pub fn rollback_error(error: anyhow::Error) -> Self {
        Self::from_phase_error(ActionPhase::Rollback, error)
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

    pub fn category(&self) -> &str {
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
            Self::Boundary(failure) => failure.phase,
            Self::Timeout(timeout) => timeout.phase,
            Self::Rollback(outcome) => outcome.phase(),
            Self::InvalidRollbackToken { .. } => ActionPhase::Rollback,
            Self::ScopeLimitExceeded(_) => ActionPhase::DryRun,
            Self::PolicyRejected { .. } => ActionPhase::Preflight,
        }
    }

    pub fn category(&self) -> &str {
        match self {
            Self::Phase(failure) => failure.category(),
            Self::Boundary(failure) => failure.reason_code.as_str(),
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
            Self::Boundary(failure) => {
                format!("{} failed: {}", failure.phase.as_str(), failure.message)
            }
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
        super::serde::serialize_action_failure(self, serializer)
    }
}

impl<'de> Deserialize<'de> for ActionFailure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::serde::deserialize_action_failure(deserializer)
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
