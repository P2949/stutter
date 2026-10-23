//! The runner service. Policy lives here: peer credentials and the root-owned authorization
//! gate every request; durable state is written before any reboot; corrupt state stops every
//! action that could reboot. Mechanical steps are delegated to `ActionExecutor`/`BootControl`.
use std::{
    fs,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};

use crate::{
    boot::BootControl,
    model::{Authorization, Envelope, ErrorCode, Request, Response, RunnerAction, SCHEMA_VERSION},
    process::ActionExecutor,
    protocol::{read_frame, write_frame},
    state::{RunnerState, StateDir, StateError, unix_ms},
    sys::{PeerCredentials, peer_credentials},
};

pub struct Server {
    pub authorization: Authorization,
    pub state: StateDir,
    executor: Arc<dyn ActionExecutor>,
    boot: Arc<dyn BootControl>,
    online_cpus: u32,
    /// One exclusive action (suite, burst, boot step) at a time.
    busy: Mutex<()>,
}

fn online_cpus() -> u32 {
    std::thread::available_parallelism().map_or(1, |n| n.get() as u32)
}

impl Server {
    pub fn new(
        authorization: Authorization,
        state: StateDir,
        executor: Arc<dyn ActionExecutor>,
        boot: Arc<dyn BootControl>,
    ) -> Self {
        Self {
            authorization,
            state,
            executor,
            boot,
            online_cpus: online_cpus(),
            busy: Mutex::new(()),
        }
    }

    /// Handles one authenticated request; every outcome, including rejections, is receipted.
    pub fn handle(&self, peer: PeerCredentials, envelope: Envelope) -> Response {
        let request_id = envelope.request_id.clone();
        let response = match self
            .authorization
            .authorize(&peer, &envelope, self.online_cpus)
        {
            Err((code, message)) => Response::rejected(&request_id, code, message),
            Ok(()) => self.dispatch(&envelope),
        };
        self.receipt(&peer, &envelope, response)
    }

    fn receipt(
        &self,
        peer: &PeerCredentials,
        envelope: &Envelope,
        mut response: Response,
    ) -> Response {
        // Request ids are validated safe identifiers only after authorization; an invalid one
        // from a rejected request must never become a path.
        if !crate::authorization::is_safe_identifier(&envelope.request_id) {
            return response;
        }
        let body = json!({
            "schema_version": SCHEMA_VERSION,
            "request_id": envelope.request_id,
            "task_id": envelope.task_id,
            "action": envelope.body.action(),
            "peer_uid": peer.uid,
            "peer_pid": peer.pid,
            "status": response.status,
            "error": response.error,
            "unix_ms": unix_ms(),
        });
        if let Ok(id) = self.state.write_receipt(&envelope.request_id, &body) {
            response.receipt = Some(id);
        }
        response
    }

    fn dispatch(&self, envelope: &Envelope) -> Response {
        let id = envelope.request_id.as_str();
        let action = envelope.body.action();
        let mut state = match self.state.load(&self.authorization.campaign_id) {
            Ok(state) => state,
            // Corrupt state stops everything that could act; it is reported, never repaired.
            Err(error @ (StateError::Corrupt(_) | StateError::Io(_))) => {
                return Response::failed(id, ErrorCode::CorruptState, error.to_string());
            }
        };
        if state.cancelled && !matches!(action, RunnerAction::Status | RunnerAction::CancelCampaign)
        {
            return Response::rejected(id, ErrorCode::Cancelled, "the campaign was cancelled");
        }
        match &envelope.body {
            Request::Status => Response::ok(
                id,
                None,
                status_data(&self.authorization, &state, self.boot.describe()),
            ),
            Request::CancelCampaign => {
                state.cancelled = true;
                state.resume_pending = false;
                state.pending_boot_entry = None;
                // A cancelled campaign must not leave a one-shot boot selection behind.
                let restored = self.boot.restore_default();
                let data = json!({"cancelled": true, "boot_restored": restored.is_ok()});
                self.persist(id, &state, data)
            }
            _ => {
                let Ok(_guard) = self.busy.try_lock() else {
                    return Response::rejected(
                        id,
                        ErrorCode::Busy,
                        "another privileged action is running",
                    );
                };
                self.exclusive(envelope, state)
            }
        }
    }

    fn persist(&self, id: &str, state: &RunnerState, data: Value) -> Response {
        match self.state.save(state) {
            Ok(()) => Response::ok(id, None, data),
            Err(error) => Response::failed(
                id,
                ErrorCode::CorruptState,
                format!("could not persist state: {error}"),
            ),
        }
    }

