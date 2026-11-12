//! Bounded, local research-control messages and kernel-authenticated peer identity.
//!
//! Parsing a request does not admit a campaign: the resident controller must separately check
//! source, game, boot, session, deadline, policy, lease and journal before taking any action.
//! Perform socket I/O away from the monitor tick; a client can take up to the read timeout.
use std::{
    io::{self, Read},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::ResearchIntentV1;

const MAX_REQUEST_BYTES: u64 = 16 * 1024;

/// Read one bounded frame with a wall-clock deadline, including slow incremental senders.
pub(crate) fn read_frame(
    stream: &UnixStream,
    limit: usize,
    timeout: Duration,
) -> anyhow::Result<Vec<u8>> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow::anyhow!("invalid frame deadline"))?;
    let mut line = Vec::new();
    let mut reader = stream;
    let mut buffer = [0_u8; 1024];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        anyhow::ensure!(!remaining.is_zero(), "frame deadline exceeded");
        stream.set_read_timeout(Some(remaining))?;
        let count = match reader.read(&mut buffer) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        anyhow::ensure!(Instant::now() <= deadline, "frame deadline exceeded");
        anyhow::ensure!(count > 0, "incomplete frame");
        anyhow::ensure!(
            line.len().saturating_add(count) <= limit,
            "frame exceeds size limit"
        );
        line.extend_from_slice(&buffer[..count]);
        if let Some(index) = line.iter().position(|byte| *byte == b'\n') {
            anyhow::ensure!(index + 1 == line.len(), "multiple frames on one request");
            return Ok(line);
        }
        anyhow::ensure!(line.len() < limit, "frame exceeds size limit");
    }
}

/// Only management verbs are accepted. This protocol never carries shell commands or paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResearchRequest {
    Submit { intent: Box<ResearchIntentV1> },
    Status {},
    Pause {},
    Resume {},
    Cancel {},
    Report {},
    Verify {},
}

/// Identity comes only from SO_PEERCRED and procfs, never from the message.
#[derive(Debug)]
pub struct AuthenticatedRequest {
    request: ResearchRequest,
    uid: u32,
    pid: u32,
    start_ticks: u64,
}

