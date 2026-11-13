//! Experimental `SteamPort` pieces. The candidate Steam launch and native benchmark flag are not
//! qualified. The process-name scan is only a conservative prefilter, not proof of ownership or
//! absence of unrelated games. Result detection and actual game-tree settlement remain unverified.

use std::{
    fs::File,
    io::Read,
    os::{
        fd::{AsFd, AsRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};

use super::{
    AttemptSlot, SteamClientIdentity, SteamProcessIdentity,
    dispatch_context::{DispatchContextV1, inspect_dispatcher_start},
    launch_ticket::SteamVerb,
    steam_host::{GameState, SteamPort},
};

const APP_ID: &str = "1091500";
const MAX_PROC_ENTRIES: usize = 65_536;
const MAX_PROCESS_ENV_BYTES: u64 = 256 * 1024;
const MAX_PROCESS_STATUS_BYTES: u64 = 64 * 1024;

/// A launcher must not carry controller capabilities into Steam. NoNewPrivs also prevents
/// acquiring file capabilities or setuid privilege during exec.
fn unprivileged_launcher_status(status: &str, uid: u32) -> bool {
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
                    && u64::from_str_radix(value, 16).is_ok_and(|caps| caps == 0)
            }
            4 => value == "1",
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

fn launcher_is_unprivileged() -> bool {
    let Ok(file) = File::open("/proc/self/status") else {
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
    std::str::from_utf8(&bytes)
        .is_ok_and(|status| unprivileged_launcher_status(status, crate::syscall::geteuid()))
}

/// Compare only the two session-routing variables; never persist or log the process environment.
fn session_environment_matches(
    bytes: &[u8],
    home: Option<&std::ffi::OsStr>,
    runtime: Option<&std::ffi::OsStr>,
) -> bool {
    if bytes.is_empty() || bytes.len() as u64 > MAX_PROCESS_ENV_BYTES || bytes.last() != Some(&0) {
        return false;
    }
    let mut found_home = None;
    let mut found_runtime = None;
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let (target, value) = if let Some(value) = entry.strip_prefix(b"HOME=") {
            (&mut found_home, value)
        } else if let Some(value) = entry.strip_prefix(b"XDG_RUNTIME_DIR=") {
            (&mut found_runtime, value)
        } else {
            continue;
        };
        if value.is_empty() || target.replace(value).is_some() {
            return false;
        }
    }
    matches!((found_home, home, found_runtime, runtime),
        (Some(a), Some(b), Some(c), Some(d)) if a == b.as_bytes() && c == d.as_bytes())
}
/// Process names (`/proc/<pid>/comm`, at most 15 bytes) whose presence makes the host not free.
const BUSY_COMMS: [&str; 9] = [
    "steam",
    "steamwebhelper",
    "reaper",
    "pressure-vessel",
    "wineserver",
    "wine64-preloader",
    "proton",
    "Cyberpunk2077.e",
    "gamescope",
];

/// Conservative prefilter only: exempts the re-attested Steam client and its descendants, not
/// Wine, game or runtime processes. An independent foreign-workload check is still required.
pub fn host_has_game_or_foreign_steam(
    proc_root: &Path,
    client: &SteamProcessIdentity,
    uid: u32,
) -> bool {
    host_has_game_or_foreign_steam_bounded(proc_root, client, uid, MAX_PROC_ENTRIES)
}

fn host_has_game_or_foreign_steam_bounded(
    proc_root: &Path,
    client: &SteamProcessIdentity,
    uid: u32,
    limit: usize,
) -> bool {
    let read = |pid| crate::telemetry::proc_view::live_process(proc_root, pid);
    let Some(steam) = read(client.steam_pid) else {
        return true;
    };
    let Some(helper) = read(client.webhelper_pid) else {
        return true;
    };
    if steam.uid != uid
        || steam.start_ticks != client.steam_start_ticks
        || helper.uid != uid
        || helper.start_ticks != client.webhelper_start_ticks
        || !helper
            .ancestors
            .contains(&(client.steam_pid, client.steam_start_ticks))
    {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        // An unreadable /proc is never evidence that the host is free.
        return true;
    };
    let mut saw_steam = false;
    let mut saw_helper = false;
    let mut scanned = 0usize;
    for entry in entries {
        let Ok(entry) = entry else { return true };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return true;
        };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        scanned += 1;
        if scanned > limit {
            return true;
        }
        let path = entry.path();
        let file = match std::fs::File::open(path.join("comm")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(&path) {
                    Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => continue,
                    _ => return true,
                }
            }
            Err(_) => return true,
        };
        let mut bytes = Vec::new();
        if file.take(18).read_to_end(&mut bytes).is_err() || bytes.is_empty() || bytes.len() > 16 {
            return true;
        }
        let Ok(comm) = std::str::from_utf8(&bytes) else {
            return true;
        };
        let comm = comm.trim();
        if BUSY_COMMS.contains(&comm) {
            if !matches!(comm, "steam" | "steamwebhelper") {
                return true;
            }
            let Ok(pid) = name.parse::<u32>() else {
                return true;
            };
            let Some(process) = read(pid) else {
                return true;
            };
            let permitted = if comm == "steam" {
                let matches =
                    pid == client.steam_pid && process.start_ticks == client.steam_start_ticks;
                saw_steam |= matches;
                matches
            } else {
                let matches = process
                    .ancestors
                    .contains(&(client.steam_pid, client.steam_start_ticks));
                saw_helper |= matches
                    && pid == client.webhelper_pid
                    && process.start_ticks == client.webhelper_start_ticks;
                matches
            };
            if process.uid != uid || !permitted || read(pid).as_ref() != Some(&process) {
                return true;
            }
        }
    }
    // Recheck the pinned identities after scanning; a vanished or reused client is not idle.
    !saw_steam
        || !saw_helper
        || !read(client.steam_pid)
            .is_some_and(|p| p.uid == uid && p.start_ticks == client.steam_start_ticks)
        || !read(client.webhelper_pid).is_some_and(|p| {
            p.uid == uid
                && p.start_ticks == client.webhelper_start_ticks
                && p.ancestors
                    .contains(&(client.steam_pid, client.steam_start_ticks))
        })
}

