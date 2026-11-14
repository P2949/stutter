//! Read-only authentication of the current user's systemd manager. This module does not start,
//! stop, or modify units. The connection alone grants no campaign or process-control authority.
//! Socket-activated buses may report the socket creator rather than the broker in SO_PEERCRED;
//! those hosts are refused until the broker and response origin can be independently verified.
use std::{
    fs::File,
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use stutter_core::ids::Pid;

use super::{ResearchIntentV1, user_launcher::VerifiedSessionEnvironment};
use crate::integrations::dbus::{DbusClient, Value, Writer, connect_stream};

const MAX_PROC_BYTES: u64 = 16 * 1024;
const SYSTEMD_DESTINATION: &str = "org.freedesktop.systemd1";
const BUS_DESTINATION: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagerIdentity {
    pub uid: u32,
    pub pid: Pid,
    pub start_ticks: u64,
    pub unique_name: String,
}

pub struct VerifiedUserManager {
    client: DbusClient,
    peer_stream: std::os::unix::net::UnixStream,
    peer_inode: u32,
    identity: ManagerIdentity,
    peer_pid: Pid,
    peer_start_ticks: u64,
    runtime: PathBuf,
    runtime_device: u64,
    runtime_inode: u64,
    socket_device: u64,
    socket_inode: u64,
    boot_id: String,
    session_id: String,
    session_observed_at: Instant,
    verified_at: Instant,
}

fn read_bounded(path: &Path) -> anyhow::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_PROC_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_PROC_BYTES,
        "user manager record exceeds limit"
    );
    Ok(bytes)
}

fn manager_command_line(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.last() == Some(&0)
        && bytes
            .split(|byte| *byte == 0)
            .filter(|arg| *arg == b"--user")
            .count()
            == 1
}

fn manager_cgroup(cgroup: &str, uid: u32) -> bool {
    let expected = format!("0::/user.slice/user-{uid}.slice/user@{uid}.service/init.scope");
    cgroup.lines().any(|line| line == expected)
}

fn bus_cgroup(cgroup: &str, uid: u32) -> bool {
    let prefix = format!("0::/user.slice/user-{uid}.slice/user@{uid}.service/");
    cgroup
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .any(|tail| {
            let parts: Vec<_> = tail.split('/').collect();
            parts
                .last()
                .is_some_and(|last| matches!(*last, "dbus.service" | "dbus-broker.service"))
                && parts[..parts.len().saturating_sub(1)]
                    .iter()
                    .all(|part| !part.is_empty() && part.ends_with(".slice"))
        })
}

fn inspect_manager_process(pid: Pid, uid: u32) -> anyhow::Result<u64> {
    let proc = Path::new("/proc");
    let raw = pid.as_u32();
    let before = crate::telemetry::proc_view::live_process(proc, raw)
        .ok_or_else(|| anyhow::anyhow!("user manager process unavailable"))?;
    anyhow::ensure!(
        uid != 0 && before.uid == uid && before.start_ticks != 0,
        "user manager owner mismatch"
    );
    let expected = std::fs::canonicalize("/run/current-system/systemd/lib/systemd/systemd")?;
    let meta = std::fs::metadata(&expected)?;
    anyhow::ensure!(
        expected.starts_with("/nix/store")
            && meta.is_file()
            && meta.uid() == 0
            && meta.mode() & 0o222 == 0,
        "untrusted systemd executable"
    );
    let dir = proc.join(raw.to_string());
    anyhow::ensure!(
        std::fs::canonicalize(dir.join("exe"))? == expected,
        "user manager executable mismatch"
    );
    let cmdline = read_bounded(&dir.join("cmdline"))?;
    anyhow::ensure!(manager_command_line(&cmdline), "not a user manager");
    let cgroup = read_bounded(&dir.join("cgroup"))?;
    let cgroup = std::str::from_utf8(&cgroup)?;
    anyhow::ensure!(manager_cgroup(cgroup, uid), "user manager cgroup mismatch");
    anyhow::ensure!(
        super::session_env::process_capabilities_clear(proc, raw, uid, false),
        "user manager carries capabilities"
    );
    let after = crate::telemetry::proc_view::live_process(proc, raw)
        .ok_or_else(|| anyhow::anyhow!("user manager process exited"))?;
    anyhow::ensure!(before == after, "user manager process changed");
    Ok(before.start_ticks)
}