    fn exclusive(&self, envelope: &Envelope, mut state: RunnerState) -> Response {
        let id = envelope.request_id.as_str();
        let from_denial =
            |(code, message): (ErrorCode, String)| Response::failed(id, code, message);
        match &envelope.body {
            Request::ExecuteLiveSuite { .. } => self
                .executor
                .live_suite(&self.authorization, envelope)
                .map_or_else(from_denial, |data| Response::ok(id, None, data)),
            Request::ExecuteSchedulerBurst { .. } => self
                .executor
                .scheduler_burst(&self.authorization, envelope)
                .map_or_else(from_denial, |data| Response::ok(id, None, data)),
            Request::ExecuteRecording { .. } => self
                .executor
                .recording(&self.authorization, envelope)
                .map_or_else(from_denial, |data| Response::ok(id, None, data)),
            Request::ExecuteWorkloadOverhead { .. } => self
                .executor
                .workload_overhead(&self.authorization, envelope)
                .map_or_else(from_denial, |data| Response::ok(id, None, data)),
            Request::SetNextBoot { entry_id } | Request::PrepareKernelCycle { entry_id } => {
                match self.boot.set_next_boot(entry_id) {
                    Ok(()) => {
                        state.pending_boot_entry = Some(entry_id.clone());
                        self.persist(id, &state, json!({"next_boot": entry_id}))
                    }
                    Err(message) => Response::failed(id, ErrorCode::Unsupported, message),
                }
            }
            Request::Reboot => {
                if state.reboots_used >= self.authorization.max_reboots {
                    return Response::rejected(
                        id,
                        ErrorCode::RebootBudgetExhausted,
                        "the reboot budget is spent",
                    );
                }
                // Durable first: the budget and the resume flag survive the reboot even if
                // the process dies before it returns.
                state.reboots_used += 1;
                state.resume_pending = true;
                if let Err(error) = self.state.save(&state) {
                    return Response::failed(
                        id,
                        ErrorCode::CorruptState,
                        format!("not rebooting: {error}"),
                    );
                }
                match self.boot.reboot() {
                    Ok(()) => Response::ok(
                        id,
                        None,
                        json!({"rebooting": true, "reboots_used": state.reboots_used}),
                    ),
                    Err(message) => {
                        state.reboots_used -= 1;
                        state.resume_pending = false;
                        let _ = self.state.save(&state);
                        Response::failed(id, ErrorCode::Unsupported, message)
                    }
                }
            }
            Request::ResumeCampaign => {
                state.resume_pending = false;
                state.pending_boot_entry = None;
                self.persist(id, &state, json!({"resumed": true}))
            }
            Request::Status | Request::CancelCampaign => {
                Response::failed(id, ErrorCode::Unsupported, "unreachable")
            }
        }
    }

    /// Serves on `socket` until the process ends. The socket is owned by the campaign user
    /// with mode 0600, and every request is additionally checked against `SO_PEERCRED`.
    pub fn serve(self: Arc<Self>, socket: &Path) -> std::io::Result<()> {
        if socket.exists() {
            fs::remove_file(socket)?;
        }
        let listener = UnixListener::bind(socket)?;
        fs::set_permissions(socket, fs::Permissions::from_mode(0o600))?;
        if crate::sys::effective_uid() == 0 {
            std::os::unix::fs::chown(
                socket,
                Some(self.authorization.owner_uid),
                Some(self.authorization.owner_gid),
            )?;
        }
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let server = Arc::clone(&self);
            std::thread::spawn(move || server.serve_connection(stream));
        }
        Ok(())
    }

    pub fn serve_connection(&self, mut stream: UnixStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let Ok(peer) = peer_credentials(&stream) else {
            return;
        };
        let envelope: Envelope = match read_frame(&mut stream) {
            Ok(envelope) => envelope,
            Err(error) => {
                let response = Response::rejected(
                    "unknown",
                    ErrorCode::InvalidArgument,
                    format!("unreadable request: {error}"),
                );
                let _ = write_frame(&mut stream, &response);
                return;
            }
        };
        let response = self.handle(peer, envelope);
        // Long suites outlive the read timeout; the response write is not time-limited.
        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
        let _ = write_frame(&mut stream, &response);
    }
}

fn status_data(authorization: &Authorization, state: &RunnerState, boot: Value) -> Value {
    json!({
        "campaign_id": authorization.campaign_id,
        "reboots_used": state.reboots_used,
        "max_reboots": authorization.max_reboots,
        "cancelled": state.cancelled,
        "resume_pending": state.resume_pending,
        "pending_boot_entry": state.pending_boot_entry,
        "grants": authorization.grants.len(),
        "allowed_boot_entries": authorization.allowed_boot_entries,
        "original_boot_entry": authorization.original_boot_entry,
        "boot": boot,
    })
}
