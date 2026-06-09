//! Privileged worker process, socket lifecycle, and request execution.

use super::*;

#[derive(Debug)]
pub struct PrivilegedWorkerHandle {
    child: Child,
    socket_path: PathBuf,
    executable_path: PathBuf,
    restart_count: u32,
    socket_ready_timeout: Duration,
    socket_ready_retry_interval: Duration,
}

impl PrivilegedWorkerHandle {
    pub fn spawn(socket_path: &Path) -> anyhow::Result<Self> {
        let executable = std::env::current_exe()?;
        Self::spawn_with_executable(socket_path, &executable)
    }

    pub fn spawn_with_timing(
        socket_path: &Path,
        socket_ready_timeout: Duration,
        socket_ready_retry_interval: Duration,
    ) -> anyhow::Result<Self> {
        let executable = std::env::current_exe()?;
        Self::spawn_with_executable_and_timing(
            socket_path,
            &executable,
            socket_ready_timeout,
            socket_ready_retry_interval,
        )
    }

    pub fn spawn_with_executable(socket_path: &Path, executable: &Path) -> anyhow::Result<Self> {
        Self::spawn_with_executable_and_timing(
            socket_path,
            executable,
            default_privileged_worker_socket_ready_timeout(),
            default_privileged_worker_socket_ready_retry_interval(),
        )
    }

    pub fn spawn_with_executable_and_timing(
        socket_path: &Path,
        executable: &Path,
        socket_ready_timeout: Duration,
        socket_ready_retry_interval: Duration,
    ) -> anyhow::Result<Self> {
        let _ = fs::remove_file(socket_path);
        prepare_privileged_worker_socket_path(socket_path)?;
        let child = spawn_privileged_worker_child(executable, socket_path)?;
        wait_for_privileged_worker_socket_with_timing(
            socket_path,
            socket_ready_timeout,
            socket_ready_retry_interval,
        )?;
        log::info!(
            "privileged_worker_started socket={} pid={}",
            socket_path.display(),
            child.id()
        );
        Ok(Self {
            child,
            socket_path: socket_path.to_path_buf(),
            executable_path: executable.to_path_buf(),
            restart_count: 0,
            socket_ready_timeout,
            socket_ready_retry_interval,
        })
    }

    pub fn restart_count(&self) -> u32 {
        self.restart_count
    }

    pub fn is_alive(&mut self) -> bool {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .unwrap_or(false)
    }

    pub fn terminate(&mut self) -> anyhow::Result<()> {
        self.child.kill().ok();
        self.child.wait().ok();
        fs::remove_file(&self.socket_path).ok();
        Ok(())
    }

    pub fn restart(&mut self) -> anyhow::Result<()> {
        self.terminate()?;
        prepare_privileged_worker_socket_path(&self.socket_path)?;
        let child = spawn_privileged_worker_child(&self.executable_path, &self.socket_path)?;
        wait_for_privileged_worker_socket_with_timing(
            &self.socket_path,
            self.socket_ready_timeout,
            self.socket_ready_retry_interval,
        )?;
        self.restart_count = self.restart_count.saturating_add(1);
        log::info!(
            "privileged_worker_restarted socket={} restart_count={} pid={}",
            self.socket_path.display(),
            self.restart_count,
            child.id()
        );
        self.child = child;
        Ok(())
    }

    pub fn shutdown_gracefully(
        &mut self,
        timeout: Duration,
        poll_interval: Duration,
    ) -> anyhow::Result<()> {
        if let Ok(mut stream) = UnixStream::connect(&self.socket_path) {
            serde_json::to_writer(&mut stream, &PrivilegedWorkerRequest::Shutdown)?;
            stream.write_all(b"\n")?;
            stream.flush()?;
        }

        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.child.kill().ok();
                self.child.wait().ok();
                return Ok(());
            }
            std::thread::sleep(poll_interval);
        }
    }
}

impl Drop for PrivilegedWorkerHandle {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        fs::remove_file(&self.socket_path).ok();
    }
}

fn spawn_privileged_worker_child(executable: &Path, socket_path: &Path) -> anyhow::Result<Child> {
    let socket = socket_path.to_str().context("socket path is not UTF-8")?;
    Command::new(executable)
        .args(["privileged-worker", "--socket", socket])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn privileged worker for socket {}",
                socket_path.display()
            )
        })
}

fn default_privileged_worker_socket_ready_timeout() -> Duration {
    Duration::from_millis(crate::daemon::config::DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_TIMEOUT_MS)
}

fn default_privileged_worker_socket_ready_retry_interval() -> Duration {
    Duration::from_millis(crate::daemon::config::DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_RETRY_MS)
}

pub(crate) fn wait_for_privileged_worker_socket_with_timing(
    socket_path: &Path,
    timeout: Duration,
    retry_interval: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;

    while Instant::now() < deadline {
        match UnixStream::connect(socket_path) {
            Ok(_) => return Ok(()),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::WouldBlock
                ) =>
            {
                last_error = Some(err);
                std::thread::sleep(retry_interval);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "privileged_worker_socket_not_ready: failed to connect to {}",
                        socket_path.display()
                    )
                });
            }
        }
    }

    let last_error = last_error.map(|err| err.to_string());
    let last_error_suffix = last_error
        .as_ref()
        .map(|err| format!("; last_error={err}"))
        .unwrap_or_default();

    Err(PrivilegedWorkerError::SocketNotReady {
        socket_path: socket_path.to_path_buf(),
        timeout_ms: timeout.as_millis(),
        last_error,
        last_error_suffix,
    }
    .into())
}

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
    prepare_privileged_worker_socket_path(socket_path)?;
    let listener = UnixListener::bind(socket_path).with_context(|| {
        format!(
            "failed to bind privileged worker socket {}",
            socket_path.display()
        )
    })?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600)).with_context(|| {
        format!(
            "failed to set permissions on privileged worker socket {}",
            socket_path.display()
        )
    })?;

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
        match handle_privileged_worker_connection(stream, service) {
            Ok(true) => break Ok(()),
            Ok(false) => {}
            Err(err) => {
                log::warn!("privileged_worker_connection_failed err={err:#}");
            }
        }
    };

    let cleanup_result = fs::remove_file(socket_path).with_context(|| {
        format!(
            "failed to remove privileged worker socket {}",
            socket_path.display()
        )
    });
    result.and(cleanup_result)
}

fn prepare_privileged_worker_socket_path(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create privileged worker socket directory {}",
                parent.display()
            )
        })?;
    }

    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to inspect existing privileged worker socket {}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_socket() {
        return Err(PrivilegedWorkerError::RefusingNonSocket {
            path: path.to_path_buf(),
        }
        .into());
    }

    fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove stale privileged worker socket {}",
            path.display()
        )
    })
}

fn handle_privileged_worker_connection(
    mut stream: UnixStream,
    service: &dyn PrivilegedActionService,
) -> anyhow::Result<bool> {
    let request = match read_privileged_worker_request(&stream) {
        Ok(request) => request,
        Err(err) => {
            let response = PrivilegedWorkerResponse::from_error(err);
            serde_json::to_writer(&mut stream, &response)?;
            stream.write_all(b"\n")?;
            stream.flush()?;
            return Ok(false);
        }
    };
    let shutdown = matches!(request, PrivilegedWorkerRequest::Shutdown);
    let response = execute_privileged_worker_request(request, service);
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
