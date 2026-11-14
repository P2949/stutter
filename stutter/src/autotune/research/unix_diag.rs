//! Strict decoder for a bounded Linux UNIX socket diagnostic dump. The caller must authenticate
//! the netlink sender as the kernel and enforce a deadline and byte/packet limits. A peer inode
//! identifies a socket endpoint, not by itself the process or authority holding that endpoint.

const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const SOCK_DIAG_BY_FAMILY: u16 = 20;
const NLM_F_DUMP_INTR: u16 = 0x10;
const UNIX_DIAG_PEER: u16 = 2;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn aligned(size: usize) -> Option<usize> {
    size.checked_add(3).map(|n| n & !3)
}

#[derive(Default)]
pub(super) struct PeerDump {
    peer: Option<u32>,
    done: bool,
    failed: bool,
    query: Option<(u32, u32)>,
    bytes_seen: usize,
    packets: usize,
}

impl PeerDump {
    /// Consume one complete datagram from the authenticated kernel netlink socket.
    pub(super) fn ingest(&mut self, bytes: &[u8], sequence: u32, inode: u32) -> anyhow::Result<()> {
        if self.failed {
            anyhow::bail!("diagnostic dump already failed");
        }
        let result = self.ingest_inner(bytes, sequence, inode);
        self.failed |= result.is_err();
        result
    }

    fn ingest_inner(&mut self, bytes: &[u8], sequence: u32, inode: u32) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.done
                && inode != 0
                && !bytes.is_empty()
                && bytes.len() <= 65_536
                && self.packets < 128
                && self.bytes_seen.saturating_add(bytes.len()) <= 4 * 1024 * 1024
                && self.query.is_none_or(|query| query == (sequence, inode)),
            "invalid diagnostic state"
        );
        self.query = Some((sequence, inode));
        self.packets += 1;
        self.bytes_seen += bytes.len();
        let mut pos = 0usize;
        while pos < bytes.len() {
            let remaining = bytes.len() - pos;
            anyhow::ensure!(remaining >= 16, "truncated netlink header");
            let length =
                u32_at(bytes, pos).ok_or_else(|| anyhow::anyhow!("missing length"))? as usize;
            let kind =
                u16_at(bytes, pos + 4).ok_or_else(|| anyhow::anyhow!("missing message type"))?;
            let flags =
                u16_at(bytes, pos + 6).ok_or_else(|| anyhow::anyhow!("missing message flags"))?;
            let seq = u32_at(bytes, pos + 8)
                .ok_or_else(|| anyhow::anyhow!("missing message sequence"))?;
            anyhow::ensure!(
                length >= 16
                    && length <= remaining
                    && seq == sequence
                    && flags & NLM_F_DUMP_INTR == 0,
                "invalid or interrupted netlink response"
            );
            let end = pos + length;
            match kind {
                NLMSG_DONE => {
                    anyhow::ensure!(
                        (length == 16 || (length == 20 && u32_at(bytes, pos + 16) == Some(0)))
                            && end == bytes.len(),
                        "invalid dump completion"
                    );
                    self.done = true;
                }
                NLMSG_ERROR => anyhow::bail!("socket diagnostic request failed"),
                SOCK_DIAG_BY_FAMILY => {
                    anyhow::ensure!(
                        length >= 32 && bytes[pos + 16] == libc::AF_UNIX as u8,
                        "invalid UNIX diagnostic record"
                    );
                    if u32_at(bytes, pos + 20) == Some(inode) {
                        anyhow::ensure!(
                            bytes[pos + 17] == libc::SOCK_STREAM as u8 && self.peer.is_none(),
                            "unexpected or duplicate socket record"
                        );
                        let mut at = pos + 32;
                        let mut peer = None;
                        while at < end {
                            anyhow::ensure!(end - at >= 4, "truncated diagnostic attribute");
                            let len = u16_at(bytes, at)
                                .ok_or_else(|| anyhow::anyhow!("missing attribute length"))?
                                as usize;
                            let attr = u16_at(bytes, at + 2)
                                .ok_or_else(|| anyhow::anyhow!("missing attribute type"))?
                                & 0x3fff;
                            anyhow::ensure!(
                                len >= 4 && len <= end - at,
                                "invalid diagnostic attribute"
                            );
                            if attr == UNIX_DIAG_PEER {
                                anyhow::ensure!(
                                    len == 8 && peer.is_none(),
                                    "invalid peer attribute"
                                );
                                peer = u32_at(bytes, at + 4);
                            }
                            at = at
                                .checked_add(
                                    aligned(len)
                                        .ok_or_else(|| anyhow::anyhow!("attribute overflow"))?,
                                )
                                .ok_or_else(|| anyhow::anyhow!("attribute overflow"))?;
                            anyhow::ensure!(at <= end, "truncated attribute padding");
                        }
                        self.peer = Some(
                            peer.filter(|value| *value != 0)
                                .ok_or_else(|| anyhow::anyhow!("missing peer inode"))?,
                        );
                    }
                }
                _ => anyhow::bail!("unexpected diagnostic message"),
            }
            pos = pos
                .checked_add(aligned(length).ok_or_else(|| anyhow::anyhow!("message overflow"))?)
                .ok_or_else(|| anyhow::anyhow!("message overflow"))?;
            anyhow::ensure!(pos <= bytes.len(), "truncated netlink padding");
        }
        Ok(())
    }

    pub(super) fn finish(self) -> anyhow::Result<u32> {
        anyhow::ensure!(
            self.done && !self.failed,
            "incomplete or failed diagnostic dump"
        );
        self.peer
            .ok_or_else(|| anyhow::anyhow!("socket peer not found"))
    }
}

