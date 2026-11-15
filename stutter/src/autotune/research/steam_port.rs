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
    time::{Duration, Instant},
};

use super::{
    AttemptSlot, SteamClientIdentity, SteamProcessIdentity,
    dispatch_context::{DispatchContextV1, inspect_dispatcher_start},
    driver::LaunchUnresolved,
    launch_ticket::SteamVerb,
    launch_txn::UserManagerPort,
    steam_host::{GameState, SteamPort},
    transient_launch::launch_through_manager,
    user_launcher::UserLaunchRequest,
};

pub type PrepareRequest =
    Box<dyn FnMut(&AttemptSlot, &DispatchContextV1) -> anyhow::Result<UserLaunchRequest> + Send>;

/// Independent proof that an attempt's launch request, process tree, worker transactions and
/// restoration are all settled. It is never derived from the attempt's own claims or results.
pub trait SettlementWitness: Send {
    fn settled(&mut self, dispatch: &File, context: &DispatchContextV1) -> anyhow::Result<bool>;
}

/// Terminates the attempt's own game process, and only that one, on cancellation or timeout.
/// Implementations must refuse to signal anything they cannot attribute to this attempt.
pub trait OwnedTerminator: Send {
    fn terminate_owned(
        &mut self,
        dispatch: &File,
        context: &DispatchContextV1,
    ) -> anyhow::Result<()>;
}

/// Starts an attempt through the authenticated user manager instead of spawning Steam directly.
/// Both parts are supplied by the caller: the manager port and the fixed-request preparer.
pub struct TransientLauncher {
    pub manager: Box<dyn UserManagerPort + Send>,
    pub prepare: PrepareRequest,
}

const APP_ID: &str = "1091500";
const MAX_PROC_ENTRIES: usize = 65_536;

fn launch_window_open(intent: &super::ResearchIntentV1, now: std::time::SystemTime) -> bool {
    now.duration_since(std::time::UNIX_EPOCH)
        .is_ok_and(|elapsed| {
            elapsed.as_secs() >= intent.issued_unix_seconds
                && elapsed.as_secs() < intent.deadline_unix_seconds
        })
}
fn launcher_is_unprivileged() -> bool {
    super::session_env::process_capabilities_clear(
        Path::new("/proc"),
        std::process::id(),
        crate::syscall::geteuid(),
        true,
    )
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
    fn begin(&mut self, arm: super::ProtonSource) -> anyhow::Result<()>;
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
    dispatch: Option<(File, DispatchContextV1)>,
    unsettled: bool,
    transient: Option<TransientLauncher>,
    witness: Option<Box<dyn SettlementWitness>>,
    terminator: Option<Box<dyn OwnedTerminator>>,
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
            dispatch: None,
            unsettled: false,
            transient: None,
            witness: None,
            terminator: None,
        }
    }

    /// The bounded, exact-owner cleanup used by `stop`; without one nothing is ever signalled.
    pub fn with_owned_terminator(mut self, terminator: Box<dyn OwnedTerminator>) -> Self {
        self.terminator = Some(terminator);
        self
    }

    /// The independent witness that may clear `unsettled`; without one the port never settles.
    pub fn with_settlement_witness(mut self, witness: Box<dyn SettlementWitness>) -> Self {
        self.witness = Some(witness);
        self
    }

    /// Launch through the user manager rather than a direct child spawn.
    pub fn with_transient_launcher(mut self, launcher: TransientLauncher) -> Self {
        self.transient = Some(launcher);
        self
    }

    /// The manager-owned launch. Once a request may have been sent the port stays unsettled, and
    /// only a verified refusal (nothing sent, nothing pending) clears it again.
    fn start_unit(
        &mut self,
        dispatch: File,
        slot: &AttemptSlot,
        context: &DispatchContextV1,
    ) -> anyhow::Result<()> {
        let launcher = self
            .transient
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no transient launcher configured"))?;
        // Nothing has been sent yet, so a preparation failure is a plain refusal.
        let request = (launcher.prepare)(slot, context)?;
        self.unsettled = true;
        self.dispatch = Some((dispatch.try_clone()?, context.clone()));
        let result = launch_through_manager(&mut *launcher.manager, &dispatch, &request);
        if matches!(&result, Err(error) if error.downcast_ref::<LaunchUnresolved>().is_none()) {
            self.unsettled = false;
            self.dispatch = None;
        }
        result
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
        !self.unsettled
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
        // Production launches go only through the authenticated user manager. A missing
        // launcher blocks; it never selects another launch method.
        anyhow::ensure!(self.transient.is_some(), "BlockedTransientLauncherRequired");
        anyhow::ensure!(!self.unsettled, "a launch remains unsettled");
        anyhow::ensure!(launcher_is_unprivileged(), "BlockedPrivilegedSteamLauncher");
        anyhow::ensure!(
            launch_window_open(&context.intent, std::time::SystemTime::now()),
            "campaign authorization expired before Steam launch"
        );
        // Re-check at the last moment: the window can close between admission and launch.
        anyhow::ensure!(
            self.idle_is_fresh() && self.steam_and_prefilter_clear(),
            "safe window closed before launch"
        );
        let dispatch = self.open_dispatch(slot, context)?;
        self.probe.begin(slot.arm)?;
        anyhow::ensure!(
            self.idle_is_fresh(),
            "desktop idle observation expired before launch"
        );
        anyhow::ensure!(
            self.inherited_session_matches_steam() && self.idle_is_fresh(),
            "BlockedSteamSessionEnvironment"
        );
        self.start_unit(dispatch, slot, context)
    }

    fn state(&mut self) -> GameState {
        if !self.idle_is_fresh() {
            return GameState::DesktopInterrupted;
        }
        let Some((dir, context)) = self.dispatch.as_ref() else {
            return GameState::IdentityMismatch;
        };
        // Authenticated dispatcher records establish this attempt's launch ancestry. Steam's
        // dispatcher may exit while the game keeps running, so a recorded dispatcher that is no
        // longer alive must not hide the game: the independent observer still has to attribute
        // the exact game process and result before anything completes.
        let mut recorded = false;
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
                    Ok(Some(_) | None) => recorded = true,
                    Err(_) => return GameState::IdentityMismatch,
                },
            }
        }
        if recorded {
            self.probe.poll()
        } else {
            GameState::Starting
        }
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        // Cleanup is best effort and exact-owner only. A refusal or failure leaves the port
        // unsettled; it never makes the attempt look settled.
        if self.unsettled
            && let (Some(terminator), Some((dir, context))) =
                (self.terminator.as_mut(), self.dispatch.as_ref())
            && let Err(error) = terminator.terminate_owned(dir, context)
        {
            log::warn!("owned cleanup did not complete: {error:#}");
        }
        // Only an independent witness clears the flag. Native output, an exited launcher or a
        // missing observation never does; a failing witness leaves the port unsettled.
        if self.unsettled
            && let (Some(witness), Some((dir, context))) =
                (self.witness.as_mut(), self.dispatch.as_ref())
            && witness.settled(dir, context).unwrap_or(false)
        {
            self.unsettled = false;
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