fn inspect_bus_peer(pid: Pid, uid: u32) -> anyhow::Result<u64> {
    let proc = Path::new("/proc");
    let raw = pid.as_u32();
    let before = crate::telemetry::proc_view::live_process(proc, raw)
        .ok_or_else(|| anyhow::anyhow!("user bus peer unavailable"))?;
    anyhow::ensure!(
        uid != 0 && before.uid == uid && before.start_ticks != 0,
        "user bus peer owner mismatch"
    );
    let dir = proc.join(raw.to_string());
    let executable = std::fs::canonicalize(dir.join("exe"))?;
    let known = [
        "/run/current-system/sw/bin/dbus-broker",
        "/run/current-system/sw/bin/dbus-daemon",
    ]
    .iter()
    .filter_map(|path| std::fs::canonicalize(path).ok())
    .any(|path| path == executable);
    let meta = std::fs::metadata(&executable)?;
    anyhow::ensure!(
        known
            && executable.starts_with("/nix/store")
            && meta.is_file()
            && meta.uid() == 0
            && meta.mode() & 0o222 == 0,
        "untrusted user bus executable"
    );
    let cgroup = read_bounded(&dir.join("cgroup"))?;
    let cgroup = std::str::from_utf8(&cgroup)?;
    anyhow::ensure!(bus_cgroup(cgroup, uid), "user bus cgroup mismatch");
    anyhow::ensure!(
        super::session_env::broker_capabilities_clear(proc, raw, uid),
        "user bus peer carries capabilities"
    );
    let after = crate::telemetry::proc_view::live_process(proc, raw)
        .ok_or_else(|| anyhow::anyhow!("user bus peer exited"))?;
    anyhow::ensure!(before == after, "user bus peer changed");
    Ok(before.start_ticks)
}

