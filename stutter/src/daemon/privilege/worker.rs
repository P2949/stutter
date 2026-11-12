//! Privileged worker process, socket lifecycle, and request execution.

use std::os::unix::fs::MetadataExt;

use super::{
    exchange::{CommitExchange, commit_timeout},
    worker_lifecycle::{
        prepare_privileged_worker_socket_path, remove_owned_socket, socket_identity,
    },
    *,
};

pub fn default_privileged_worker_socket_path() -> anyhow::Result<PathBuf> {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime_dir).join("stutter-privileged-worker.sock"));
    }

    let Some(home) = std::env::var_os("HOME") else {
        return Err(PrivilegedWorkerError::MissingSocketRuntimeDirectory.into());
    };

    Ok(PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("stutter")
        .join("privileged-worker.sock"))
}

pub fn run_privileged_worker(socket_path: &Path) -> anyhow::Result<()> {
    let service = InProcessPrivilegedActionService::default();
    run_privileged_worker_with_service(socket_path, &service)
}

pub fn run_privileged_worker_with_service(
    socket_path: &Path,
    service: &dyn PrivilegedActionService,
) -> anyhow::Result<()> {
    run_worker(socket_path, service, None)
}

pub(super) fn run_worker(
    socket_path: &Path,
    service: &dyn PrivilegedActionService,
    service_security: Option<(u32, crate::daemon_policy::DaemonPolicy)>,
) -> anyhow::Result<()> {
    prepare_privileged_worker_socket_path(socket_path)?;
    let listener = UnixListener::bind(socket_path).with_context(|| {
        format!(
            "failed to bind privileged worker socket {}",
            socket_path.display()
        )
    })?;
    let bound_identity = socket_identity(socket_path)?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "failed to set permissions on privileged worker socket {}",
            socket_path.display()
        )
    })?;

    if let Some((uid, _)) = &service_security {
        std::os::unix::fs::chown(socket_path, Some(*uid), None)
            .context("set private service socket owner")?;
    }

    log::info!(
        "privileged_worker_listening socket={} auth=filesystem_permissions mode=0600",
        socket_path.display()
    );

    let result = loop {
        let (stream, _) = listener.accept().with_context(|| {
            format!(
                "failed to accept privileged worker connection on {}",
                socket_path.display()
            )
        })?;
        let peer = match super::peer::peer_credentials(&stream) {
            Ok(peer) => peer,
            Err(error) => {
                log::warn!("privileged_worker_peer_credentials_failed error={error}");
                continue;
            }
        };
        let owner = service_security
            .as_ref()
            .map(|(uid, _)| *uid)
            .unwrap_or(fs::metadata(socket_path)?.uid());
        if peer.uid != owner {
            log::warn!(
                "privileged_worker_peer_denied uid={} expected_uid={owner}",
                peer.uid
            );
            continue;
        }
        match handle_privileged_worker_connection(
            stream,
            service,
            service_security.as_ref().map(|(_, policy)| policy),
        ) {
            Ok(true) => break Ok(()),
            Ok(false) => {}
            Err(err) => {
                log::warn!("privileged_worker_connection_failed err={err:#}");
            }
        }
    };

    let cleanup_result = remove_owned_socket(socket_path, bound_identity).with_context(|| {
        format!(
            "failed to remove privileged worker socket {}",
            socket_path.display()
        )
    });
    result.and(cleanup_result)
}

