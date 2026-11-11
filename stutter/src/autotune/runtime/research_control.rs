//! Optional local research-control transport. It has no worker or launcher authority.
//!
//! Connections are bounded and parsed away from monitor ticks. The resident loop owns request
//! decisions. Until campaign admission and recovery are implemented, it refuses submission.
use std::{
    fs::File,
    io::Write,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tokio::{
    net::UnixListener,
    sync::{Semaphore, mpsc, oneshot},
    task::JoinHandle,
};

use crate::autotune::research::{
    AuthenticatedRequest, CampaignFacts, CampaignLease, ResearchPhase, ResearchRequest,
    receive_request,
};

const SOCKET_NAME: &str = "research.sock";
const MAX_CONNECTIONS: usize = 8;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct Envelope {
    pub request: AuthenticatedRequest,
    pub reply: oneshot::Sender<String>,
}

struct ActiveCampaign {
    lease: CampaignLease,
    intent_sha256: String,
}

struct TerminalCampaign {
    phase: ResearchPhase,
    intent_sha256: String,
}

/// Controller-owned admission and management state. The production controller supplies host
/// facts only after independently verifying source, policy, Steam installation and tool identity.
/// An absent context deliberately keeps submit fail-closed while status remains available.
pub(super) struct ResearchCoordinator {
    campaign_dir: Option<PathBuf>,
    storage_blocker: Option<&'static str>,
    facts: Option<CampaignFacts>,
    context_blocker: &'static str,
    active: Option<ActiveCampaign>,
    terminal: Option<TerminalCampaign>,
    submitted: bool,
}

impl ResearchCoordinator {
    pub(super) fn new(
        campaign_dir: Option<PathBuf>,
        facts: Option<CampaignFacts>,
        context_blocker: &'static str,
    ) -> Self {
        let storage_blocker = match campaign_dir.as_deref() {
            Some(dir) => campaign_storage_blocker(dir),
            None => Some("BlockedResearchStorage"),
        };
        Self {
            campaign_dir,
            storage_blocker,
            facts,
            context_blocker,
            active: None,
            terminal: None,
            submitted: false,
        }
    }

    pub(super) fn handle(&mut self, envelope: Envelope, now: u64) {
        let response = if envelope.request.revalidate_peer().is_err() {
            response("denied", "PeerChanged")
        } else {
            match envelope.request.request() {
                ResearchRequest::Status {} => self.status(),
                ResearchRequest::Submit { intent } => self.submit(
                    intent,
                    envelope.request.uid(),
                    envelope.request.pid(),
                    envelope.request.start_ticks(),
                    now,
                ),
                ResearchRequest::Pause {} => self.pause(now),
                ResearchRequest::Resume {} => self.resume(now),
                ResearchRequest::Cancel {} => self.cancel(now),
                ResearchRequest::Report {} => response("blocked", "NoReport"),
                ResearchRequest::Verify {} => response("blocked", "NoReportToVerify"),
            }
        };
        let _ = envelope.reply.send(response);
    }

    fn status(&self) -> String {
        if let Some(active) = &self.active {
            return serde_json::json!({
                "status": "ok",
                "phase": active.lease.phase(),
                "intent_sha256": active.intent_sha256,
            })
            .to_string()
                + "\n";
        }
        if let Some(terminal) = &self.terminal {
            return serde_json::json!({
                "status": "ok",
                "phase": terminal.phase,
                "intent_sha256": terminal.intent_sha256,
            })
            .to_string()
                + "\n";
        }
        if self.submitted {
            return response("blocked", "CampaignTerminal");
        }
        if let Some(code) = self.storage_blocker {
            return response("blocked", code);
        }
        response("idle", self.context_blocker)
    }

    fn submit(
        &mut self,
        intent: &crate::autotune::research::ResearchIntentV1,
        peer_uid: u32,
        peer_pid: u32,
        peer_start_ticks: u64,
        now: u64,
    ) -> String {
        if self.submitted {
            return response("denied", "SingleCampaignLimit");
        }
        if let Some(code) = self.storage_blocker {
            return response("blocked", code);
        }
        let Some(dir) = self.campaign_dir.as_deref() else {
            return response("blocked", "BlockedResearchStorage");
        };
        let Some(facts) = self.facts.as_ref() else {
            return response("blocked", self.context_blocker);
        };
        if intent.uid != peer_uid {
            return response("denied", "WrongUser");
        }
        let digest = match intent.digest() {
            Ok(value) => value,
            Err(_) => return response("denied", "InvalidIntent"),
        };
        let opened = match crate::syscall::open_dir_no_symlinks(dir) {
            Ok(value) => value,
            Err(_) => return response("blocked", "BlockedResearchStorage"),
        };
        let lease = match CampaignLease::create(
            opened,
            intent.clone(),
            facts.clone(),
            now,
            peer_pid,
            peer_start_ticks,
        ) {
            Ok(value) => value,
            Err(error) => {
                let detail = format!("{error:#}");
                let code = if detail.contains("not empty") {
                    "UnsafeToResume"
                } else if detail.contains("locked") {
                    "CampaignAlreadyOwned"
                } else {
                    "AdmissionRejected"
                };
                return response("blocked", code);
            }
        };
        self.submitted = true;
        self.active = Some(ActiveCampaign {
            lease,
            intent_sha256: digest,
        });
        let Some(active) = self.active.as_mut() else {
            return response("blocked", "InternalStateError");
        };
        for phase in [
            ResearchPhase::Preflight,
            ResearchPhase::Prepared,
            ResearchPhase::AwaitSafeWindow,
        ] {
            if active.lease.advance(phase, now).is_err() {
                return self.block_after_admission(now, "AdmissionPersistenceFailed");
            }
        }
        self.status()
    }

    fn pause(&mut self, now: u64) -> String {
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if active.lease.phase() == ResearchPhase::Paused {
            return self.status();
        }
        if active.lease.phase() != ResearchPhase::AwaitSafeWindow {
            return response("blocked", "CannotPauseActiveStage");
        }
        if active.lease.advance(ResearchPhase::Paused, now).is_err() {
            return response("blocked", "PausePersistenceFailed");
        }
        self.status()
    }

    fn resume(&mut self, now: u64) -> String {
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if active.lease.phase() == ResearchPhase::AwaitSafeWindow {
            return self.status();
        }
        if active.lease.phase() != ResearchPhase::Paused {
            return response("blocked", "CannotResumeActiveStage");
        }
        if active
            .lease
            .advance(ResearchPhase::AwaitSafeWindow, now)
            .is_err()
        {
            return response("blocked", "ResumePersistenceFailed");
        }
        self.status()
    }

    fn cancel(&mut self, now: u64) -> String {
        let Some(active) = self.active.as_mut() else {
            return response("denied", "NoCampaign");
        };
        if matches!(
            active.lease.phase(),
            ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
        ) {
            return self.status();
        }
        if !matches!(
            active.lease.phase(),
            ResearchPhase::Requested
                | ResearchPhase::Preflight
                | ResearchPhase::Prepared
                | ResearchPhase::AwaitSafeWindow
                | ResearchPhase::Paused
        ) {
            return response("blocked", "CancellationNeedsExecutorSettlement");
        }
        if active.lease.advance(ResearchPhase::Restoring, now).is_err()
            || active.lease.advance(ResearchPhase::Aborted, now).is_err()
        {
            return response("blocked", "CancellationPersistenceFailed");
        }
        self.terminal = Some(TerminalCampaign {
            phase: ResearchPhase::Aborted,
            intent_sha256: active.intent_sha256.clone(),
        });
        self.active = None;
        self.status()
    }

    fn block_after_admission(&mut self, now: u64, code: &'static str) -> String {
        if let Some(active) = self.active.as_mut() {
            let phase = active.lease.phase();
            if !matches!(
                phase,
                ResearchPhase::Completed | ResearchPhase::Blocked | ResearchPhase::Aborted
            ) && active.lease.advance(ResearchPhase::Restoring, now).is_ok()
            {
                let _ = active.lease.advance(ResearchPhase::Blocked, now);
            }
            return response("blocked", code);
        }
        response("blocked", code)
    }
}

fn campaign_storage_blocker(dir: &Path) -> Option<&'static str> {
    let Ok(opened) = crate::syscall::open_dir_no_symlinks(dir) else {
        return Some("BlockedResearchStorage");
    };
    let file = File::from(opened);
    let Ok(metadata) = file.metadata() else {
        return Some("BlockedResearchStorage");
    };
    if !metadata.is_dir()
        || metadata.uid() != crate::syscall::geteuid()
        || metadata.mode() & 0o077 != 0
    {
        return Some("BlockedResearchStorage");
    }
    match std::fs::read_dir(format!("/proc/self/fd/{}", file.as_raw_fd())) {
        Ok(mut entries) => match entries.next() {
            None => None,
            Some(Ok(_)) => Some("UnsafeToResume"),
            Some(Err(_)) => Some("BlockedResearchStorage"),
        },
        Err(_) => Some("BlockedResearchStorage"),
    }
}

