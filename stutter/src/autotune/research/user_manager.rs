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

use super::{
    ResearchIntentV1, user_launcher::VerifiedSessionEnvironment, user_manager_port::BusFault,
};
use crate::integrations::dbus::{DbusClient, DbusError, Value, Writer, connect_stream};

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

fn verify_manager_executable(proc_exe: &Path, expected: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        std::fs::canonicalize(proc_exe)? == expected,
        "user manager executable mismatch"
    );
    Ok(())
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
    let cmdline = read_bounded(&dir.join("cmdline"))?;
    // argv[0] is process-controlled and cannot attest the executable when procfs denies access.
    verify_manager_executable(&dir.join("exe"), &expected)?;
    anyhow::ensure!(manager_command_line(&cmdline), "not a user manager");
    let cgroup = read_bounded(&dir.join("cgroup"))?;
    let cgroup = std::str::from_utf8(&cgroup)?;
    anyhow::ensure!(manager_cgroup(cgroup, uid), "user manager cgroup mismatch");
    anyhow::ensure!(
        super::session_env::user_manager_capabilities_clear(proc, raw, uid),
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

    /// One bounded call to the authenticated systemd manager. The bus, session and manager
    /// identity are revalidated first, and the reply must come from the manager's own unique
    /// name, so a bus-generated or foreign error is never mistaken for the manager's answer.
    pub(super) fn call_manager(
        &mut self,
        path: &str,
        interface: &str,
        member: &str,
        signature: &str,
        body: &[u8],
    ) -> Result<Vec<Value>, BusFault> {
        self.revalidate().map_err(|error| {
            log::warn!("user-manager revalidation failed: {error:#}");
            BusFault::NotSent
        })?;
        let sender = self.identity.unique_name.clone();
        match self.client.call_with_sender(
            SYSTEMD_DESTINATION,
            path,
            interface,
            member,
            signature,
            body,
            Some(&sender),
        ) {
            Ok(values) => Ok(values),
            Err(DbusError::Remote(error)) => Err(BusFault::Remote(error)),
            Err(_) => Err(BusFault::Uncertain),
        }
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
#[path = "user_manager_tests.rs"]
mod tests;
