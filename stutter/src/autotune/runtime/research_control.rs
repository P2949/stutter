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

use crate::autotune::research::{AuthenticatedRequest, ResearchRequest, receive_request};

const SOCKET_NAME: &str = "research.sock";
const MAX_CONNECTIONS: usize = 8;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct Envelope {
    pub request: AuthenticatedRequest,
    pub reply: oneshot::Sender<&'static str>,
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

/// Runs only on the resident controller loop. No submission is acknowledged until durable
/// admission, source/host verification, and campaign lease handling are connected.
pub(super) fn respond(envelope: Envelope) {
    let response = if envelope.request.revalidate_peer().is_err() {
        "{\"status\":\"denied\",\"code\":\"PeerChanged\"}\n"
    } else {
        match envelope.request.request() {
            ResearchRequest::Status {} => {
                "{\"status\":\"idle\",\"admission\":\"BlockedContract\"}\n"
            }
            ResearchRequest::Submit { .. } => {
                "{\"status\":\"blocked\",\"code\":\"BlockedContract\"}\n"
            }
            ResearchRequest::Pause {}
            | ResearchRequest::Resume {}
            | ResearchRequest::Cancel {}
            | ResearchRequest::Report {}
            | ResearchRequest::Verify {} => "{\"status\":\"denied\",\"code\":\"NoCampaign\"}\n",
        }
    };
    let _ = envelope.reply.send(response);
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    #[tokio::test]
    async fn resident_transport_authenticates_and_refuses_unimplemented_admission() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (tx, mut rx) = mpsc::channel(4);
        let server = ResearchServer::bind(dir.path(), tx.clone()).await.unwrap();
        assert!(ResearchServer::bind(dir.path(), tx).await.is_err());
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
        respond(envelope);
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).await.unwrap();
        assert_eq!(
            line,
            "{\"status\":\"idle\",\"admission\":\"BlockedContract\"}\n"
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
        respond(envelope);
        let mut response = String::new();
        BufReader::new(submit)
            .read_line(&mut response)
            .await
            .unwrap();
        assert_eq!(
            response,
            "{\"status\":\"blocked\",\"code\":\"BlockedContract\"}\n"
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
        respond(envelope);
        cli.await.unwrap().unwrap();
        drop(server);
        assert!(!dir.path().join(SOCKET_NAME).exists());
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
