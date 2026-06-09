//! Unix socket privileged action service client.

use super::*;

fn unexpected_response_error(response: PrivilegedWorkerResponse) -> anyhow::Error {
    PrivilegedWorkerError::UnexpectedResponse {
        response: Box::new(response),
    }
    .into()
}

#[derive(Clone, Debug)]
pub struct UnixSocketPrivilegedActionService {
    socket_path: PathBuf,
}

impl UnixSocketPrivilegedActionService {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    fn call_worker(
        &self,
        request: PrivilegedWorkerRequest,
    ) -> anyhow::Result<PrivilegedWorkerResponse> {
        let mut stream = UnixStream::connect(self.socket_path()).with_context(|| {
            format!(
                "failed to connect to privileged worker socket {}",
                self.socket_path().display()
            )
        })?;
        serde_json::to_writer(&mut stream, &request)?;
        stream.write_all(b"\n")?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).with_context(|| {
            format!(
                "failed to read privileged worker response from {}",
                self.socket_path().display()
            )
        })?;
        if line.trim().is_empty() {
            return Err(PrivilegedWorkerError::EmptyResponse {
                socket_path: self.socket_path().to_path_buf(),
            }
            .into());
        }

        serde_json::from_str(&line).with_context(|| {
            format!(
                "failed to decode privileged worker response from {}",
                self.socket_path.display()
            )
        })
    }

    pub fn request_shutdown(&self) -> anyhow::Result<()> {
        match self.call_worker(PrivilegedWorkerRequest::Shutdown)? {
            PrivilegedWorkerResponse::Shutdown { .. } => Ok(()),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => Err(unexpected_response_error(other)),
        }
    }

    #[cfg(test)]
    pub fn request_shutdown_for_tests(&self) -> anyhow::Result<()> {
        self.request_shutdown()
    }
}

impl PrivilegedActionService for UnixSocketPrivilegedActionService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord> {
        let response = self.call_worker(PrivilegedWorkerRequest::DryRun {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(&request.plan),
            policy: request.policy,
            context: request.context,
            max_plan_age_nanos: request.max_plan_age_nanos,
        })?;
        match response {
            PrivilegedWorkerResponse::DryRun { record } => Ok(record),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => Err(unexpected_response_error(other)),
        }
    }

    fn apply_candidate(&self, request: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        let response = self.call_worker(PrivilegedWorkerRequest::Apply {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(&request.plan),
            policy: request.policy,
            context: request.context,
            max_plan_age_nanos: request.max_plan_age_nanos,
        })?;
        match response {
            PrivilegedWorkerResponse::Apply { result } => Ok(result),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => Err(unexpected_response_error(other)),
        }
    }

    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult> {
        let response = self.call_worker(PrivilegedWorkerRequest::Rollback {
            plan: PrivilegedWorkerCandidatePlan::from_plan_request(
                &CandidatePlanRequest::from_candidate(
                    request.candidate,
                    crate::audit::unix_nanos_now(),
                ),
            ),
            token: request.token,
            policy: request.policy,
            context: request.context,
        })?;
        match response {
            PrivilegedWorkerResponse::Rollback { result } => Ok(result),
            response @ PrivilegedWorkerResponse::Error { .. } => Err(response.into_error()),
            other => Err(unexpected_response_error(other)),
        }
    }
}