pub fn launch_argv() -> Vec<String> {
    ["steam", "-applaunch", APP_ID, "-benchmark"]
        .map(str::to_owned)
        .to_vec()
}

pub trait ResultProbe {
    /// Prepare for a new launch (for example snapshot the native results directory).
    fn begin(&mut self) -> anyhow::Result<()>;
    fn poll(&mut self) -> GameState;
}

pub struct ProductionSteamPort<R: ResultProbe> {
    idle_at: Option<Instant>,
    pub proc_root: PathBuf,
    pub study_compat_data: PathBuf,
    pub study_root: PathBuf,
    pub steamapps: PathBuf,
    pub proton_manifest: PathBuf,
    expected_client: SteamClientIdentity,
    pub tools: Vec<(super::ProtonSource, PathBuf)>,
    pub probe: R,
    child: Option<Child>,
    dispatch: Option<(File, DispatchContextV1)>,
    unsettled: bool,
}

impl<R: ResultProbe> ProductionSteamPort<R> {
    pub fn new(
        study_compat_data: PathBuf,
        study_root: PathBuf,
        steamapps: PathBuf,
        proton_manifest: PathBuf,
        expected_client: SteamClientIdentity,
        tools: Vec<(super::ProtonSource, PathBuf)>,
        probe: R,
    ) -> Self {
        Self {
            idle_at: None,
            proc_root: PathBuf::from("/proc"),
            study_compat_data,
            study_root,
            steamapps,
            proton_manifest,
            expected_client,
            tools,
            probe,
            child: None,
            dispatch: None,
            unsettled: false,
        }
    }

    fn idle_is_fresh(&self) -> bool {
        self.idle_at
            .is_some_and(|at| at.elapsed() <= Duration::from_secs(3))
    }

    fn steam_and_prefilter_clear(&self) -> bool {
        let uid = crate::syscall::geteuid();
        let Ok(libraries) = super::discover_steam_libraries(&self.steamapps, uid) else {
            return false;
        };
        let Ok(inventory) = super::inspect_proton_inventory(
            &self.proton_manifest,
            &self.steamapps,
            &libraries,
            uid,
        ) else {
            return false;
        };
        let Some(observed) = inventory.steam_client else {
            return false;
        };
        if observed != self.expected_client {
            return false;
        }
        observed
            .process
            .as_ref()
            .is_some_and(|client| !host_has_game_or_foreign_steam(&self.proc_root, client, uid))
    }

