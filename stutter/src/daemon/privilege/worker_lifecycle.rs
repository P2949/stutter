//! Managed worker process and socket lifecycle.
use std::{io::Read, os::unix::fs::MetadataExt};

use super::*;

#[derive(Debug)]
pub struct PrivilegedWorkerHandle {
    child: Child,
    socket_path: PathBuf,
    socket_identity: (u64, u64),
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
        prepare_privileged_worker_socket_path(socket_path)?;
        let (child, socket_identity) = spawn_ready_worker(
            executable,
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
            socket_identity,
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
        remove_owned_socket(&self.socket_path, self.socket_identity)
    }

    pub fn restart(&mut self) -> anyhow::Result<()> {
        self.terminate()?;
        prepare_privileged_worker_socket_path(&self.socket_path)?;
        let (child, socket_identity) = spawn_ready_worker(
            &self.executable_path,
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
        self.socket_identity = socket_identity;
        Ok(())
    }

    pub fn shutdown_gracefully(
        &mut self,
        timeout: Duration,
        poll_interval: Duration,
    ) -> anyhow::Result<()> {
        match socket_identity(&self.socket_path) {
            Ok(identity) if identity != self.socket_identity => {
                return Err(PrivilegedWorkerError::SocketChanged {
                    path: self.socket_path.clone(),
                }
                .into());
            }
            Ok(_) => {}
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
        if let Ok(mut stream) = UnixStream::connect(&self.socket_path) {
            require_socket_identity(&self.socket_path, self.socket_identity)?;
            let peer = super::peer::peer_credentials(&stream)?;
            require_worker_peer(peer.pid, self.child.id())?;
            stream.set_read_timeout(Some(timeout))?;
            serde_json::to_writer(&mut stream, &PrivilegedWorkerRequest::Shutdown)?;
            stream.write_all(b"\n")?;
            stream.flush()?;
            let mut line = String::new();
            BufReader::new(stream).take(8_193).read_line(&mut line)?;
            if line.len() > 8_192 || !line.ends_with('\n') {
                return Err(PrivilegedWorkerError::ShutdownResponseInvalid {
                    path: self.socket_path.clone(),
                }
                .into());
            }
            let response: PrivilegedWorkerResponse = serde_json::from_str(&line).map_err(|_| {
                PrivilegedWorkerError::ShutdownResponseInvalid {
                    path: self.socket_path.clone(),
                }
            })?;
            match response {
                PrivilegedWorkerResponse::Shutdown { .. } => {}
                response @ PrivilegedWorkerResponse::Error { .. } => {
                    return Err(response.into_error());
                }
                response => {
                    return Err(PrivilegedWorkerError::UnexpectedResponse {
                        response: Box::new(response),
                    }
                    .into());
                }
            }
        }

        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(PrivilegedWorkerError::ShutdownFailed {
                        status: status.to_string(),
                    }
                    .into())
                };
            }
            if Instant::now() >= deadline {
                self.child.kill().ok();
                self.child.wait().ok();
                return Err(PrivilegedWorkerError::ShutdownTimeout {
                    timeout_ms: timeout.as_millis(),
                }
                .into());
            }
            std::thread::sleep(poll_interval);
        }
    }
}

impl Drop for PrivilegedWorkerHandle {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        if let Err(error) = remove_owned_socket(&self.socket_path, self.socket_identity) {
            log::warn!("privileged worker socket cleanup refused: {error:#}");
        }
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

fn spawn_ready_worker(
    executable: &Path,
    socket_path: &Path,
    timeout: Duration,
    retry_interval: Duration,
) -> anyhow::Result<(Child, (u64, u64))> {
    let mut child = spawn_privileged_worker_child(executable, socket_path)?;
    let ready = (|| -> anyhow::Result<(u64, u64)> {
        wait_for_privileged_worker_socket_with_timing(socket_path, timeout, retry_interval)?;
        let identity = socket_identity(socket_path)?;
        let stream = UnixStream::connect(socket_path)?;
        let peer = super::peer::peer_credentials(&stream)?;
        require_worker_peer(peer.pid, child.id())?;
        require_socket_identity(socket_path, identity)?;
        Ok(identity)
    })();
    match ready {
        Ok(identity) => Ok((child, identity)),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

pub(super) fn socket_identity(path: &Path) -> anyhow::Result<(u64, u64)> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_socket() {
        return Err(PrivilegedWorkerError::RefusingNonSocket {
            path: path.to_owned(),
        }
        .into());
    }
    Ok((metadata.dev(), metadata.ino()))
}

fn require_socket_identity(path: &Path, expected: (u64, u64)) -> anyhow::Result<()> {
    if socket_identity(path)? != expected {
        return Err(PrivilegedWorkerError::SocketChanged {
            path: path.to_owned(),
        }
        .into());
    }
    Ok(())
}

fn require_worker_peer(actual: u32, expected: u32) -> anyhow::Result<()> {
    if actual != expected {
        return Err(PrivilegedWorkerError::SocketPeerMismatch { expected, actual }.into());
    }
    Ok(())
}

pub(super) fn remove_owned_socket(path: &Path, expected: (u64, u64)) -> anyhow::Result<()> {
    match socket_identity(path) {
        Ok(identity) if identity == expected => fs::remove_file(path).map_err(Into::into),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(())
        }
        _ => Err(PrivilegedWorkerError::SocketChanged {
            path: path.to_owned(),
        }
        .into()),
    }
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

pub(super) fn prepare_privileged_worker_socket_path(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create privileged worker socket directory {}",
                parent.display()
            )
        })?;
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect existing privileged worker socket {}",
                    path.display()
                )
            });
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(PrivilegedWorkerError::RefusingNonSocket {
            path: path.to_path_buf(),
        }
        .into());
    }

    match UnixStream::connect(path) {
        Ok(_) => {
            return Err(PrivilegedWorkerError::SocketInUse {
                path: path.to_owned(),
            }
            .into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "cannot verify existing privileged worker socket {}",
                    path.display()
                )
            });
        }
    }
    let current = fs::symlink_metadata(path)?;
    if !current.file_type().is_socket()
        || current.dev() != metadata.dev()
        || current.ino() != metadata.ino()
    {
        return Err(PrivilegedWorkerError::SocketChanged {
            path: path.to_owned(),
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
