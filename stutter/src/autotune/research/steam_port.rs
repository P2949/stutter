//! Production `SteamPort` pieces. Launching is a plain, unprivileged `steam -applaunch` of the one
//! owned app, gated on an externally supplied idle signal AND an independent scan showing no Steam,
//! Wine/Proton or Cyberpunk process at all, so an unrelated game or user session is never touched.
//! Result detection is injected (`ResultProbe`): it must verify a fresh native result for exactly
//! this launch, and is wired to `cyberpunk::verify_benchmark_completion` after live qualification.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use super::{
    AttemptSlot,
    steam_host::{GameState, SteamPort},
};

const APP_ID: &str = "1091500";
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

/// True when any process on the host is Steam, a Wine/Proton runtime or the game.
pub fn host_has_game_or_steam(proc_root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        // An unreadable /proc is never evidence that the host is free.
        return true;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.bytes().all(|b| b.is_ascii_digit()))
            && std::fs::read_to_string(entry.path().join("comm"))
                .is_ok_and(|comm| BUSY_COMMS.contains(&comm.trim()))
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
    pub idle: Arc<AtomicBool>,
    pub proc_root: PathBuf,
    pub study_compat_data: PathBuf,
    pub steamapps: PathBuf,
    pub proton_manifest: PathBuf,
    pub tools: Vec<(super::ProtonSource, PathBuf)>,
    pub probe: R,
    child: Option<Child>,
}

impl<R: ResultProbe> ProductionSteamPort<R> {
    pub fn new(
        idle: Arc<AtomicBool>,
        study_compat_data: PathBuf,
        steamapps: PathBuf,
        proton_manifest: PathBuf,
        tools: Vec<(super::ProtonSource, PathBuf)>,
        probe: R,
    ) -> Self {
        Self {
            idle,
            proc_root: PathBuf::from("/proc"),
            study_compat_data,
            steamapps,
            proton_manifest,
            tools,
            probe,
            child: None,
        }
    }
}

impl<R: ResultProbe> SteamPort for ProductionSteamPort<R> {
    fn safe_window(&mut self) -> bool {
        self.child.is_none()
            && self.idle.load(Ordering::Acquire)
            && !host_has_game_or_steam(&self.proc_root)
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

    fn launch(&mut self, _: &AttemptSlot) -> anyhow::Result<()> {
        anyhow::ensure!(self.child.is_none(), "a launch is already live");
        // Re-check at the last moment: the window can close between admission and launch.
        anyhow::ensure!(
            self.idle.load(Ordering::Acquire) && !host_has_game_or_steam(&self.proc_root),
            "safe window closed before launch"
        );
        self.probe.begin()?;
        let argv = launch_argv();
        let child = Command::new(&argv[0]).args(&argv[1..]).spawn()?;
        self.child = Some(child);
        Ok(())
    }

    fn state(&mut self) -> GameState {
        self.probe.poll()
    }

    fn stop(&mut self) {
        // Only the launcher child this port started; it never signals unrelated processes.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
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
    fn any_steam_wine_or_game_process_makes_the_host_busy() {
        assert!(!host_has_game_or_steam(
            proc_with(&["zsh", "firefox"]).path()
        ));
        for busy in ["steam", "wineserver", "Cyberpunk2077.e", "gamescope"] {
            assert!(
                host_has_game_or_steam(proc_with(&["zsh", busy]).path()),
                "{busy}"
            );
        }
        assert!(host_has_game_or_steam(Path::new("/definitely/not/proc")));
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
    fn launch_refuses_unless_idle_and_free_and_the_command_is_the_one_owned_app() {
        let proc = proc_with(&["zsh"]);
        let idle = Arc::new(AtomicBool::new(false));
        let mut port = ProductionSteamPort::new(
            idle.clone(),
            "/s".into(),
            "/a".into(),
            "/m".into(),
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
        assert!(!port.safe_window());
        assert!(port.launch(&slot).is_err());
        idle.store(true, Ordering::Release);
        assert!(port.safe_window());
        assert_eq!(port.tool_dir(&slot).unwrap(), PathBuf::from("/t/ge"));
        let mut other = slot.clone();
        other.arm = super::super::ProtonSource::Em;
        assert!(port.tool_dir(&other).is_err());
        assert_eq!(
            launch_argv(),
            ["steam", "-applaunch", "1091500", "-benchmark"]
        );
    }
}