    fn inherited_session_matches_steam(&self) -> bool {
        let Some(client) = self.expected_client.process.as_ref() else {
            return false;
        };
        let uid = crate::syscall::geteuid();
        let read = || crate::telemetry::proc_view::live_process(&self.proc_root, client.steam_pid);
        let Some(before) = read() else { return false };
        if before.uid != uid || before.start_ticks != client.steam_start_ticks {
            return false;
        }
        let Ok(file) = File::open(
            self.proc_root
                .join(client.steam_pid.to_string())
                .join("environ"),
        ) else {
            return false;
        };
        let mut bytes = Vec::new();
        if file
            .take(MAX_PROCESS_ENV_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return false;
        }
        let home = std::env::var_os("HOME");
        let runtime = std::env::var_os("XDG_RUNTIME_DIR");
        let expected_runtime = format!("/run/user/{uid}");
        if runtime.as_deref() != Some(std::ffi::OsStr::new(&expected_runtime)) {
            return false;
        }
        let Some(home_path) = home.as_deref().map(Path::new) else {
            return false;
        };
        if !home_path.is_absolute() {
            return false;
        }
        let Ok(home_dir) = crate::syscall::open_dir_no_symlinks(home_path) else {
            return false;
        };
        let Ok(home_meta) = File::from(home_dir).metadata() else {
            return false;
        };
        let Ok(runtime_dir) = crate::syscall::open_dir_no_symlinks(Path::new(&expected_runtime))
        else {
            return false;
        };
        let Ok(runtime_meta) = File::from(runtime_dir).metadata() else {
            return false;
        };
        if home_meta.uid() != uid || runtime_meta.uid() != uid || runtime_meta.mode() & 0o077 != 0 {
            return false;
        }
        session_environment_matches(&bytes, home.as_deref(), runtime.as_deref())
            && read()
                .is_some_and(|after| after.uid == uid && after.start_ticks == before.start_ticks)
    }

    fn open_dispatch(
        &self,
        slot: &AttemptSlot,
        context: &DispatchContextV1,
    ) -> anyhow::Result<File> {
        let uid = crate::syscall::geteuid();
        anyhow::ensure!(
            context.slot == *slot
                && context.intent.uid == uid
                && context.intent.steam_client == self.expected_client,
            "dispatch context mismatch"
        );
        let digest = context.intent.digest()?;
        anyhow::ensure!(slot.study_sha256 == digest, "dispatch study mismatch");
        let root = File::from(crate::syscall::open_dir_no_symlinks(&self.study_root)?);
        let meta = root.metadata()?;
        let expected = context
            .intent
            .study_root
            .ok_or_else(|| anyhow::anyhow!("study root is not frozen"))?;
        anyhow::ensure!(
            meta.uid() == uid
                && meta.mode() & 0o077 == 0
                && meta.dev() == expected.device
                && meta.ino() == expected.inode,
            "study root changed"
        );
        let mut dir = root;
        for component in [&digest[..16], "dispatch", &format!("{:04}", slot.index)] {
            dir = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), component)?);
            let meta = dir.metadata()?;
            anyhow::ensure!(
                meta.uid() == uid && meta.mode() & 0o077 == 0,
                "untrusted dispatch ancestry"
            );
        }
        Ok(dir)
    }
}

impl<R: ResultProbe> SteamPort for ProductionSteamPort<R> {
    fn observe_desktop_idle(&mut self, idle_at: Option<Instant>) {
        self.idle_at = idle_at;
    }

    fn safe_window(&mut self) -> bool {
        self.child.is_none()
            && !self.unsettled
            && self.idle_is_fresh()
            && launcher_is_unprivileged()
            && self.steam_and_prefilter_clear()
            && self.inherited_session_matches_steam()
            && self.idle_is_fresh()
    }

    fn tool_dir(&mut self, slot: &AttemptSlot) -> anyhow::Result<PathBuf> {
        self.tools
            .iter()
            .find(|(source, _)| *source == slot.arm)
            .map(|(_, path)| path.clone())
            .ok_or_else(|| anyhow::anyhow!("no verified installation for the scheduled arm"))
    }

    fn study_compat_data(&self) -> PathBuf {
        self.study_compat_data.clone()
    }

