//! Typed errors for the privileged-worker and privilege-boundary path.

use std::path::PathBuf;

use thiserror::Error;

use super::PrivilegedWorkerResponse;

#[derive(Debug, Error)]
pub enum PrivilegedWorkerError {
    #[error("stale_candidate_plan: stale candidate plan: age {age_ns}ns exceeded max {max_ns}ns")]
    StaleCandidatePlan { age_ns: u128, max_ns: u128 },

    #[error(
        "candidate_plan_descriptor_mismatch: candidate plan descriptor does not match action payload"
    )]
    CandidatePlanDescriptorMismatch,

    #[error("candidate_plan_objective_mismatch: candidate objective changed")]
    CandidatePlanObjectiveMismatch,

    #[error("candidate_plan_missing_evidence: candidate plan has no evidence")]
    CandidatePlanMissingEvidence,

    #[error(
        "privileged_worker_unsupported_schema: unsupported privileged worker IPC schema: got {got}, expected {expected}"
    )]
    UnsupportedSchema { got: u32, expected: u32 },

    #[error(
        "privileged_worker_unsupported_candidate_plan_schema: unsupported candidate plan schema: got {got}, expected {expected}"
    )]
    UnsupportedCandidatePlanSchema { got: u32, expected: u32 },

    #[error(
        "privileged_worker_candidate_plan_metadata_mismatch: candidate plan metadata does not match worker request"
    )]
    CandidatePlanMetadataMismatch,

    #[error(
        "privileged_worker_candidate_plan_manual_only: candidate '{candidate_name}' action_kind={action_kind} is manual-only: {reason}"
    )]
    CandidatePlanManualOnly {
        candidate_name: String,
        action_kind: String,
        reason: String,
    },

    #[error(
        "privileged_worker_candidate_plan_not_executable: candidate '{candidate_name}' action_kind={action_kind} has no executable payload"
    )]
    CandidatePlanNotExecutable {
        candidate_name: String,
        action_kind: String,
    },

    #[error(
        "privileged_worker_empty_response: privileged worker returned an empty response from {socket_path}"
    )]
    EmptyResponse { socket_path: PathBuf },

    #[error(
        "privileged_worker_unexpected_response: privileged worker returned unexpected response: {response:?}"
    )]
    UnexpectedResponse {
        response: Box<PrivilegedWorkerResponse>,
    },

    #[error(
        "privileged_worker_socket_not_ready: privileged worker socket {socket_path} was not connectable within {timeout_ms}ms{last_error_suffix}"
    )]
    SocketNotReady {
        socket_path: PathBuf,
        timeout_ms: u128,
        last_error: Option<String>,
        last_error_suffix: String,
    },

    #[error(
        "privileged_worker_missing_socket_runtime_directory: cannot choose default privileged worker socket path without XDG_RUNTIME_DIR or HOME"
    )]
    MissingSocketRuntimeDirectory,

    #[error(
        "privileged_worker_socket_refusing_non_socket: refusing to replace non-socket path {path}"
    )]
    RefusingNonSocket { path: PathBuf },

    #[error("privileged_worker_empty_request: privileged worker request body was empty")]
    EmptyRequest,

    #[error("privileged_apply_missing_rollback: privileged apply completed without rollback token")]
    MissingRollbackToken,
}

impl PrivilegedWorkerError {
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::StaleCandidatePlan { .. } => "stale_candidate_plan",
            Self::CandidatePlanDescriptorMismatch => "candidate_plan_descriptor_mismatch",
            Self::CandidatePlanObjectiveMismatch => "candidate_plan_objective_mismatch",
            Self::CandidatePlanMissingEvidence => "candidate_plan_missing_evidence",
            Self::UnsupportedSchema { .. } => "privileged_worker_unsupported_schema",
            Self::UnsupportedCandidatePlanSchema { .. } => {
                "privileged_worker_unsupported_candidate_plan_schema"
            }
            Self::CandidatePlanMetadataMismatch => {
                "privileged_worker_candidate_plan_metadata_mismatch"
            }
            Self::CandidatePlanManualOnly { .. } => "privileged_worker_candidate_plan_manual_only",
            Self::CandidatePlanNotExecutable { .. } => {
                "privileged_worker_candidate_plan_not_executable"
            }
            Self::EmptyResponse { .. } => "privileged_worker_empty_response",
            Self::UnexpectedResponse { .. } => "privileged_worker_unexpected_response",
            Self::SocketNotReady { .. } => "privileged_worker_socket_not_ready",
            Self::MissingSocketRuntimeDirectory => {
                "privileged_worker_missing_socket_runtime_directory"
            }
            Self::RefusingNonSocket { .. } => "privileged_worker_socket_refusing_non_socket",
            Self::EmptyRequest => "privileged_worker_empty_request",
            Self::MissingRollbackToken => "privileged_apply_missing_rollback",
        }
    }

    #[cfg(test)]
    pub fn message_with_reason_code(&self) -> String {
        format!("{}: {self}", self.reason_code())
    }
}