/// Resolve the peer endpoint of this live connection. This does not identify who owns it.
pub(super) fn peer_inode(stream: &std::os::unix::net::UnixStream) -> anyhow::Result<u32> {
    use std::os::unix::fs::MetadataExt;
    let duplicate: std::os::fd::OwnedFd = stream.try_clone()?.into();
    let inode: u32 = std::fs::File::from(duplicate)
        .metadata()?
        .ino()
        .try_into()
        .map_err(|_| anyhow::anyhow!("socket inode is outside diagnostic range"))?;
    let mut dump = PeerDump::default();
    crate::syscall::unix_diag_dump(|bytes| {
        dump.ingest(bytes, 1, inode)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(dump.done)
    })?;
    dump.finish()
}

/// Observe whether a process currently holds the endpoint. This is a candidate observation, not
/// exclusive ownership or process identity proof; callers must independently attest and recheck it.
pub(super) fn process_holds_peer(
    proc_root: &std::path::Path,
    pid: stutter_core::ids::Pid,
    peer: u32,
) -> anyhow::Result<bool> {
    anyhow::ensure!(pid.as_u32() > 1 && peer != 0, "invalid peer observation");
    let fds = std::fs::read_dir(proc_root.join(pid.to_string()).join("fd"))?;
    let expected = format!("socket:[{peer}]");
    let mut scanned = 0usize;
    let mut found = false;
    for entry in fds {
        scanned += 1;
        anyhow::ensure!(scanned <= 65_536, "process descriptor scan exceeded limit");
        let entry = entry?;
        let name = entry.file_name();
        anyhow::ensure!(
            name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()),
            "invalid descriptor name"
        );
        match std::fs::read_link(entry.path()) {
            Ok(target) if target == std::path::Path::new(&expected) => {
                anyhow::ensure!(!found, "peer endpoint held by multiple descriptors");
                found = true;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: u16, flags: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&((16 + payload.len()) as u32).to_ne_bytes());
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(&flags.to_ne_bytes());
        bytes.extend_from_slice(&seq.to_ne_bytes());
        bytes.extend_from_slice(&0u32.to_ne_bytes());
        bytes.extend_from_slice(payload);
        bytes.resize(aligned(bytes.len()).unwrap(), 0);
        bytes
    }

    fn record(inode: u32, peer: u32) -> Vec<u8> {
        let mut body = vec![libc::AF_UNIX as u8, libc::SOCK_STREAM as u8, 1, 0];
        body.extend_from_slice(&inode.to_ne_bytes());
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&8u16.to_ne_bytes());
        body.extend_from_slice(&UNIX_DIAG_PEER.to_ne_bytes());
        body.extend_from_slice(&peer.to_ne_bytes());
        message(SOCK_DIAG_BY_FAMILY, 0, 9, &body)
    }

    #[test]
    fn peer_requires_the_target_socket_and_complete_successful_dump() {
        let mut dump = PeerDump::default();
        dump.ingest(&record(20, 30), 9, 20).unwrap();
        assert!(PeerDump::default().finish().is_err());
        dump.ingest(&message(NLMSG_DONE, 0, 9, &0u32.to_ne_bytes()), 9, 20)
            .unwrap();
        assert_eq!(dump.finish().unwrap(), 30);
    }

    #[test]
    fn malformed_or_interrupted_responses_and_duplicate_records_fail_closed() {
        let mut dump = PeerDump::default();
        assert!(dump.ingest(&record(20, 30), 8, 20).is_err());
        assert!(dump.ingest(&record(20, 30), 9, 20).is_err());
        assert!(dump.finish().is_err());
        assert!(PeerDump::default().ingest(&vec![0; 65_537], 9, 20).is_err());
        assert!(
            PeerDump::default()
                .ingest(&record(20, 30)[..10], 9, 20)
                .is_err()
        );
        assert!(
            PeerDump::default()
                .ingest(&message(NLMSG_DONE, NLM_F_DUMP_INTR, 9, &[]), 9, 20)
                .is_err()
        );
        assert!(
            PeerDump::default()
                .ingest(&message(NLMSG_DONE, 0, 9, &1u32.to_ne_bytes()), 9, 20)
                .is_err()
        );
        let mut duplicate = PeerDump::default();
        duplicate.ingest(&record(20, 30), 9, 20).unwrap();
        assert!(duplicate.ingest(&record(20, 30), 9, 20).is_err());
    }

    #[test]
    fn kernel_diagnostic_resolves_a_local_socket_pair() {
        use std::os::unix::fs::MetadataExt;
        let (first, second) = std::os::unix::net::UnixStream::pair().unwrap();
        let second_fd: std::os::fd::OwnedFd = second.try_clone().unwrap().into();
        let expected = std::fs::File::from(second_fd).metadata().unwrap().ino();
        assert_eq!(u64::from(peer_inode(&first).unwrap()), expected);
    }

    #[test]
    fn descriptor_observation_refuses_ambiguity_and_missing_processes() {
        use std::os::unix::fs::symlink;
        let proc = tempfile::tempdir().unwrap();
        let dir = proc.path().join("123/fd");
        std::fs::create_dir_all(&dir).unwrap();
        symlink("socket:[42]", dir.join("3")).unwrap();
        assert!(process_holds_peer(proc.path(), stutter_core::ids::Pid::new(123), 42).unwrap());
        assert!(!process_holds_peer(proc.path(), stutter_core::ids::Pid::new(123), 43).unwrap());
        symlink("socket:[42]", dir.join("4")).unwrap();
        assert!(process_holds_peer(proc.path(), stutter_core::ids::Pid::new(123), 42).is_err());
        assert!(process_holds_peer(proc.path(), stutter_core::ids::Pid::new(124), 42).is_err());
    }

    #[test]
    #[ignore = "read-only observation of the current user's live session bus"]
    fn current_user_bus_peer_inode_is_held_by_broker() {
        use std::os::unix::fs::MetadataExt;
        let uid = crate::syscall::geteuid();
        let stream =
            std::os::unix::net::UnixStream::connect(format!("/run/user/{uid}/bus")).unwrap();
        let peer = peer_inode(&stream).unwrap();
        let expected = format!("socket:[{peer}]");
        let mut matches = 0;
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let path = entry.path();
            if !entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|c| c.is_ascii_digit())
                || !path.metadata().is_ok_and(|meta| meta.uid() == uid)
                || !std::fs::read_to_string(path.join("comm"))
                    .is_ok_and(|name| name.trim() == "dbus-broker")
            {
                continue;
            }
            if let Ok(fds) = std::fs::read_dir(path.join("fd")) {
                for fd in fds.flatten() {
                    if std::fs::read_link(fd.path())
                        .is_ok_and(|target| target == std::path::Path::new(&expected))
                    {
                        matches += 1;
                    }
                }
            }
        }
        assert_eq!(
            matches, 1,
            "peer inode was not uniquely observed in the user broker"
        );
    }
}