fn handle_privileged_worker_connection(
    mut stream: UnixStream,
    service: &dyn PrivilegedActionService,
    service_policy: Option<&crate::daemon_policy::DaemonPolicy>,
) -> anyhow::Result<bool> {
    let mut request = match read_privileged_worker_request(&stream) {
        Ok(request) => request,
        Err(err) => {
            let response = PrivilegedWorkerResponse::from_error(err);
            serde_json::to_writer(&mut stream, &response)?;
            stream.write_all(b"\n")?;
            stream.flush()?;
            return Ok(false);
        }
    };
    if service_policy.is_some()
        && let Err(error) = super::bounded::validate_request(&request)
    {
        serde_json::to_writer(&mut stream, &PrivilegedWorkerResponse::from_error(error))?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        return Ok(false);
    }
    if let Some(service_policy) = service_policy {
        match &mut request {
            PrivilegedWorkerRequest::DryRun { policy, .. }
            | PrivilegedWorkerRequest::ApplyPrepared { policy, .. }
            | PrivilegedWorkerRequest::Rollback { policy, .. } => *policy = service_policy.clone(),
            _ => (), // The closed request validator has already rejected unprepared applies.
        }
    }
    if matches!(
        request,
        PrivilegedWorkerRequest::SettleIssuedTokens | PrivilegedWorkerRequest::FenceStatus
    ) {
        // Neither carries data. FenceStatus is read-only: the worker reads its own fence now.
        let response = match (&request, service_policy) {
            (PrivilegedWorkerRequest::FenceStatus, Some(_)) => service
                .fence_status()
                .map(|report| PrivilegedWorkerResponse::FenceStatus { report })
                .unwrap_or_else(PrivilegedWorkerResponse::from_error),
            (PrivilegedWorkerRequest::FenceStatus, None) => PrivilegedWorkerResponse::from_error(
                PrivilegedWorkerError::FenceStatusUnsupported.into(),
            ),
            (_, Some(policy)) => service
                .settle_issued_tokens(policy)
                .map(|outstanding| PrivilegedWorkerResponse::Settled { outstanding })
                .unwrap_or_else(PrivilegedWorkerResponse::from_error),
            (_, None) => PrivilegedWorkerResponse::from_error(
                PrivilegedWorkerError::SettleIssuedTokensUnsupported.into(),
            ),
        };
        serde_json::to_writer(&mut stream, &response)?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        return Ok(false);
    }
    let shutdown = matches!(request, PrivilegedWorkerRequest::Shutdown);
    let response = if matches!(request, PrivilegedWorkerRequest::ApplyPrepared { .. }) {
        let audit_sink = PrivilegeAuditSink::default();
        let mut exchange = CommitExchange::new(&stream, commit_timeout());
        execute_privileged_worker_request_inner(
            request,
            service,
            &audit_sink,
            Some(&mut |token: &RollbackToken| exchange.prepare_and_wait(token)),
        )
    } else {
        execute_privileged_worker_request(request, service)
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(shutdown)
}

fn read_privileged_worker_request(stream: &UnixStream) -> anyhow::Result<PrivilegedWorkerRequest> {
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim().is_empty() {
        return Err(PrivilegedWorkerError::EmptyRequest.into());
    }

    serde_json::from_str(&line)
        .with_context(|| "privileged_worker_decode_request_failed: invalid worker request JSON")
}

pub(crate) fn execute_privileged_worker_request(
    request: PrivilegedWorkerRequest,
    service: &dyn PrivilegedActionService,
) -> PrivilegedWorkerResponse {
    let audit_sink = PrivilegeAuditSink::default();
    execute_privileged_worker_request_with_audit_sink(request, service, &audit_sink)
}

pub(crate) fn execute_privileged_worker_request_with_audit_sink(
    request: PrivilegedWorkerRequest,
    service: &dyn PrivilegedActionService,
    audit_sink: &PrivilegeAuditSink,
) -> PrivilegedWorkerResponse {
    execute_privileged_worker_request_inner(request, service, audit_sink, None)
}

/// The commit barrier of a two-phase apply: called with the durable rollback token, it answers
/// the controller and returns only once the controller committed.
type CommitBarrier<'a> = &'a mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>;

fn execute_privileged_worker_request_inner(
    request: PrivilegedWorkerRequest,
    service: &dyn PrivilegedActionService,
    audit_sink: &PrivilegeAuditSink,
    barrier: Option<CommitBarrier<'_>>,
) -> PrivilegedWorkerResponse {
    let operation = request.operation();
    let request_name = request.request_name();
    let policy_intent = request.policy_intent();
    let descriptor = request.descriptor().cloned();
    let decision = PrivilegeCommandAllowlist.check(&PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::ControlPlane,
        operation,
        transport: PrivilegeTransport::UnixSocket,
        authenticated: true,
        apply_authorized: true,
    });

    log::info!(
        "privileged_worker_boundary_decision request={} operation={} allowed={} reason_code={}",
        request_name,
        operation.as_str(),
        decision.allowed,
        decision.reason_code
    );

    record_worker_boundary_audit(
        audit_sink,
        PrivilegeBoundaryAuditInput {
            stage: "allowlist_decision",
            request_kind: request_name,
            operation,
            policy_intent: policy_intent.clone(),
            caller_role: PrivilegeProcessRole::ControlPlane,
            transport: PrivilegeTransport::UnixSocket,
            descriptor: descriptor.as_ref(),
            success: decision.allowed,
            reason_code: &decision.reason_code,
            detail: &decision.message,
            affected_tasks: 0,
            mutation: None,
        },
    );

    if !decision.allowed {
        return PrivilegedWorkerResponse::Error {
            reason_code: decision.reason_code,
            message: decision.message,
        };
    }

    let result = match request {
        PrivilegedWorkerRequest::DryRun {
            plan,
            policy,
            context,
            max_plan_age_nanos,
        } => plan.into_plan_request().and_then(|plan| {
            service
                .dry_run_candidate(CandidateApplyRequest {
                    plan,
                    policy,
                    context,
                    max_plan_age_nanos,
                })
                .map(|record| PrivilegedWorkerResponse::DryRun { record })
        }),
        PrivilegedWorkerRequest::Apply {
            plan,
            policy,
            context,
            max_plan_age_nanos,
        } => plan.into_plan_request().and_then(|plan| {
            service
                .apply_candidate(CandidateApplyRequest {
                    plan,
                    policy,
                    context,
                    max_plan_age_nanos,
                })
                .map(|result| PrivilegedWorkerResponse::Apply { result })
        }),
        PrivilegedWorkerRequest::ApplyPrepared {
            plan,
            policy,
            context,
            max_plan_age_nanos,
        } => match barrier {
            Some(barrier) => plan.into_plan_request().and_then(|plan| {
                service
                    .apply_candidate_prepared(
                        CandidateApplyRequest {
                            plan,
                            policy,
                            context,
                            max_plan_age_nanos,
                        },
                        barrier,
                    )
                    .map(|result| PrivilegedWorkerResponse::Apply { result })
            }),
            None => Err(anyhow::anyhow!(
                "privileged_prepared_apply_requires_stream: apply_prepared needs the exchange connection"
            )),
        },
        PrivilegedWorkerRequest::CommitPrepared { .. } => Err(anyhow::anyhow!(
            "privileged_commit_without_prepare: a commit is only valid as the answer to a prepare"
        )),
        PrivilegedWorkerRequest::Rollback {
            plan,
            token,
            policy,
            context,
        } => plan.into_plan_request().and_then(|plan| {
            service
                .rollback(RollbackRequest {
                    candidate: plan.candidate,
                    token,
                    policy,
                    context,
                })
                .map(|result| PrivilegedWorkerResponse::Rollback { result })
        }),
        PrivilegedWorkerRequest::SettleIssuedTokens => {
            Err(PrivilegedWorkerError::SettleIssuedTokensUnsupported.into())
        }
        PrivilegedWorkerRequest::FenceStatus => {
            Err(PrivilegedWorkerError::FenceStatusUnsupported.into())
        }
        PrivilegedWorkerRequest::Shutdown => Ok(PrivilegedWorkerResponse::Shutdown {
            message: "privileged worker shutdown requested".to_owned(),
        }),
    };

    let response = result.unwrap_or_else(PrivilegedWorkerResponse::from_error);
    let (success, stage, reason_code, detail, affected_tasks) = match &response {
        PrivilegedWorkerResponse::DryRun { record } => (
            true,
            "dry_run_completed",
            "dry_run_completed",
            "privileged worker dry-run completed",
            record.affected_tasks,
        ),
        PrivilegedWorkerResponse::Apply { result } => (
            true,
            "apply_completed",
            "apply_completed",
            "privileged worker apply completed",
            result.state.affected_tasks,
        ),
        PrivilegedWorkerResponse::Prepared { .. } => (
            true,
            "prepared",
            "prepared",
            "privileged worker prepared a restore snapshot",
            0,
        ),
        PrivilegedWorkerResponse::Rollback { result } => (
            true,
            "rollback_completed",
            "rollback_completed",
            "privileged worker rollback completed",
            result.affected_tasks,
        ),
        PrivilegedWorkerResponse::Shutdown { .. } => (
            true,
            "shutdown_completed",
            "shutdown_completed",
            "privileged worker shutdown completed",
            0,
        ),
        PrivilegedWorkerResponse::FenceStatus { .. } => (
            true,
            "fence_status_reported",
            "fence_status_reported",
            "privileged worker reported its validation fence",
            0,
        ),
        PrivilegedWorkerResponse::Settled { outstanding } => (
            true,
            "settle_completed",
            "settle_completed",
            "privileged worker settled its issued tokens",
            *outstanding,
        ),
        PrivilegedWorkerResponse::Error {
            reason_code,
            message,
        } => (
            false,
            "request_failed",
            reason_code.as_str(),
            message.as_str(),
            0,
        ),
    };
    record_worker_boundary_audit(
        audit_sink,
        PrivilegeBoundaryAuditInput {
            stage,
            request_kind: request_name,
            operation,
            policy_intent,
            caller_role: PrivilegeProcessRole::ControlPlane,
            transport: PrivilegeTransport::UnixSocket,
            descriptor: descriptor.as_ref(),
            success,
            reason_code,
            detail,
            affected_tasks,
            mutation: None,
        },
    );

    response
}

fn record_worker_boundary_audit(
    audit_sink: &PrivilegeAuditSink,
    input: PrivilegeBoundaryAuditInput<'_>,
) {
    if let Err(err) = audit_sink.record_boundary(input) {
        log::warn!(
            "privileged_worker_boundary_audit_failed path={} err={err:#}",
            audit_sink.path().display()
        );
    }
}
