use serde::{Deserialize, Serialize};

use crate::actions::model::ActionPhase;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionError {
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
        elapsed_ms: u128,
        timeout_ms: u128,
    },
    TimeoutRollbackCompleted {
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
    },
    TimeoutRollbackFailure {
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
        rollback_error: String,
    },
    PolicyRejected {
        message: String,
    },
}

impl ActionError {
    pub fn preflight(error: impl std::fmt::Display) -> Self {
        Self::PreflightFailure {
            message: error.to_string(),
        }
    }

    pub fn dry_run(error: impl std::fmt::Display) -> Self {
        Self::DryRunFailure {
            message: error.to_string(),
        }
    }

    pub fn apply(error: impl std::fmt::Display) -> Self {
        Self::ApplyFailure {
            message: error.to_string(),
        }
    }

    pub fn verify(error: impl std::fmt::Display) -> Self {
        Self::VerifyFailure {
            message: error.to_string(),
        }
    }

    pub fn verify_rollback_completed(verify_error: impl std::fmt::Display) -> Self {
        Self::VerifyFailureRollbackCompleted {
            verify_error: verify_error.to_string(),
        }
    }

    pub fn rollback(error: impl std::fmt::Display) -> Self {
        Self::RollbackFailure {
            message: error.to_string(),
        }
    }

    pub fn emergency_rollback(
        verify_error: impl std::fmt::Display,
        rollback_error: impl std::fmt::Display,
    ) -> Self {
        Self::EmergencyRollbackFailure {
            verify_error: verify_error.to_string(),
            rollback_error: rollback_error.to_string(),
        }
    }

    pub fn policy_rejected(error: impl std::fmt::Display) -> Self {
        Self::PolicyRejected {
            message: error.to_string(),
        }
    }

    pub fn scope_limit_exceeded(affected_tasks: usize, max_affected_tasks: usize) -> Self {
        Self::ScopeLimitExceeded {
            affected_tasks,
            max_affected_tasks,
        }
    }

    pub fn timeout(phase: ActionPhase, elapsed_ms: u128, timeout_ms: u128) -> Self {
        Self::Timeout {
            phase,
            elapsed_ms,
            timeout_ms,
        }
    }

    pub fn timeout_rollback_completed(
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
    ) -> Self {
        Self::TimeoutRollbackCompleted {
            phase,
            elapsed_ms,
            timeout_ms,
        }
    }

    pub fn timeout_rollback_failure(
        phase: ActionPhase,
        elapsed_ms: u128,
        timeout_ms: u128,
        rollback_error: impl std::fmt::Display,
    ) -> Self {
        Self::TimeoutRollbackFailure {
            phase,
            elapsed_ms,
            timeout_ms,
            rollback_error: rollback_error.to_string(),
        }
    }

    pub fn phase(&self) -> ActionPhase {
        match self {
            Self::PreflightFailure { .. } => ActionPhase::Preflight,
            Self::DryRunFailure { .. } => ActionPhase::DryRun,
            Self::ApplyFailure { .. } => ActionPhase::Apply,
            Self::VerifyFailure { .. } | Self::VerifyFailureRollbackCompleted { .. } => {
                ActionPhase::Verify
            }
            Self::RollbackFailure { .. } => ActionPhase::Rollback,
            Self::EmergencyRollbackFailure { .. } => ActionPhase::EmergencyRollback,
            Self::ScopeLimitExceeded { .. } => ActionPhase::DryRun,
            Self::Timeout { phase, .. } | Self::TimeoutRollbackCompleted { phase, .. } => *phase,
            Self::TimeoutRollbackFailure { .. } => ActionPhase::EmergencyRollback,
            Self::PolicyRejected { .. } => ActionPhase::Preflight,
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::PreflightFailure { .. } => "preflight_failure",
            Self::DryRunFailure { .. } => "dry_run_failure",
            Self::ApplyFailure { .. } => "apply_failure",
            Self::VerifyFailure { .. } => "verify_failure",
            Self::VerifyFailureRollbackCompleted { .. } => "verify_failure_rollback_completed",
            Self::RollbackFailure { .. } => "rollback_failure",
            Self::EmergencyRollbackFailure { .. } => "emergency_rollback_failure",
            Self::ScopeLimitExceeded { .. } => "scope_limit_exceeded",
            Self::Timeout { .. } => "timeout",
            Self::TimeoutRollbackCompleted { .. } => "timeout_rollback_completed",
            Self::TimeoutRollbackFailure { .. } => "timeout_rollback_failure",
            Self::PolicyRejected { .. } => "policy_rejected",
        }
    }

    pub fn human_message(&self) -> String {
        match self {
            Self::PreflightFailure { message } => format!("preflight failed: {message}"),
            Self::DryRunFailure { message } => format!("dry run failed: {message}"),
            Self::ApplyFailure { message } => format!("apply failed: {message}"),
            Self::VerifyFailure { message } => format!("verify failed: {message}"),
            Self::VerifyFailureRollbackCompleted { verify_error } => {
                format!("verify failed; rollback completed: {verify_error}")
            }
            Self::RollbackFailure { message } => format!("rollback failed: {message}"),
            Self::EmergencyRollbackFailure {
                verify_error,
                rollback_error,
            } => format!(
                "verify failed; emergency rollback failed: verify error: {verify_error}; rollback error: {rollback_error}"
            ),
            Self::ScopeLimitExceeded {
                affected_tasks,
                max_affected_tasks,
            } => format!(
                "dry run affected {affected_tasks} task(s), exceeding scope limit {max_affected_tasks}"
            ),
            Self::Timeout {
                phase,
                elapsed_ms,
                timeout_ms,
            } => format!(
                "action timed out during {}: elapsed_ms={} timeout_ms={}",
                phase.as_str(),
                elapsed_ms,
                timeout_ms
            ),
            Self::TimeoutRollbackCompleted {
                phase,
                elapsed_ms,
                timeout_ms,
            } => format!(
                "action timed out during {}; rollback completed: elapsed_ms={} timeout_ms={}",
                phase.as_str(),
                elapsed_ms,
                timeout_ms
            ),
            Self::TimeoutRollbackFailure {
                phase,
                elapsed_ms,
                timeout_ms,
                rollback_error,
            } => format!(
                "action timed out during {}; emergency rollback failed: elapsed_ms={} timeout_ms={}; rollback error: {}",
                phase.as_str(),
                elapsed_ms,
                timeout_ms,
                rollback_error
            ),
            Self::PolicyRejected { message } => format!("policy rejected: {message}"),
        }
    }
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.human_message())
    }
}

impl std::error::Error for ActionError {}

pub type ActionResult<T> = anyhow::Result<T>;