    fn library_paths(&self) -> (PathBuf, PathBuf) {
        (self.steamapps.clone(), self.proton_manifest.clone())
    }

    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.child.is_none() && !self.unsettled,
            "a launch remains unsettled"
        );
        anyhow::ensure!(launcher_is_unprivileged(), "BlockedPrivilegedSteamLauncher");
        // Re-check at the last moment: the window can close between admission and launch.
        anyhow::ensure!(
            self.idle_is_fresh() && self.steam_and_prefilter_clear(),
            "safe window closed before launch"
        );
        let dispatch = self.open_dispatch(slot, context)?;
        self.probe.begin()?;
        anyhow::ensure!(
            self.idle_is_fresh(),
            "desktop idle observation expired before launch"
        );
        anyhow::ensure!(
            self.inherited_session_matches_steam() && self.idle_is_fresh(),
            "BlockedSteamSessionEnvironment"
        );
        let argv = launch_argv();
        let child = Command::new(&argv[0]).args(&argv[1..]).spawn()?;
        self.unsettled = true;
        self.child = Some(child);
        self.dispatch = Some((dispatch, context.clone()));
        Ok(())
    }

    fn state(&mut self) -> GameState {
        if !self.idle_is_fresh() {
            return GameState::DesktopInterrupted;
        }
        let Some((dir, context)) = self.dispatch.as_ref() else {
            return GameState::IdentityMismatch;
        };
        let mut active_anchor = false;
        for verb in [SteamVerb::WaitForExitAndRun, SteamVerb::Run] {
            let path = format!(
                "/proc/self/fd/{}/dispatcher-{}.json",
                dir.as_raw_fd(),
                verb.as_steam_argument()
            );
            match std::fs::symlink_metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return GameState::IdentityMismatch,
                Ok(_) => match inspect_dispatcher_start(dir, context, verb, &self.proc_root) {
                    Ok(Some(_)) => active_anchor = true,
                    Ok(None) => {}
                    Err(_) => return GameState::IdentityMismatch,
                },
            }
        }
        if active_anchor {
            self.probe.poll()
        } else {
            GameState::Starting
        }
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        // Only the launcher child this port started; it never signals unrelated processes.
        if let Some(child) = self.child.as_mut() {
            if child.try_wait()?.is_none() {
                child.kill()?;
                child.wait()?;
            }
            self.child = None;
        }
        anyhow::ensure!(
            !self.unsettled,
            "Cyberpunk process-tree settlement is unverified"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc_with(comms: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (pid, comm) in comms.iter().enumerate() {
            let p = dir.path().join((pid + 100).to_string());
            std::fs::create_dir(&p).unwrap();
            std::fs::write(p.join("comm"), format!("{comm}\n")).unwrap();
        }
        std::fs::create_dir(dir.path().join("self")).unwrap();
        dir
    }

    #[test]
    fn only_the_frozen_client_tree_is_exempt_from_the_prefilter() {
        fn add(proc: &Path, pid: u32, parent: u32, ticks: u64, comm: &str) {
            let path = proc.join(pid.to_string());
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(
                path.join("stat"),
                format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0"),
            )
            .unwrap();
        }
        let proc = tempfile::tempdir().unwrap();
        add(proc.path(), 100, 1, 11, "steam");
        add(proc.path(), 101, 100, 12, "steamwebhelper");
        add(proc.path(), 102, 101, 13, "steamwebhelper");
        let client = SteamProcessIdentity {
            steam_pid: 100,
            steam_start_ticks: 11,
            webhelper_pid: 101,
            webhelper_start_ticks: 12,
            logged_in: true,
        };
        let uid = crate::syscall::geteuid();
        assert!(!host_has_game_or_foreign_steam(proc.path(), &client, uid));
        assert!(host_has_game_or_foreign_steam_bounded(
            proc.path(),
            &client,
            uid,
            2
        ));
        assert!(host_has_game_or_foreign_steam(
            proc.path(),
            &client,
            uid + 1
        ));
        add(proc.path(), 103, 1, 14, "steam");
        assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
        std::fs::remove_dir_all(proc.path().join("103")).unwrap();
        add(proc.path(), 103, 100, 14, "steam");
        assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
        std::fs::remove_dir_all(proc.path().join("103")).unwrap();
        add(proc.path(), 103, 100, 14, "wineserver");
        assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
        std::fs::remove_dir_all(proc.path().join("103")).unwrap();
        let mut reused = client.clone();
        reused.steam_start_ticks += 1;
        assert!(host_has_game_or_foreign_steam(proc.path(), &reused, uid));
        std::fs::remove_file(proc.path().join("102/comm")).unwrap();
        assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
        std::fs::write(proc.path().join("102/comm"), "steamwebhelper\n").unwrap();
        std::fs::remove_file(proc.path().join("102/stat")).unwrap();
        assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
        assert!(host_has_game_or_foreign_steam(
            Path::new("/definitely/not/proc"),
            &client,
            uid
        ));
    }

    struct Probe;
    impl ResultProbe for Probe {
        fn begin(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        fn poll(&mut self) -> GameState {
            GameState::Running
        }
    }

    #[test]
    fn session_environment_requires_exact_unique_home_and_runtime() {
        use std::ffi::OsStr;
        let home = Some(OsStr::new("/home/user"));
        let runtime = Some(OsStr::new("/run/user/1000"));
        let good = b"HOME=/home/user\0XDG_RUNTIME_DIR=/run/user/1000\0OTHER=redacted\0";
        assert!(session_environment_matches(good, home, runtime));
        assert!(!session_environment_matches(
            good,
            Some(OsStr::new("/var/lib/stutter")),
            runtime
        ));
        assert!(!session_environment_matches(good, home, None));
        assert!(!session_environment_matches(
            b"HOME=/home/user\0HOME=/home/user\0XDG_RUNTIME_DIR=/run/user/1000\0",
            home,
            runtime
        ));
        assert!(!session_environment_matches(
            b"HOME=/home/user\0XDG_RUNTIME_DIR=/run/user/1000",
            home,
            runtime
        ));
    }

    #[test]
    fn launcher_requires_zero_capabilities_fixed_uid_and_no_new_privileges() {
        let safe = "Uid:\t1000 1000 1000 1000\nCapInh:\t0000000000000000\nCapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapAmb:\t0000000000000000\nNoNewPrivs:\t1\n";
        assert!(unprivileged_launcher_status(safe, 1000));
        assert!(!unprivileged_launcher_status(safe, 0));
        assert!(!unprivileged_launcher_status(
            &safe.replace("CapAmb:\t0000000000000000", "CapAmb:\t0000000000000001"),
            1000
        ));
        assert!(!unprivileged_launcher_status(
            &safe.replace("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
            1000
        ));
        assert!(!unprivileged_launcher_status(
            &safe.replace("Uid:\t1000 1000 1000 1000", "Uid:\t0 1000 0 1000"),
            1000
        ));
        assert!(!unprivileged_launcher_status(
            &safe.replace("CapPrm:\t0000000000000000\n", ""),
            1000
        ));
        assert!(!unprivileged_launcher_status(
            &format!("{safe}CapEff:\t0000000000000000\n"),
            1000
        ));
    }

    #[test]
    fn launch_refuses_unless_idle_and_free_and_the_command_is_the_one_owned_app() {
        let proc = proc_with(&["zsh"]);
        let mut port = ProductionSteamPort::new(
            "/s".into(),
            "/study".into(),
            "/a".into(),
            "/m".into(),
            super::super::tests::intent().steam_client,
            vec![(super::super::ProtonSource::Ge, "/t/ge".into())],
            Probe,
        );
        port.proc_root = proc.path().to_path_buf();
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: super::super::ProtonSource::Ge,
        };
        let context = DispatchContextV1 {
            schema_version: 1,
            intent: super::super::tests::intent(),
            slot: slot.clone(),
            tickets: Default::default(),
            spec: super::super::dispatch_context::ArmSpec {
                tool_dir: "/t/ge".into(),
                study_compat_data: "/s".into(),
                arm_env: Default::default(),
                steamapps: "/a".into(),
                proton_manifest: "/m".into(),
            },
        };
        assert!(!port.safe_window());
        assert!(port.launch(&slot, &context).is_err());
        port.observe_desktop_idle(Some(Instant::now()));
        // Idle alone cannot qualify the client: the fixture has no trusted Steam inventory.
        assert!(!port.safe_window());
        port.observe_desktop_idle(Some(Instant::now() - Duration::from_secs(4)));
        assert!(!port.safe_window());
        assert_eq!(port.state(), GameState::DesktopInterrupted);
        port.observe_desktop_idle(None);
        assert_eq!(port.state(), GameState::DesktopInterrupted);
        port.observe_desktop_idle(Some(Instant::now()));
        assert_eq!(port.tool_dir(&slot).unwrap(), PathBuf::from("/t/ge"));
        let mut other = slot.clone();
        other.arm = super::super::ProtonSource::Em;
        assert!(port.tool_dir(&other).is_err());
        assert_eq!(
            launch_argv(),
            ["steam", "-applaunch", "1091500", "-benchmark"]
        );
        assert!(port.stop().is_ok());
        port.unsettled = true;
        assert!(port.stop().is_err());
        assert!(!port.safe_window());
        assert!(port.launch(&slot, &context).is_err());
    }

    #[test]
    fn exited_first_steam_verb_never_completes_an_attempt_with_a_live_second_verb() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        use super::super::dispatch_context::{
            ArmSpec, DispatcherStartV1, arm_attempt, authorize_dispatch,
        };

        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let attempt = File::from(crate::syscall::open_dir_no_symlinks(dir.path()).unwrap());
        let mut intent = super::super::tests::intent();
        intent.uid = crate::syscall::geteuid();
        let steam = intent.steam_client.process.as_mut().unwrap();
        steam.steam_pid = 200;
        steam.steam_start_ticks = 40;
        steam.webhelper_pid = 201;
        steam.webhelper_start_ticks = 41;
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: intent.tools[0].source,
        };
        let spec = ArmSpec {
            tool_dir: "/tools/GE".into(),
            study_compat_data: "/study/compatdata".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        };
        let now = intent.issued_unix_seconds + 1;
        let context = arm_attempt(&attempt, &intent, &slot, spec, (now, 60)).unwrap();
        let env = [
            ("SteamAppId", "1091500"),
            ("STEAM_COMPAT_DATA_PATH", "/study/compatdata"),
            ("STEAM_COMPAT_CLIENT_INSTALL_PATH", "/home/u/.steam/steam"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .into();
        let proc = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(proc.path().join("sys/kernel/random")).unwrap();
        std::fs::write(
            proc.path().join("sys/kernel/random/boot_id"),
            &intent.boot_id,
        )
        .unwrap();
        for pid in [123, 200, 201] {
            std::fs::create_dir_all(proc.path().join(pid.to_string()).join("ns")).unwrap();
        }
        std::fs::create_dir_all(proc.path().join("self/ns")).unwrap();
        let ns = proc.path().join("self/ns/pid");
        std::fs::write(&ns, b"namespace").unwrap();
        std::fs::hard_link(&ns, proc.path().join("123/ns/pid")).unwrap();
        let stat = |pid: u32, parent: u32, ticks: u64| {
            format!("{pid} (fixture) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0")
        };
        std::fs::write(proc.path().join("200/stat"), stat(200, 1, 40)).unwrap();
        std::fs::write(proc.path().join("201/stat"), stat(201, 200, 41)).unwrap();
        std::fs::write(proc.path().join("123/stat"), stat(123, 200, 80)).unwrap();
        let ns_meta = std::fs::metadata(&ns).unwrap();
        for (verb, pid, ticks) in [
            (SteamVerb::WaitForExitAndRun, 124, 79),
            (SteamVerb::Run, 123, 80),
        ] {
            let argv = ["/x/proton", verb.as_steam_argument(), "/g/game.exe"].map(str::to_owned);
            let (_, ticket) =
                authorize_dispatch(&attempt, &argv, &env, intent.uid, now + 1, &|_, _| Ok(()))
                    .unwrap();
            let record = DispatcherStartV1 {
                schema_version: 1,
                ticket_id: ticket.ticket_id,
                intent_sha256: ticket.intent_sha256,
                attempt_sha256: ticket.attempt_sha256,
                verb,
                uid: intent.uid,
                boot_id: intent.boot_id.clone(),
                observed_unix_seconds: now + 1,
                pid,
                start_ticks: ticks,
                pid_namespace_device: ns_meta.dev(),
                pid_namespace_inode: ns_meta.ino(),
            };
            super::super::report_store::write_new(
                &attempt,
                &format!("dispatcher-{}.json", verb.as_steam_argument()),
                &serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
        }
        let mut port = ProductionSteamPort::new(
            "/study/compatdata".into(),
            "/study".into(),
            "/a".into(),
            "/m".into(),
            intent.steam_client.clone(),
            vec![],
            Probe,
        );
        port.proc_root = proc.path().to_path_buf();
        port.observe_desktop_idle(Some(Instant::now()));
        port.dispatch = Some((attempt, context));
        assert_eq!(port.state(), GameState::Running);
        std::fs::remove_dir_all(proc.path().join("123")).unwrap();
        assert_eq!(port.state(), GameState::Starting);
    }
}
