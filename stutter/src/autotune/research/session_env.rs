//! Read only the routing paths needed for a user-session Steam helper. Other environment values
//! are neither returned nor logged. A matching environment does not itself authorize a launch.
use std::{
    fs::File,
    io::Read,
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

use super::SteamProcessIdentity;

const MAX_PROCESS_ENV_BYTES: u64 = 256 * 1024;

pub(super) struct SessionPaths {
    pub home: PathBuf,
    pub runtime: PathBuf,
}

fn parse_session_paths(bytes: &[u8]) -> Option<SessionPaths> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_PROCESS_ENV_BYTES || bytes.last() != Some(&0) {
        return None;
    }
    let mut home = None;
    let mut runtime = None;
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let (target, value) = if let Some(value) = entry.strip_prefix(b"HOME=") {
            (&mut home, value)
        } else if let Some(value) = entry.strip_prefix(b"XDG_RUNTIME_DIR=") {
            (&mut runtime, value)
        } else {
            continue;
        };
        if value.is_empty() || target.replace(value).is_some() {
            return None;
        }
    }
    Some(SessionPaths {
        home: PathBuf::from(std::ffi::OsString::from_vec(home?.to_vec())),
        runtime: PathBuf::from(std::ffi::OsString::from_vec(runtime?.to_vec())),
    })
}

pub(super) fn read_session_paths(
    proc_root: &Path,
    client: &SteamProcessIdentity,
    uid: u32,
) -> anyhow::Result<SessionPaths> {
    read_session_paths_at(proc_root, client, uid, Path::new("/run/user"))
}

fn read_session_paths_at(
    proc_root: &Path,
    client: &SteamProcessIdentity,
    uid: u32,
    runtime_root: &Path,
) -> anyhow::Result<SessionPaths> {
    anyhow::ensure!(uid != 0, "session owner must be unprivileged");
    let read = || crate::telemetry::proc_view::live_process(proc_root, client.steam_pid);
    let before = read().ok_or_else(|| anyhow::anyhow!("Steam identity unavailable"))?;
    anyhow::ensure!(
        before.uid == uid && before.start_ticks == client.steam_start_ticks,
        "Steam identity changed"
    );
    let file = File::open(proc_root.join(client.steam_pid.to_string()).join("environ"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PROCESS_ENV_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let paths = parse_session_paths(&bytes)
        .ok_or_else(|| anyhow::anyhow!("Steam session paths unavailable"))?;
    anyhow::ensure!(
        paths.home.is_absolute()
            && paths.home.components().all(|part| matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            ))
            && paths.runtime == runtime_root.join(uid.to_string()),
        "invalid Steam session paths"
    );
    let home = File::from(crate::syscall::open_dir_no_symlinks(&paths.home)?).metadata()?;
    let runtime = File::from(crate::syscall::open_dir_no_symlinks(&paths.runtime)?).metadata()?;
    anyhow::ensure!(
        home.uid() == uid && runtime.uid() == uid && runtime.mode() & 0o077 == 0,
        "Steam session path ownership mismatch"
    );
    anyhow::ensure!(
        read().is_some_and(|after| after.uid == uid && after.start_ticks == before.start_ticks),
        "Steam identity changed"
    );
    Ok(paths)
}

#[cfg(test)]
pub(super) fn session_environment_matches(
    bytes: &[u8],
    home: Option<&std::ffi::OsStr>,
    runtime: Option<&std::ffi::OsStr>,
) -> bool {
    parse_session_paths(bytes).is_some_and(|paths| {
        Some(paths.home.as_os_str()) == home && Some(paths.runtime.as_os_str()) == runtime
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[test]
    fn process_identity_and_private_routing_paths_are_rechecked() {
        let tmp = tempfile::tempdir().unwrap();
        let uid = crate::syscall::geteuid();
        let proc = tmp.path().join("proc");
        let home = tmp.path().join("home");
        let runtime_root = tmp.path().join("runtime");
        let runtime = runtime_root.join(uid.to_string());
        for path in [&proc.join("100"), &home, &runtime] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let stat =
            |ticks: u64| format!("100 (steam) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0");
        std::fs::write(proc.join("100/stat"), stat(11)).unwrap();
        let env = format!(
            "HOME={}\0XDG_RUNTIME_DIR={}\0IGNORED=secret\0",
            home.display(),
            runtime.display()
        );
        std::fs::write(proc.join("100/environ"), &env).unwrap();
        let client = SteamProcessIdentity {
            steam_pid: 100,
            steam_start_ticks: 11,
            webhelper_pid: 101,
            webhelper_start_ticks: 12,
            logged_in: true,
        };
        let paths = read_session_paths_at(&proc, &client, uid, &runtime_root).unwrap();
        assert_eq!(paths.home, home);
        assert_eq!(paths.runtime, runtime);
        std::fs::write(proc.join("100/stat"), stat(12)).unwrap();
        assert!(read_session_paths_at(&proc, &client, uid, &runtime_root).is_err());
        std::fs::write(proc.join("100/stat"), stat(11)).unwrap();
        std::fs::write(
            proc.join("100/environ"),
            format!("{env}HOME={}\0", home.display()),
        )
        .unwrap();
        assert!(read_session_paths_at(&proc, &client, uid, &runtime_root).is_err());
        let link = tmp.path().join("home-link");
        symlink(&home, &link).unwrap();
        std::fs::write(
            proc.join("100/environ"),
            format!(
                "HOME={}\0XDG_RUNTIME_DIR={}\0",
                link.display(),
                runtime.display()
            ),
        )
        .unwrap();
        assert!(read_session_paths_at(&proc, &client, uid, &runtime_root).is_err());
        std::fs::write(
            proc.join("100/environ"),
            format!(
                "HOME={}/../home\0XDG_RUNTIME_DIR={}\0",
                home.display(),
                runtime.display()
            ),
        )
        .unwrap();
        assert!(read_session_paths_at(&proc, &client, uid, &runtime_root).is_err());
    }
}
