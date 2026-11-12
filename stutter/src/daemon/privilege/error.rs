//! Typed errors for the privileged-worker and privilege-boundary path.

use std::path::PathBuf;

use thiserror::Error;

use super::PrivilegedWorkerResponse;

#[derive(Debug, Error)]
pub enum PrivilegedWorkerError {
    #[error(
        "service_token_already_outstanding: the previous identical transaction must be restored before another prepare"
    )]
    ServiceTokenAlreadyOutstanding,
    #[error(
        "settle_issued_tokens_unsupported: only the bounded service worker keeps an issued-token ledger"
    )]
    SettleIssuedTokensUnsupported,
    #[error("fence_status_unsupported: only the bounded service worker reads a validation fence")]
    FenceStatusUnsupported,
    #[error("stale_target_before_mutation: {detail}")]
    StaleTargetBeforeMutation { detail: String },
    #[error(
        "service_rollback_unresolved: a live task is still in stutter's state and its ownership or original resource could not be resolved"
    )]
    ServiceRollbackUnresolved,
    #[error(
        "service_rollback_token_not_issued: rollback data does not match a worker-issued durable token"
    )]
    ServiceRollbackTokenNotIssued,
    #[error(
        "bounded_service_requires_prepared_transaction: service applies must use prepare and digest-bound commit"
    )]
    BoundedServiceRequiresPrepared,
    #[error("bounded_service_backend_unknown: service backend is not recognized")]
    BoundedServiceBackendUnknown,
    #[error("stale_candidate_plan: stale candidate plan: age {age_ns}ns exceeded max {max_ns}ns")]
    StaleCandidatePlan { age_ns: u128, max_ns: u128 },

    #[error("privileged_worker_invalid_action_descriptor: {reason}")]
    InvalidActionDescriptor { reason: String },

    #[error(
        "candidate_plan_descriptor_mismatch: candidate plan descriptor does not match action payload"
    )]
    CandidatePlanDescriptorMismatch,

    #[error("candidate_plan_objective_mismatch: candidate objective changed")]
    CandidatePlanObjectiveMismatch,

    #[error("candidate_plan_operation_mismatch: candidate operation changed")]
    CandidatePlanOperationMismatch,

    #[error(
        "system_wide_operation_not_allowlisted: operation {operation} is not on the system-wide operation allowlist"
    )]
    SystemWideOperationNotAllowlisted { operation: String },

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

    #[error("privileged_worker_socket_in_use: a worker is already listening at {path}")]
    SocketInUse { path: PathBuf },

    #[error("privileged_worker_socket_changed: the socket path {path} was replaced")]
    SocketChanged { path: PathBuf },

    #[error("privileged_worker_peer_mismatch: expected process {expected}, got {actual}")]
    SocketPeerMismatch { expected: u32, actual: u32 },

    #[error("privileged_worker_shutdown_response_invalid: invalid shutdown response from {path}")]
    ShutdownResponseInvalid { path: PathBuf },

    #[error("privileged_worker_shutdown_timeout: worker did not exit within {timeout_ms}ms")]
    ShutdownTimeout { timeout_ms: u128 },

    #[error("privileged_worker_shutdown_failed: worker exited with {status}")]
    ShutdownFailed { status: String },

    #[error("privileged_worker_empty_request: privileged worker request body was empty")]
    EmptyRequest,

    #[error("privileged_apply_missing_rollback: privileged apply completed without rollback token")]
    MissingRollbackToken,

    #[error(
        "privileged_prepared_apply_unsupported: this privileged service has no two-phase apply"
    )]
    PreparedApplyUnsupported,

    #[error(
        "privileged_prepared_hook_not_invoked: the action mutated without reaching the commit barrier"
    )]
    PreparedHookNotInvoked,

    #[error("privileged_commit_rejected: {reason}")]
    CommitRejected { reason: String },

    #[error(
        "privileged_commit_expired: no commit frame arrived within {timeout_ms} ms of the prepare"
    )]
    CommitExpired { timeout_ms: u128 },

    #[error(
        "bounded_service_target_boundary_unimplemented: this operation family has no registry-qualified service resource boundary"
    )]
    BoundedServiceTargetBoundaryUnimplemented,
}

impl PrivilegedWorkerError {
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::ServiceTokenAlreadyOutstanding => "service_token_already_outstanding",
            Self::SettleIssuedTokensUnsupported => "settle_issued_tokens_unsupported",
            Self::FenceStatusUnsupported => "fence_status_unsupported",
            Self::ServiceRollbackUnresolved => "service_rollback_unresolved",
            Self::StaleTargetBeforeMutation { .. } => "stale_target_before_mutation",
            Self::ServiceRollbackTokenNotIssued => "service_rollback_token_not_issued",
            Self::BoundedServiceRequiresPrepared => "bounded_service_requires_prepared_transaction",
            Self::BoundedServiceBackendUnknown => "bounded_service_backend_unknown",
            Self::BoundedServiceTargetBoundaryUnimplemented => {
                "bounded_service_target_boundary_unimplemented"
            }
            Self::StaleCandidatePlan { .. } => "stale_candidate_plan",
            Self::InvalidActionDescriptor { .. } => "privileged_worker_invalid_action_descriptor",
            Self::CandidatePlanDescriptorMismatch => "candidate_plan_descriptor_mismatch",
            Self::CandidatePlanObjectiveMismatch => "candidate_plan_objective_mismatch",
            Self::CandidatePlanOperationMismatch => "candidate_plan_operation_mismatch",
            Self::CandidatePlanMissingEvidence => "candidate_plan_missing_evidence",
            Self::SystemWideOperationNotAllowlisted { .. } => {
                "system_wide_operation_not_allowlisted"
            }
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
            Self::SocketInUse { .. } => "privileged_worker_socket_in_use",
            Self::SocketChanged { .. } => "privileged_worker_socket_changed",
            Self::SocketPeerMismatch { .. } => "privileged_worker_peer_mismatch",
            Self::ShutdownResponseInvalid { .. } => "privileged_worker_shutdown_response_invalid",
            Self::ShutdownTimeout { .. } => "privileged_worker_shutdown_timeout",
            Self::ShutdownFailed { .. } => "privileged_worker_shutdown_failed",
            Self::EmptyRequest => "privileged_worker_empty_request",
            Self::MissingRollbackToken => "privileged_apply_missing_rollback",
            Self::PreparedApplyUnsupported => "privileged_prepared_apply_unsupported",
            Self::PreparedHookNotInvoked => "privileged_prepared_hook_not_invoked",
            Self::CommitRejected { .. } => "privileged_commit_rejected",
            Self::CommitExpired { .. } => "privileged_commit_expired",
        }
    }

    #[cfg(test)]
    pub fn message_with_reason_code(&self) -> String {
        format!("{}: {self}", self.reason_code())
    }
}
