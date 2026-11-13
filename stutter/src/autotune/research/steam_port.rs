//! Experimental `SteamPort` pieces. The candidate Steam launch and native benchmark flag are not
//! qualified. The process-name scan is only a conservative prefilter, not proof of ownership or
//! absence of unrelated games. Result detection and actual game-tree settlement remain unverified.

use std::{
    fs::File,
    io::Read,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
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
const MAX_PROCESS_STATUS_BYTES: u64 = 64 * 1024;

fn launch_window_open(intent: &super::ResearchIntentV1, now: std::time::SystemTime) -> bool {
    now.duration_since(std::time::UNIX_EPOCH)
        .is_ok_and(|elapsed| {
            elapsed.as_secs() >= intent.issued_unix_seconds
                && elapsed.as_secs() < intent.deadline_unix_seconds
        })
}

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
        let Ok(paths) = super::session_env::read_session_paths(
            &self.proc_root,
            client,
            crate::syscall::geteuid(),
        ) else {
            return false;
        };
        std::env::var_os("HOME").as_deref() == Some(paths.home.as_os_str())
            && std::env::var_os("XDG_RUNTIME_DIR").as_deref() == Some(paths.runtime.as_os_str())
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
        anyhow::ensure!(
            launch_window_open(&context.intent, std::time::SystemTime::now()),
            "campaign authorization expired before Steam launch"
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
#[path = "steam_port_tests.rs"]
mod tests;