impl AuthenticatedRequest {
    pub fn request(&self) -> &ResearchRequest {
        &self.request
    }
    pub fn uid(&self) -> u32 {
        self.uid
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn start_ticks(&self) -> u64 {
        self.start_ticks
    }

    /// Call immediately before consuming the request; a reused or exited PID is refused.
    pub fn revalidate_peer(&self) -> anyhow::Result<()> {
        self.revalidate_at(Path::new("/proc"))
    }

    /// A cancellation was authenticated at receipt and may still settle a campaign after its
    /// sender exits. PID reuse, an inaccessible proc entry, and all other verbs remain denied.
    pub fn revalidate_for_handling(&self) -> anyhow::Result<()> {
        self.revalidate_for_handling_at(Path::new("/proc"))
    }

    fn revalidate_for_handling_at(&self, proc_root: &Path) -> anyhow::Result<()> {
        match self.revalidate_at(proc_root) {
            Ok(()) => Ok(()),
            Err(error) if matches!(self.request, ResearchRequest::Cancel {}) => {
                let root = std::fs::metadata(proc_root)?;
                anyhow::ensure!(root.is_dir(), "proc root unavailable");
                match std::fs::symlink_metadata(proc_root.join(self.pid.to_string())) {
                    Err(missing) if missing.kind() == io::ErrorKind::NotFound => Ok(()),
                    _ => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn revalidate_at(&self, proc_root: &Path) -> anyhow::Result<()> {
        let live = crate::telemetry::proc_view::live_process(proc_root, self.pid)
            .ok_or_else(|| anyhow::anyhow!("peer process unavailable"))?;
        anyhow::ensure!(
            live.uid == self.uid && live.start_ticks == self.start_ticks && self.start_ticks != 0,
            "peer identity changed"
        );
        Ok(())
    }
}

/// Read one request from a connected local peer. The expected UID is supplied by the resident
/// service configuration, not the client. Read time and memory are bounded; no writes occur.
pub fn receive_request(
    stream: &UnixStream,
    expected_uid: u32,
) -> anyhow::Result<AuthenticatedRequest> {
    anyhow::ensure!(expected_uid != 0, "research peer must be unprivileged");
    let (pid, uid, _) = crate::syscall::unix_peer_credentials(stream)?;
    anyhow::ensure!(uid == expected_uid && pid != 0, "wrong research peer");
    let live = crate::telemetry::proc_view::live_process(Path::new("/proc"), pid)
        .ok_or_else(|| anyhow::anyhow!("peer process unavailable"))?;
    anyhow::ensure!(
        live.uid == uid && live.start_ticks != 0,
        "peer identity unavailable"
    );
    let line = read_frame(stream, MAX_REQUEST_BYTES as usize, Duration::from_secs(2))?;
    let request: ResearchRequest = serde_json::from_slice(&line)?;
    if let ResearchRequest::Submit { intent } = &request {
        intent.validate()?;
        anyhow::ensure!(intent.uid == uid, "intent belongs to another user");
    }
    let authenticated = AuthenticatedRequest {
        request,
        uid,
        pid,
        start_ticks: live.start_ticks,
    };
    authenticated.revalidate_peer()?;
    Ok(authenticated)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn pair(message: &[u8]) -> (UnixStream, UnixStream) {
        let (mut client, server) = UnixStream::pair().unwrap();
        client.write_all(message).unwrap();
        (client, server)
    }

    #[test]
    fn kernel_peer_is_bound_and_wrong_uid_is_rejected() {
        let (_client, server) = pair(b"{\"verb\":\"status\"}\n");
        let uid = crate::syscall::geteuid();
        let request = receive_request(&server, uid).unwrap();
        assert_eq!(request.request(), &ResearchRequest::Status {});
        assert_eq!(request.pid(), std::process::id());
        assert!(request.revalidate_peer().is_ok());
        let mut stale = request;
        stale.start_ticks += 1;
        assert!(stale.revalidate_peer().is_err());
        let (_client, server) = pair(b"{\"verb\":\"status\"}\n");
        assert!(receive_request(&server, uid.wrapping_add(1)).is_err());
    }

    #[test]
    fn only_authenticated_cancel_survives_an_exited_peer() {
        let (_client, server) = pair(b"{\"verb\":\"cancel\"}\n");
        let uid = crate::syscall::geteuid();
        let cancel = receive_request(&server, uid).unwrap();
        let proc_root = tempfile::tempdir().unwrap();
        assert!(cancel.revalidate_for_handling_at(proc_root.path()).is_ok());
        std::fs::create_dir(proc_root.path().join(cancel.pid.to_string())).unwrap();
        assert!(cancel.revalidate_for_handling_at(proc_root.path()).is_err());
        let (_client, server) = pair(b"{\"verb\":\"status\"}\n");
        let status = receive_request(&server, uid).unwrap();
        std::fs::remove_dir(proc_root.path().join(cancel.pid.to_string())).unwrap();
        assert!(status.revalidate_for_handling_at(proc_root.path()).is_err());
    }

    #[test]
    fn unknown_commands_untrusted_fields_and_incomplete_frames_are_refused() {
        let uid = crate::syscall::geteuid();
        for message in [
            b"{\"verb\":\"execute\",\"command\":\"id\"}\n".as_slice(),
            b"{\"verb\":\"status\",\"command\":\"id\"}\n".as_slice(),
            b"{\"verb\":\"status\",\"verb\":\"cancel\"}\n".as_slice(),
        ] {
            let (_client, server) = pair(message);
            assert!(receive_request(&server, uid).is_err());
        }
        let (client, server) = pair(b"{\"verb\":\"status\"}");
        client.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(receive_request(&server, uid).is_err());
        let oversized = vec![b'x'; MAX_REQUEST_BYTES as usize + 1];
        let (_client, server) = pair(&oversized);
        assert!(receive_request(&server, uid).is_err());
    }

    #[test]
    fn submit_requires_the_authenticated_user_and_a_valid_bounded_intent() {
        let uid = crate::syscall::geteuid();
        let mut intent = super::super::tests::intent();
        intent.uid = uid;
        let encode = |intent: ResearchIntentV1| {
            let mut bytes = serde_json::to_vec(&ResearchRequest::Submit {
                intent: Box::new(intent),
            })
            .unwrap();
            bytes.push(b'\n');
            bytes
        };
        let (_client, server) = pair(&encode(intent.clone()));
        assert!(matches!(
            receive_request(&server, uid).unwrap().request(),
            ResearchRequest::Submit { .. }
        ));
        intent.uid = uid.wrapping_add(1);
        let (_client, server) = pair(&encode(intent.clone()));
        assert!(receive_request(&server, uid).is_err());
        intent.uid = uid;
        intent.desktop_research_consented = false;
        let (_client, server) = pair(&encode(intent));
        assert!(receive_request(&server, uid).is_err());
    }

    #[test]
    fn slow_sender_cannot_extend_the_total_frame_deadline() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let writer = std::thread::spawn(move || {
            for _ in 0..50 {
                if client.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let start = Instant::now();
        assert!(read_frame(&server, 1024, Duration::from_millis(60)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(server);
        writer.join().unwrap();
    }
}
