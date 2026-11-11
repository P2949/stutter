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
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    #[tokio::test]
    async fn resident_transport_authenticates_and_blocks_without_independent_host_facts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        let server = ResearchServer::bind(dir.path(), tx.clone()).await.unwrap();
        assert!(ResearchServer::bind(dir.path(), tx).await.is_err());
        let campaign_dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(campaign_dir.path(), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        // A client that sends no complete request must not block other control traffic.
        let slow = tokio::net::UnixStream::connect(dir.path().join(SOCKET_NAME))
            .await
            .unwrap();
        let mut client = tokio::net::UnixStream::connect(dir.path().join(SOCKET_NAME))
            .await
            .unwrap();
        client.write_all(b"{\"verb\":\"status\"}\n").await.unwrap();
        let envelope = tokio::time::timeout(RESPONSE_TIMEOUT, rx.recv())
            .await
            .unwrap()
            .unwrap();
        let mut coordinator = ResearchCoordinator::new(
            Some(campaign_dir.path().to_path_buf()),
            None,
            "BlockedHostIdentity",
        );
        coordinator.handle(envelope, 100);
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).await.unwrap();
        assert_eq!(
            line,
            "{\"code\":\"BlockedHostIdentity\",\"status\":\"idle\"}\n"
        );
        drop(slow);
        let mut submit = tokio::net::UnixStream::connect(dir.path().join(SOCKET_NAME))
            .await
            .unwrap();
        let intent = serde_json::json!({
            "schema_version": 1, "game": "cyberpunk2077", "app_id": 1091500,
            "uid": crate::syscall::geteuid(), "boot_id": "boot-1", "session_id": "11",
            "source_sha256": "a".repeat(64), "policy_sha256": "b".repeat(64),
            "game_executable_sha256": "c".repeat(64), "build_id": 1,
            "tools": [{"source":"ge", "identity_sha256":"d".repeat(64)}],
            "factors": [], "actions": [], "issued_unix_seconds": 10,
            "deadline_unix_seconds": 3610, "max_active_seconds": 3600,
            "max_attempt_seconds": 720, "max_owned_pressure_seconds_per_study": 600,
            "max_attempts": 10, "max_evidence_bytes": 1024,
            "desktop_research_consented": true
        });
        submit
            .write_all(
                format!(
                    "{}\n",
                    serde_json::json!({"verb":"submit", "intent":intent})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let envelope = tokio::time::timeout(RESPONSE_TIMEOUT, rx.recv())
            .await
            .unwrap()
            .unwrap();
        coordinator.handle(envelope, 100);
        let mut response = String::new();
        BufReader::new(submit)
            .read_line(&mut response)
            .await
            .unwrap();
        assert_eq!(
            response,
            "{\"code\":\"BlockedHostIdentity\",\"status\":\"blocked\"}\n"
        );
        let socket_dir = dir.path().to_path_buf();
        let cli = tokio::spawn(async move {
            crate::commands::research::run(crate::commands::research::ResearchClientInput {
                socket_dir: Some(socket_dir),
                action: crate::commands::research::ResearchClientAction::Status,
            })
            .await
        });
        let envelope = tokio::time::timeout(RESPONSE_TIMEOUT, rx.recv())
            .await
            .unwrap()
            .unwrap();
        coordinator.handle(envelope, 100);
        cli.await.unwrap().unwrap();
        drop(server);
        assert!(!dir.path().join(SOCKET_NAME).exists());
    }

    fn request(message: &ResearchRequest) -> AuthenticatedRequest {
        let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
        serde_json::to_writer(&mut client, message).unwrap();
        client.write_all(b"\n").unwrap();
        receive_request(&server, crate::syscall::geteuid()).unwrap()
    }

    fn fixture() -> (crate::autotune::research::ResearchIntentV1, CampaignFacts) {
        use crate::autotune::research::{
            ProtonSource, ResearchGame, ResearchIntentV1, ResearchTool,
        };

        let intent = ResearchIntentV1 {
            schema_version: 1,
            game: ResearchGame::Cyberpunk2077,
            app_id: 1_091_500,
            uid: crate::syscall::geteuid(),
            boot_id: "boot-1".into(),
            session_id: "11".into(),
            source_sha256: "a".repeat(64),
            policy_sha256: "b".repeat(64),
            game_executable_sha256: "c".repeat(64),
            build_id: 1,
            tools: vec![ResearchTool {
                source: ProtonSource::Ge,
                identity_sha256: "d".repeat(64),
            }],
            factors: vec![],
            actions: vec![],
            issued_unix_seconds: 10,
            deadline_unix_seconds: 3_610,
            max_active_seconds: 3_600,
            max_attempt_seconds: 720,
            max_owned_pressure_seconds_per_study: 600,
            max_attempts: 10,
            max_evidence_bytes: 1_024,
            desktop_research_consented: true,
        };
        let facts = CampaignFacts {
            uid: intent.uid,
            boot_id: intent.boot_id.clone(),
            session_id: intent.session_id.clone(),
            source_sha256: intent.source_sha256.clone(),
            policy_sha256: intent.policy_sha256.clone(),
            game_executable_sha256: intent.game_executable_sha256.clone(),
            build_id: intent.build_id,
            tools: intent.tools.clone(),
            validation_override_present: false,
            mutable_action_present: false,
        };
        (intent, facts)
    }

    fn send(coordinator: &mut ResearchCoordinator, message: ResearchRequest, now: u64) -> String {
        let (reply, answer) = oneshot::channel();
        coordinator.handle(
            Envelope {
                request: request(&message),
                reply,
            },
            now,
        );
        answer.blocking_recv().unwrap()
    }

    #[test]
    fn one_verified_intent_is_durable_pausable_cancellable_and_terminal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (intent, facts) = fixture();
        let mut coordinator = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts.clone()),
            "BlockedHostIdentity",
        );

        let accepted = send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent.clone()),
            },
            100,
        );
        let accepted: serde_json::Value = serde_json::from_str(&accepted).unwrap();
        assert_eq!(accepted["status"], "ok");
        assert_eq!(accepted["phase"], "await_safe_window");

        let second = send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            101,
        );
        assert!(second.contains("SingleCampaignLimit"));

        let paused = send(&mut coordinator, ResearchRequest::Pause {}, 101);
        assert!(paused.contains("\"paused\""));
        let resumed = send(&mut coordinator, ResearchRequest::Resume {}, 102);
        assert!(resumed.contains("\"await_safe_window\""));
        let cancelled = send(&mut coordinator, ResearchRequest::Cancel {}, 103);
        assert!(cancelled.contains("\"aborted\""));
        let status = send(&mut coordinator, ResearchRequest::Status {}, 103);
        assert!(status.contains("\"aborted\""));

        drop(coordinator);
        let recovered = CampaignLease::recover(
            crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
            &facts,
            103,
        )
        .unwrap();
        assert_eq!(recovered.phase(), ResearchPhase::Aborted);
        drop(recovered);

        let mut restarted = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts),
            "BlockedHostIdentity",
        );
        let status = send(&mut restarted, ResearchRequest::Status {}, 103);
        assert!(status.contains("UnsafeToResume"));
    }

    #[test]
    fn intent_without_verified_facts_never_creates_campaign_records() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (intent, _) = fixture();
        let mut coordinator = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            None,
            "BlockedSourceIdentity",
        );
        let response = send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        assert!(response.contains("BlockedSourceIdentity"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn public_campaign_directory_is_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let (intent, facts) = fixture();
        let mut coordinator = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts),
            "BlockedHostIdentity",
        );
        let result = send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        assert!(result.contains("BlockedResearchStorage"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn independent_controllers_cannot_own_the_same_campaign_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (intent, facts) = fixture();
        let mut first = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts.clone()),
            "BlockedHostIdentity",
        );
        let mut second = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts),
            "BlockedHostIdentity",
        );

        let admitted = send(
            &mut first,
            ResearchRequest::Submit {
                intent: Box::new(intent.clone()),
            },
            100,
        );
        assert!(admitted.contains("await_safe_window"));
        let refused = send(
            &mut second,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        assert!(refused.contains("CampaignAlreadyOwned"));
    }

    #[test]
    fn cancellation_during_an_unsettled_launch_does_not_claim_restoration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (intent, facts) = fixture();
        let digest = intent.digest().unwrap();
        let mut lease = CampaignLease::create(
            crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
            intent,
            facts.clone(),
            100,
            std::process::id(),
            1,
        )
        .unwrap();
        for phase in [
            ResearchPhase::Preflight,
            ResearchPhase::Prepared,
            ResearchPhase::AwaitSafeWindow,
            ResearchPhase::Launching,
        ] {
            lease.advance(phase, 100).unwrap();
        }
        let mut coordinator = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts.clone()),
            "BlockedHostIdentity",
        );
        coordinator.active = Some(ActiveCampaign {
            lease,
            intent_sha256: digest,
        });
        coordinator.submitted = true;
        let result = send(&mut coordinator, ResearchRequest::Cancel {}, 101);
        assert!(result.contains("CancellationNeedsExecutorSettlement"));
        assert_eq!(
            coordinator.active.as_ref().unwrap().lease.phase(),
            ResearchPhase::Launching
        );
        drop(coordinator);
        assert!(
            CampaignLease::recover(
                crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
                &facts,
                101,
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn cleanup_preserves_a_replacement_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (tx, _rx) = mpsc::channel(1);
        let server = ResearchServer::bind(dir.path(), tx).await.unwrap();
        let path = dir.path().join(SOCKET_NAME);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"keep").unwrap();
        drop(server);
        assert_eq!(std::fs::read(path).unwrap(), b"keep");
    }

    #[tokio::test]
    async fn cleanup_stays_with_the_opened_directory_after_rename() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("control");
        let moved = root.path().join("moved");
        std::fs::create_dir(&original).unwrap();
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();
        let (tx, _rx) = mpsc::channel(1);
        let server = ResearchServer::bind(&original, tx).await.unwrap();
        std::fs::rename(&original, &moved).unwrap();
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join(SOCKET_NAME), b"keep").unwrap();
        drop(server);
        assert_eq!(std::fs::read(original.join(SOCKET_NAME)).unwrap(), b"keep");
        assert!(!moved.join(SOCKET_NAME).exists());
    }

    #[tokio::test]
    async fn stale_socket_is_recovered_but_a_live_or_non_socket_path_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join(SOCKET_NAME);
        let old = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (tx, _rx) = mpsc::channel(1);
        assert!(ResearchServer::bind(dir.path(), tx.clone()).await.is_err());
        assert!(path.exists());
        drop(old);
        let server = ResearchServer::bind(dir.path(), tx.clone()).await.unwrap();
        drop(server);
        std::fs::write(&path, b"keep").unwrap();
        assert!(ResearchServer::bind(dir.path(), tx).await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"keep");
    }
}
