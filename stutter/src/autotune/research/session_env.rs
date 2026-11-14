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
const MAX_PROCESS_STATUS_BYTES: u64 = 64 * 1024;

#[cfg(test)]
pub(super) fn capability_status(status: &str, uid: u32, require_no_new_privs: bool) -> bool {
    capability_status_inner(status, uid, require_no_new_privs, false)
}

fn capability_status_inner(
    status: &str,
    uid: u32,
    require_no_new_privs: bool,
    allow_inheritable: bool,
) -> bool {
    if uid == 0 {
        return false;
    }
    let mut seen = [false; 6];
    for line in status.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let index = match key {
            "CapInh" => 0,
            "CapPrm" => 1,
            "CapEff" => 2,
            "CapAmb" => 3,
            "NoNewPrivs" => 4,
            "Uid" => 5,
            _ => continue,
        };
        if std::mem::replace(&mut seen[index], true) {
            return false;
        }
        let valid = match index {
            0..=3 => {
                !value.is_empty()
                    && value.len() <= 16
                    && u64::from_str_radix(value, 16)
                        .is_ok_and(|caps| caps == 0 || (index == 0 && allow_inheritable))
            }
            4 => value == "1" || (!require_no_new_privs && value == "0"),
            5 => {
                let ids: Vec<_> = value.split_whitespace().collect();
                ids.len() == 4
                    && ids
                        .iter()
                        .all(|id| id.parse::<u32>().is_ok_and(|id| id == uid))
            }
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    seen.into_iter().all(|value| value)
}

pub(super) fn process_capabilities_clear(
    proc_root: &Path,
    pid: u32,
    uid: u32,
    require_no_new_privs: bool,
) -> bool {
    process_capabilities(proc_root, pid, uid, require_no_new_privs, false)
}

/// A pre-existing broker is not the Steam launcher. It may have inheritable bits, but must not
/// have permitted, effective or ambient capabilities; launcher checks remain stricter.
pub(super) fn broker_capabilities_clear(proc_root: &Path, pid: u32, uid: u32) -> bool {
    process_capabilities(proc_root, pid, uid, false, true)
}

fn process_capabilities(
    proc_root: &Path,
    pid: u32,
    uid: u32,
    require_no_new_privs: bool,
    allow_inheritable: bool,
) -> bool {
    let Ok(file) = File::open(proc_root.join(pid.to_string()).join("status")) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file
        .take(MAX_PROCESS_STATUS_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > MAX_PROCESS_STATUS_BYTES
    {
        return false;
    }
    std::str::from_utf8(&bytes).is_ok_and(|status| {
        capability_status_inner(status, uid, require_no_new_privs, allow_inheritable)
    })
}

pub(super) struct SessionPaths {
    pub home: PathBuf,
    pub runtime: PathBuf,
    pub runtime_device: u64,
    pub runtime_inode: u64,
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
        runtime_device: 0,
        runtime_inode: 0,
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
    let mut paths = parse_session_paths(&bytes)
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
    paths.runtime_device = runtime.dev();
    paths.runtime_inode = runtime.ino();
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

    #[test]
    fn manager_capability_check_accepts_no_new_privs_zero_but_never_capabilities() {
        let status = "Uid:\t1000 1000 1000 1000\nCapInh:\t0\nCapPrm:\t0\nCapEff:\t0\nCapAmb:\t0\nNoNewPrivs:\t0\n";
        assert!(capability_status(status, 1000, false));
        assert!(!capability_status(status, 1000, true));
        assert!(!capability_status(
            &status.replace("CapEff:\t0", "CapEff:\t1"),
            1000,
            false
        ));
        let inherited = status.replace("CapInh:\t0", "CapInh:\t800000000");
        assert!(capability_status_inner(&inherited, 1000, false, true));
        assert!(!capability_status(&inherited, 1000, false));
        assert!(!capability_status_inner(
            &inherited.replace("CapPrm:\t0", "CapPrm:\t1"),
            1000,
            false,
            true
        ));
    }

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
