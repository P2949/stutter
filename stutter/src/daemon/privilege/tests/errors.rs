use super::*;

#[test]
fn privileged_worker_error_reason_codes_are_stable() {
    let cases = [
        (
            PrivilegedWorkerError::StaleCandidatePlan {
                age_ns: 20,
                max_ns: 10,
            },
            "stale_candidate_plan",
        ),
        (
            PrivilegedWorkerError::CandidatePlanDescriptorMismatch,
            "candidate_plan_descriptor_mismatch",
        ),
        (
            PrivilegedWorkerError::CandidatePlanObjectiveMismatch,
            "candidate_plan_objective_mismatch",
        ),
        (
            PrivilegedWorkerError::CandidatePlanMissingEvidence,
            "candidate_plan_missing_evidence",
        ),
        (
            PrivilegedWorkerError::UnsupportedSchema {
                got: 2,
                expected: 1,
            },
            "privileged_worker_unsupported_schema",
        ),
        (
            PrivilegedWorkerError::UnsupportedCandidatePlanSchema {
                got: 2,
                expected: 1,
            },
            "privileged_worker_unsupported_candidate_plan_schema",
        ),
        (
            PrivilegedWorkerError::CandidatePlanMetadataMismatch,
            "privileged_worker_candidate_plan_metadata_mismatch",
        ),
        (
            PrivilegedWorkerError::CandidatePlanManualOnly {
                candidate_name: "candidate".to_owned(),
                action_kind: "kind".to_owned(),
                reason: "manual".to_owned(),
            },
            "privileged_worker_candidate_plan_manual_only",
        ),
        (
            PrivilegedWorkerError::CandidatePlanNotExecutable {
                candidate_name: "candidate".to_owned(),
                action_kind: "kind".to_owned(),
            },
            "privileged_worker_candidate_plan_not_executable",
        ),
        (
            PrivilegedWorkerError::EmptyResponse {
                socket_path: PathBuf::from("/tmp/stutter.sock"),
            },
            "privileged_worker_empty_response",
        ),
        (
            PrivilegedWorkerError::UnexpectedResponse {
                response: Box::new(PrivilegedWorkerResponse::Shutdown {
                    message: "bye".to_owned(),
                }),
            },
            "privileged_worker_unexpected_response",
        ),
        (
            PrivilegedWorkerError::SocketNotReady {
                socket_path: PathBuf::from("/tmp/stutter.sock"),
                timeout_ms: 100,
                last_error: Some("refused".to_owned()),
                last_error_suffix: "; last_error=refused".to_owned(),
            },
            "privileged_worker_socket_not_ready",
        ),
        (
            PrivilegedWorkerError::MissingSocketRuntimeDirectory,
            "privileged_worker_missing_socket_runtime_directory",
        ),
        (
            PrivilegedWorkerError::RefusingNonSocket {
                path: PathBuf::from("/tmp/not-a-socket"),
            },
            "privileged_worker_socket_refusing_non_socket",
        ),
        (
            PrivilegedWorkerError::EmptyRequest,
            "privileged_worker_empty_request",
        ),
    ];

    for (error, expected_reason_code) in cases {
        assert_eq!(error.reason_code(), expected_reason_code);
        assert!(
            error
                .message_with_reason_code()
                .starts_with(expected_reason_code),
            "{error:?} did not include stable reason code"
        );
    }
}

#[test]
fn privileged_worker_response_from_error_prefers_typed_reason_code() {
    let response = PrivilegedWorkerResponse::from_error(
        PrivilegedWorkerError::UnsupportedSchema {
            got: 2,
            expected: 1,
        }
        .into(),
    );

    match response {
        PrivilegedWorkerResponse::Error {
            reason_code,
            message,
        } => {
            assert_eq!(reason_code, "privileged_worker_unsupported_schema");
            assert!(message.contains("unsupported privileged worker IPC schema"));
        }
        other => panic!("expected error response, got {other:?}"),
    }
}

#[test]
fn privileged_worker_response_from_error_keeps_legacy_string_reason_code_fallback() {
    let response = PrivilegedWorkerResponse::from_error(anyhow::anyhow!(
        "legacy_reason_code: old style error"
    ));

    match response {
        PrivilegedWorkerResponse::Error {
            reason_code,
            message,
        } => {
            assert_eq!(reason_code, "legacy_reason_code");
            assert!(message.contains("old style error"));
        }
        other => panic!("expected error response, got {other:?}"),
    }
}