fn response(status: &str, code: &str) -> String {
    serde_json::json!({ "status": status, "code": code }).to_string() + "\n"
}

pub(super) struct ResearchServer {
    task: JoinHandle<()>,
    // Drop the socket guard before closing the descriptor embedded in its anchored path.
    _socket: SocketGuard,
    _directory_lock: File,
}

struct SocketGuard {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Ok(meta) = std::fs::symlink_metadata(&self.path)
            && meta.file_type().is_socket()
            && meta.dev() == self.dev
            && meta.ino() == self.ino
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl ResearchServer {
    pub async fn bind(dir: &Path, sender: mpsc::Sender<Envelope>) -> anyhow::Result<Self> {
        let uid = crate::syscall::geteuid();
        anyhow::ensure!(uid != 0, "research control cannot run as root");
        let directory_lock = File::from(crate::syscall::open_dir_no_symlinks(dir)?);
        let dir_meta = directory_lock.metadata()?;
        anyhow::ensure!(
            dir_meta.is_dir() && dir_meta.uid() == uid && dir_meta.mode() & 0o077 == 0,
            "untrusted research socket directory"
        );
        anyhow::ensure!(
            crate::syscall::flock_exclusive_nonblocking(directory_lock.as_fd()),
            "research controller already owns this directory"
        );
        // All socket operations use the pinned directory, even if its path is renamed.
        let socket = PathBuf::from(format!(
            "/proc/self/fd/{}/{SOCKET_NAME}",
            directory_lock.as_raw_fd()
        ));
        if let Ok(old) = std::fs::symlink_metadata(&socket) {
            anyhow::ensure!(
                old.file_type().is_socket()
                    && old.uid() == uid
                    && old.nlink() == 1
                    && old.mode() & 0o077 == 0,
                "refusing to replace an untrusted research socket path"
            );
            match tokio::time::timeout(
                Duration::from_millis(250),
                tokio::net::UnixStream::connect(&socket),
            )
            .await
            {
                Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
                _ => anyhow::bail!("research socket is live or its state is unknown"),
            }
            let current = std::fs::symlink_metadata(&socket)?;
            anyhow::ensure!(
                current.file_type().is_socket()
                    && current.dev() == old.dev()
                    && current.ino() == old.ino(),
                "research socket changed during stale check"
            );
            std::fs::remove_file(&socket)?;
        }
        let listener = std::os::unix::net::UnixListener::bind(&socket)?;
        let meta = std::fs::symlink_metadata(&socket)?;
        anyhow::ensure!(
            meta.file_type().is_socket() && meta.uid() == uid && meta.nlink() == 1,
            "research socket identity mismatch"
        );
        let socket_guard = SocketGuard {
            path: socket,
            dev: meta.dev(),
            ino: meta.ino(),
        };
        crate::syscall::chmod_nofollow_at(directory_lock.as_fd(), c"research.sock", 0o600)?;
        listener.set_nonblocking(true)?;
        let current = std::fs::symlink_metadata(&socket_guard.path)?;
        anyhow::ensure!(
            current.file_type().is_socket()
                && current.dev() == socket_guard.dev
                && current.ino() == socket_guard.ino
                && current.mode() & 0o077 == 0,
            "research socket changed during binding"
        );
        let listener = UnixListener::from_std(listener)?;
        let task = tokio::spawn(accept_connections(listener, uid, sender));
        Ok(Self {
            task,
            _socket: socket_guard,
            _directory_lock: directory_lock,
        })
    }
}

impl Drop for ResearchServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept_connections(listener: UnixListener, uid: u32, sender: mpsc::Sender<Envelope>) {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                log::warn!("research control listener stopped: {error}");
                break;
            }
        };
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            continue;
        };
        let Ok(stream) = stream.into_std() else {
            continue;
        };
        if stream.set_nonblocking(false).is_err() {
            continue;
        }
        let sender = sender.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let Ok((mut stream, Ok(request))) = tokio::task::spawn_blocking(move || {
                let request = receive_request(&stream, uid);
                (stream, request)
            })
            .await
            else {
                return;
            };
            let (reply, answer) = oneshot::channel();
            if sender.try_send(Envelope { request, reply }).is_err() {
                return;
            }
            let Ok(Ok(response)) = tokio::time::timeout(RESPONSE_TIMEOUT, answer).await else {
                return;
            };
            let _ = tokio::task::spawn_blocking(move || {
                stream.set_write_timeout(Some(RESPONSE_TIMEOUT))?;
                stream.write_all(response.as_bytes())
            })
            .await;
        });
    }
}

#[cfg(test)]
#[path = "research_control_tests.rs"]
mod tests;
