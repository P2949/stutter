//! Select the unique compositor socket for the already identified local graphical session.
use std::{
    fs::{self, File},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Deserialize;
use tokio::io::AsyncReadExt;

const MAX_ENTRIES: usize = 4096;
const MAX_SOCKETS: usize = 16;
const MAX_HELPER_OUTPUT: u64 = 64 * 1024;

#[derive(Deserialize)]
struct Instance {
    pid: u32,
    wl_socket: String,
}

fn socket_name(name: &str) -> bool {
    name.strip_prefix("wayland-").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn peer_in_session(pid: u32, uid: u32, session: &str) -> anyhow::Result<u64> {
    anyhow::ensure!(
        !session.is_empty() && session.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "invalid session identity"
    );
    let proc = Path::new("/proc");
    let before = crate::telemetry::proc_view::live_process(proc, pid)
        .ok_or_else(|| anyhow::anyhow!("compositor identity unavailable"))?;
    anyhow::ensure!(
        before.uid == uid && before.start_ticks != 0,
        "compositor owner mismatch"
    );
    let mut bytes = Vec::new();
    File::open(proc.join(pid.to_string()).join("cgroup"))?
        .take(16_385)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 16_384, "compositor cgroup exceeds limit");
    let cgroup = std::str::from_utf8(&bytes)?;
    let scope = format!("session-{session}.scope");
    anyhow::ensure!(
        cgroup
            .lines()
            .filter_map(|line| line.split_once("::").map(|(_, path)| path))
            .any(|path| path.split('/').any(|part| part == scope)),
        "compositor is outside the active session"
    );
    let after = crate::telemetry::proc_view::live_process(proc, pid)
        .ok_or_else(|| anyhow::anyhow!("compositor exited"))?;
    anyhow::ensure!(
        after.uid == uid && after.start_ticks == before.start_ticks,
        "compositor identity changed"
    );
    Ok(before.start_ticks)
}

async fn active_session(uid: u32, expected: &str) -> anyhow::Result<()> {
    let current =
        tokio::task::spawn_blocking(move || super::super::host::active_wayland_session(uid))
            .await??;
    anyhow::ensure!(current == expected, "graphical session changed");
    Ok(())
}

async fn hyprland_instances(runtime: &Path) -> anyhow::Result<Vec<Instance>> {
    let executable = fs::canonicalize("/run/current-system/sw/bin/hyprctl")?;
    let metadata = fs::metadata(&executable)?;
    anyhow::ensure!(
        executable.starts_with("/nix/store")
            && metadata.is_file()
            && metadata.uid() == 0
            && metadata.mode() & 0o222 == 0,
        "untrusted compositor helper"
    );
    let mut child = tokio::process::Command::new(executable)
        .args(["-j", "instances"])
        .env_clear()
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let output = tokio::time::timeout(Duration::from_secs(2), async move {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("compositor helper has no output"))?;
        let mut bytes = Vec::new();
        stdout
            .take(MAX_HELPER_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .await?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_HELPER_OUTPUT,
            "compositor helper output exceeds limit"
        );
        anyhow::ensure!(child.wait().await?.success(), "compositor helper failed");
        Ok::<_, anyhow::Error>(bytes)
    })
    .await??;
    let instances: Vec<Instance> = serde_json::from_slice(&output)?;
    anyhow::ensure!(
        instances.len() <= MAX_SOCKETS,
        "too many compositor instances"
    );
    Ok(instances)
}

pub(super) struct ActiveCompositor {
    pub stream: std::os::unix::net::UnixStream,
    pid: u32,
    ticks: u64,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl ActiveCompositor {
    pub async fn revalidate(&self, uid: u32, session: &str) -> anyhow::Result<()> {
        active_session(uid, session).await?;
        anyhow::ensure!(
            peer_in_session(self.pid, uid, session)? == self.ticks,
            "compositor process changed"
        );
        let current = fs::symlink_metadata(&self.path)?;
        anyhow::ensure!(
            current.file_type().is_socket()
                && current.uid() == uid
                && current.dev() == self.device
                && current.ino() == self.inode,
            "compositor socket changed"
        );
        Ok(())
    }
}

pub(super) async fn active_compositor(uid: u32, session: &str) -> anyhow::Result<ActiveCompositor> {
    anyhow::ensure!(
        uid != 0 && uid == crate::syscall::geteuid(),
        "wrong observer user"
    );
    active_session(uid, session).await?;
    let runtime = PathBuf::from(format!("/run/user/{uid}"));
    let directory = crate::syscall::open_dir_no_symlinks(&runtime)?;
    let before = File::from(directory.try_clone()?).metadata()?;
    anyhow::ensure!(
        before.is_dir() && before.uid() == uid && before.mode() & 0o077 == 0,
        "untrusted user runtime directory"
    );
    let instances = hyprland_instances(&runtime).await?;
    let mut sockets = Vec::new();
    for (count, entry) in
        fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))?.enumerate()
    {
        anyhow::ensure!(count < MAX_ENTRIES, "too many runtime entries");
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if socket_name(&name) {
            sockets.push(name);
            anyhow::ensure!(sockets.len() <= MAX_SOCKETS, "too many Wayland sockets");
        }
    }
    let mut accepted = Vec::new();
    for name in sockets {
        let path = runtime.join(&name);
        let meta = fs::symlink_metadata(&path)?;
        if !meta.file_type().is_socket() || meta.uid() != uid || meta.nlink() != 1 {
            continue;
        }
        let stream = match tokio::time::timeout(
            Duration::from_secs(1),
            tokio::net::UnixStream::connect(&path),
        )
        .await
        {
            Ok(Ok(stream)) => stream.into_std()?,
            _ => continue,
        };
        let (pid, peer_uid, _) = crate::syscall::unix_peer_credentials(&stream)?;
        if peer_uid != uid
            || instances
                .iter()
                .filter(|instance| instance.pid == pid && instance.wl_socket == name)
                .count()
                != 1
        {
            continue;
        }
        let ticks = peer_in_session(pid, uid, session)?;
        let current = fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            current.file_type().is_socket()
                && current.uid() == uid
                && current.dev() == meta.dev()
                && current.ino() == meta.ino(),
            "compositor socket changed"
        );
        anyhow::ensure!(
            peer_in_session(pid, uid, session)? == ticks,
            "compositor process changed"
        );
        accepted.push(ActiveCompositor {
            stream,
            pid,
            ticks,
            path,
            device: meta.dev(),
            inode: meta.ino(),
        });
    }
    let current = File::from(crate::syscall::open_dir_no_symlinks(&runtime)?).metadata()?;
    anyhow::ensure!(
        current.dev() == before.dev() && current.ino() == before.ino(),
        "user runtime directory changed"
    );
    active_session(uid, session).await?;
    anyhow::ensure!(
        accepted.len() == 1,
        "active compositor socket is unavailable or ambiguous"
    );
    accepted
        .pop()
        .ok_or_else(|| anyhow::anyhow!("active compositor socket unavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_numeric_wayland_socket_names_are_accepted() {
        assert!(socket_name("wayland-1"));
        for name in ["wayland-", "wayland-1.lock", "wayland-../x", "xwayland-1"] {
            assert!(!socket_name(name));
        }
    }
}