/// Identify a trusted broker candidate holding the connected peer endpoint. A candidate is not
/// sufficient by itself to authorize a manager call: session, owner and response checks remain.
fn broker_candidate(
    proc_root: &Path,
    uid: u32,
    peer_inode: u32,
    inspect: &dyn Fn(Pid, u32) -> anyhow::Result<u64>,
) -> anyhow::Result<(Pid, u64)> {
    let mut scanned = 0usize;
    let mut candidate = None;
    for entry in std::fs::read_dir(proc_root)? {
        scanned += 1;
        anyhow::ensure!(scanned <= 65_536, "process scan exceeded limit");
        let entry = entry?;
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|text| text.parse::<u32>().ok()) else {
            continue;
        };
        if pid <= 1 {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.uid() != uid {
            continue;
        }
        let comm = match read_bounded(&entry.path().join("comm")) {
            Ok(comm) => comm,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if !matches!(comm.as_slice(), b"dbus-broker\n" | b"dbus-daemon\n") {
            continue;
        }
        let pid = Pid::new(pid);
        if !super::unix_diag::process_holds_peer(proc_root, pid, peer_inode)? {
            continue;
        }
        let before = inspect(pid, uid)?;
        anyhow::ensure!(
            before != 0
                && super::unix_diag::process_holds_peer(proc_root, pid, peer_inode)?
                && inspect(pid, uid)? == before
                && candidate.is_none(),
            "broker peer identity is ambiguous"
        );
        candidate = Some((pid, before));
    }
    candidate.ok_or_else(|| anyhow::anyhow!("user bus broker peer unavailable"))
}

fn bus_u32(client: &mut DbusClient, method: &str, owner: &str) -> anyhow::Result<u32> {
    let mut body = Writer::default();
    body.string(owner);
    let result = client
        .call_with_sender(
            BUS_DESTINATION,
            BUS_PATH,
            BUS_DESTINATION,
            method,
            "s",
            &body.buf,
            Some(BUS_DESTINATION),
        )
        .map_err(|_| anyhow::anyhow!("user bus identity query failed"))?;
    match result.as_slice() {
        [Value::U32(value)] => Ok(*value),
        _ => anyhow::bail!("unexpected user bus identity reply"),
    }
}

fn bus_name_owner(client: &mut DbusClient) -> anyhow::Result<String> {
    let mut body = Writer::default();
    body.string(SYSTEMD_DESTINATION);
    let result = client
        .call_with_sender(
            BUS_DESTINATION,
            BUS_PATH,
            BUS_DESTINATION,
            "GetNameOwner",
            "s",
            &body.buf,
            Some(BUS_DESTINATION),
        )
        .map_err(|_| anyhow::anyhow!("user manager owner query failed"))?;
    match result.as_slice() {
        [Value::Str(owner)] => Ok(owner.clone()),
        _ => anyhow::bail!("unexpected user bus owner reply"),
    }
}

fn authenticate_manager(
    client: &mut DbusClient,
    uid: u32,
    inspect: &dyn Fn(Pid, u32) -> anyhow::Result<u64>,
) -> anyhow::Result<ManagerIdentity> {
    let owner = bus_name_owner(client)?;
    anyhow::ensure!(
        owner.starts_with(':')
            && owner.len() > 2
            && owner.len() <= 255
            && owner.contains('.')
            && owner
                .bytes()
                .skip(1)
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')),
        "invalid user manager bus identity"
    );
    anyhow::ensure!(
        bus_u32(client, "GetConnectionUnixUser", &owner)? == uid,
        "user manager bus owner mismatch"
    );
    let pid = Pid::new(bus_u32(client, "GetConnectionUnixProcessID", &owner)?);
    anyhow::ensure!(pid.as_u32() > 1, "invalid user manager pid");
    let ticks = inspect(pid, uid)?;
    anyhow::ensure!(
        ticks != 0
            && bus_name_owner(client)? == owner
            && bus_u32(client, "GetConnectionUnixProcessID", &owner)? == pid.as_u32()
            && inspect(pid, uid)? == ticks,
        "user manager identity changed"
    );
    Ok(ManagerIdentity {
        uid,
        pid,
        start_ticks: ticks,
        unique_name: owner,
    })
}

impl VerifiedUserManager {
    pub fn inspect(
        session: &VerifiedSessionEnvironment,
        intent: &ResearchIntentV1,
    ) -> anyhow::Result<Self> {
        let (uid, runtime, device, inode, observed_at) = session.runtime_identity();
        anyhow::ensure!(
            uid == intent.uid
                && uid == crate::syscall::geteuid()
                && observed_at.elapsed() <= Duration::from_secs(3),
            "session observation is stale"
        );
        let runtime_fd = File::from(crate::syscall::open_dir_no_symlinks(runtime)?);
        let runtime_meta = runtime_fd.metadata()?;
        anyhow::ensure!(
            runtime_meta.uid() == uid
                && runtime_meta.mode() & 0o077 == 0
                && runtime_meta.dev() == device
                && runtime_meta.ino() == inode,
            "runtime directory changed"
        );
        let socket_path = runtime.join("bus");
        let socket = std::fs::symlink_metadata(&socket_path)?;
        anyhow::ensure!(
            socket.file_type().is_socket() && socket.uid() == uid && socket.nlink() == 1,
            "untrusted user bus socket"
        );
        let stream = connect_stream(&socket_path)
            .map_err(|_| anyhow::anyhow!("user bus connection failed"))?;
        let (creator_pid, peer_uid, _) = crate::syscall::unix_peer_credentials(&stream)?;
        anyhow::ensure!(peer_uid == uid && creator_pid > 1, "user bus peer mismatch");
        let peer_inode = super::unix_diag::peer_inode(&stream)?;
        let (peer_pid, peer_ticks) =
            broker_candidate(Path::new("/proc"), uid, peer_inode, &inspect_bus_peer)?;
        let peer_stream = stream.try_clone()?;
        let after_socket = std::fs::symlink_metadata(&socket_path)?;
        anyhow::ensure!(
            after_socket.file_type().is_socket()
                && after_socket.uid() == uid
                && after_socket.dev() == socket.dev()
                && after_socket.ino() == socket.ino(),
            "user bus socket changed"
        );
        let mut client = DbusClient::from_stream(stream, uid)
            .map_err(|_| anyhow::anyhow!("user bus authentication failed"))?;
        let identity = authenticate_manager(&mut client, uid, &inspect_manager_process)?;
        let verified_at = Instant::now();
        let runtime_after =
            File::from(crate::syscall::open_dir_no_symlinks(runtime)?).metadata()?;
        let socket_after = std::fs::symlink_metadata(&socket_path)?;
        anyhow::ensure!(
            inspect_bus_peer(peer_pid, uid)? == peer_ticks
                && super::unix_diag::peer_inode(&peer_stream)? == peer_inode
                && super::unix_diag::process_holds_peer(Path::new("/proc"), peer_pid, peer_inode)?
                && runtime_after.uid() == uid
                && runtime_after.mode() & 0o077 == 0
                && runtime_after.dev() == device
                && runtime_after.ino() == inode
                && socket_after.file_type().is_socket()
                && socket_after.uid() == uid
                && socket_after.nlink() == 1
                && socket_after.dev() == socket.dev()
                && socket_after.ino() == socket.ino()
                && crate::telemetry::identity::read_boot_id()? == intent.boot_id
                && super::host::active_wayland_session(uid)? == intent.session_id
                && observed_at.elapsed() <= Duration::from_secs(3),
            "user session changed during manager inspection"
        );
        Ok(Self {
            client,
            peer_stream,
            peer_inode,
            identity,
            peer_pid,
            peer_start_ticks: peer_ticks,
            runtime: runtime.to_path_buf(),
            runtime_device: device,
            runtime_inode: inode,
            socket_device: socket.dev(),
            socket_inode: socket.ino(),
            boot_id: intent.boot_id.clone(),
            session_id: intent.session_id.clone(),
            session_observed_at: observed_at,
            verified_at,
        })
    }

    pub fn identity(&self) -> &ManagerIdentity {
        &self.identity
    }

    pub fn is_fresh(&self) -> bool {
        self.verified_at.elapsed() <= Duration::from_secs(3)
    }

    pub fn revalidate(&mut self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.is_fresh() && self.session_observed_at.elapsed() <= Duration::from_secs(3),
            "user manager observation is stale"
        );
        let runtime =
            File::from(crate::syscall::open_dir_no_symlinks(&self.runtime)?).metadata()?;
        let socket = std::fs::symlink_metadata(self.runtime.join("bus"))?;
        anyhow::ensure!(
            runtime.uid() == self.identity.uid
                && runtime.mode() & 0o077 == 0
                && runtime.dev() == self.runtime_device
                && runtime.ino() == self.runtime_inode
                && socket.file_type().is_socket()
                && socket.uid() == self.identity.uid
                && socket.nlink() == 1
                && socket.dev() == self.socket_device
                && socket.ino() == self.socket_inode
                && inspect_bus_peer(self.peer_pid, self.identity.uid)? == self.peer_start_ticks
                && super::unix_diag::peer_inode(&self.peer_stream)? == self.peer_inode
                && super::unix_diag::process_holds_peer(
                    Path::new("/proc"),
                    self.peer_pid,
                    self.peer_inode
                )?
                && crate::telemetry::identity::read_boot_id()? == self.boot_id
                && super::host::active_wayland_session(self.identity.uid)? == self.session_id,
            "user bus or session identity changed"
        );
        let current = authenticate_manager(
            &mut self.client,
            self.identity.uid,
            &inspect_manager_process,
        )?;
        anyhow::ensure!(
            current == self.identity
                && self.is_fresh()
                && self.session_observed_at.elapsed() <= Duration::from_secs(3),
            "user manager identity changed"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;

    use super::*;
    use crate::integrations::dbus::fake::{Reply, serve};

    #[test]
    fn broker_candidate_requires_a_unique_stable_owner_of_the_peer_endpoint() {
        use std::os::unix::fs::symlink;
        let proc = tempfile::tempdir().unwrap();
        for pid in [123u32, 124] {
            let dir = proc.path().join(pid.to_string());
            std::fs::create_dir_all(dir.join("fd")).unwrap();
            std::fs::write(dir.join("comm"), "dbus-broker\n").unwrap();
        }
        symlink("socket:[42]", proc.path().join("123/fd/4")).unwrap();
        let uid = crate::syscall::geteuid();
        let candidate =
            broker_candidate(proc.path(), uid, 42, &|pid, _| Ok(u64::from(pid.as_u32()))).unwrap();
        assert_eq!((candidate.0.as_u32(), candidate.1), (123, 123));
        let candidate = broker_candidate(proc.path(), uid, 42, &|pid, _| {
            anyhow::ensure!(
                pid.as_u32() == 123,
                "unrelated broker must not be inspected"
            );
            Ok(123)
        })
        .unwrap();
        assert_eq!(candidate.0.as_u32(), 123);
        assert!(broker_candidate(proc.path(), uid, 43, &|_, _| Ok(1)).is_err());
        let calls = std::sync::atomic::AtomicUsize::new(0);
        assert!(
            broker_candidate(proc.path(), uid, 42, &|_, _| {
                Ok(calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) as u64 + 1)
            })
            .is_err()
        );
        symlink("socket:[42]", proc.path().join("124/fd/4")).unwrap();
        assert!(broker_candidate(proc.path(), uid, 42, &|_, _| Ok(1)).is_err());
    }

    #[test]
    #[ignore = "read-only observation of the current user's live session bus"]
    fn live_session_bus_peer_has_a_stable_broker_candidate() {
        let uid = crate::syscall::geteuid();
        let stream = connect_stream(&PathBuf::from(format!("/run/user/{uid}/bus"))).unwrap();
        let peer = super::super::unix_diag::peer_inode(&stream).unwrap();
        let (pid, ticks) =
            broker_candidate(Path::new("/proc"), uid, peer, &inspect_bus_peer).unwrap();
        assert!(ticks > 0);
        assert_eq!(inspect_bus_peer(pid, uid).unwrap(), ticks);
        assert_eq!(super::super::unix_diag::peer_inode(&stream).unwrap(), peer);
        assert!(
            super::super::unix_diag::process_holds_peer(Path::new("/proc"), pid, peer).unwrap()
        );
    }

    fn reply_string(value: &str) -> Reply {
        let mut writer = Writer::default();
        writer.string(value);
        Reply::Body("s".into(), writer.buf)
    }

    fn reply_u32(value: u32) -> Reply {
        let mut writer = Writer::default();
        writer.u32(value);
        Reply::Body("u".into(), writer.buf)
    }

    #[test]
    fn manager_and_bus_must_be_in_the_exact_user_service_scope() {
        assert!(manager_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/init.scope\n",
            1000
        ));
        assert!(!manager_cgroup(
            "0::/other/user-1000.slice/user@1000.service/init.scope\n",
            1000
        ));
        assert!(!manager_cgroup(
            "0::/user.slice/user-1001.slice/user@1001.service/init.scope\n",
            1000
        ));
        assert!(bus_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/session.slice/dbus-broker.service\n",
            1000
        ));
        assert!(!bus_cgroup(
            "0::/user.slice/user-1000.slice/user@1000.service/app.service/dbus-broker.service\n",
            1000
        ));
        assert!(!bus_cgroup(
            "0::/other/user-1000.slice/user@1000.service/session.slice/dbus-broker.service\n",
            1000
        ));
        assert!(manager_command_line(b"/nix/store/systemd\0--user\0"));
        assert!(!manager_command_line(b"/nix/store/systemd\0--user"));
        assert!(!manager_command_line(
            b"/nix/store/systemd\0--user\0--user\0"
        ));
    }

    #[test]
    fn owner_attestation_uses_only_read_only_queries_and_rechecks_identity() {
        let (client_end, server_end) = UnixStream::pair().unwrap();
        let calls = serve(server_end, |call| match call.member.as_str() {
            "GetNameOwner" => reply_string(":1.7"),
            "GetConnectionUnixUser" => reply_u32(1000),
            "GetConnectionUnixProcessID" => reply_u32(4242),
            _ => Reply::Error("org.test.Unexpected"),
        });
        let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
        let identity = authenticate_manager(&mut client, 1000, &|pid, uid| {
            assert_eq!((pid.as_u32(), uid), (4242, 1000));
            Ok(77)
        })
        .unwrap();
        assert_eq!(identity.pid.as_u32(), 4242);
        assert_eq!(identity.start_ticks, 77);
        drop(client);
        let seen = calls.lock().unwrap();
        assert_eq!(
            seen.iter()
                .map(|call| call.member.as_str())
                .collect::<Vec<_>>(),
            [
                "GetNameOwner",
                "GetConnectionUnixUser",
                "GetConnectionUnixProcessID",
                "GetNameOwner",
                "GetConnectionUnixProcessID"
            ]
        );
    }

    #[test]
    fn wrong_bus_owner_and_changed_owner_are_refused() {
        let (client_end, server_end) = UnixStream::pair().unwrap();
        serve(server_end, |call| match call.member.as_str() {
            "GetNameOwner" => reply_string(":1.7"),
            "GetConnectionUnixUser" => reply_u32(1001),
            _ => Reply::Error("org.test.Unexpected"),
        });
        let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
        assert!(authenticate_manager(&mut client, 1000, &|_, _| Ok(77)).is_err());

        let (client_end, server_end) = UnixStream::pair().unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        serve(server_end, move |call| match call.member.as_str() {
            "GetNameOwner" => {
                if observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                    reply_string(":1.7")
                } else {
                    reply_string(":1.8")
                }
            }
            "GetConnectionUnixUser" => reply_u32(1000),
            "GetConnectionUnixProcessID" => reply_u32(4242),
            _ => Reply::Error("org.test.Unexpected"),
        });
        let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
        assert!(authenticate_manager(&mut client, 1000, &|_, _| Ok(77)).is_err());
    }

    #[test]
    fn owner_reply_from_a_different_sender_is_refused() {
        let (client_end, server_end) = UnixStream::pair().unwrap();
        serve(server_end, |_| {
            let mut writer = Writer::default();
            writer.string(":1.7");
            Reply::BodyFrom("s".into(), writer.buf, ":1.666")
        });
        let mut client = DbusClient::from_stream(client_end, 1000).unwrap();
        assert!(authenticate_manager(&mut client, 1000, &|_, _| Ok(77)).is_err());
    }

    #[test]
    fn manager_witness_expires_and_retains_its_identity() {
        let (client_end, server_end) = UnixStream::pair().unwrap();
        serve(server_end, |_| Reply::Error("org.test.Unexpected"));
        let peer_stream = client_end.try_clone().unwrap();
        let client = DbusClient::from_stream(client_end, 1000).unwrap();
        let identity = ManagerIdentity {
            uid: 1000,
            pid: Pid::new(4242),
            start_ticks: 77,
            unique_name: ":1.7".into(),
        };
        let mut manager = VerifiedUserManager {
            client,
            peer_stream,
            peer_inode: 1,
            identity: identity.clone(),
            peer_pid: Pid::new(4243),
            peer_start_ticks: 78,
            runtime: "/run/user/1000".into(),
            runtime_device: 1,
            runtime_inode: 2,
            socket_device: 3,
            socket_inode: 4,
            boot_id: "fixture-boot".into(),
            session_id: "fixture-session".into(),
            session_observed_at: Instant::now() - Duration::from_secs(4),
            verified_at: Instant::now() - Duration::from_secs(4),
        };
        assert_eq!(manager.identity(), &identity);
        assert!(!manager.is_fresh());
        assert!(manager.revalidate().is_err());
        let _: fn(
            &VerifiedSessionEnvironment,
            &ResearchIntentV1,
        ) -> anyhow::Result<VerifiedUserManager> = VerifiedUserManager::inspect;
    }
}
