//! Experimental `SteamPort` pieces. The candidate Steam launch and native benchmark flag are not
//! qualified. The process-name scan is only a conservative prefilter, not proof of ownership or
//! absence of unrelated games. Result detection and actual game-tree settlement remain unverified.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
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
    for entry in entries {
        let Ok(entry) = entry else { return true };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return true;
        };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
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
        if BUSY_COMMS.contains(&comm.trim()) {
            return true;
        }
    }
    false
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
    pub steamapps: PathBuf,
    pub proton_manifest: PathBuf,
    pub tools: Vec<(super::ProtonSource, PathBuf)>,
    pub probe: R,
    child: Option<Child>,
    unsettled: bool,
}

impl<R: ResultProbe> ProductionSteamPort<R> {
    pub fn new(
        study_compat_data: PathBuf,
        steamapps: PathBuf,
        proton_manifest: PathBuf,
        tools: Vec<(super::ProtonSource, PathBuf)>,
        probe: R,
    ) -> Self {
        Self {
            idle_at: None,
            proc_root: PathBuf::from("/proc"),
            study_compat_data,
            steamapps,
            proton_manifest,
            tools,
            probe,
            child: None,
            unsettled: false,
        }
    }

    fn idle_is_fresh(&self) -> bool {
        self.idle_at
            .is_some_and(|at| at.elapsed() <= Duration::from_secs(3))
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
        anyhow::ensure!(
            self.child.is_none() && !self.unsettled,
            "a launch remains unsettled"
        );
        // Re-check at the last moment: the window can close between admission and launch.
        anyhow::ensure!(
            self.idle_is_fresh() && !host_has_game_or_steam(&self.proc_root),
            "safe window closed before launch"
        );
        self.probe.begin()?;
        let argv = launch_argv();
        let child = Command::new(&argv[0]).args(&argv[1..]).spawn()?;
        self.unsettled = true;
        self.child = Some(child);
        Ok(())
    }

    fn state(&mut self) -> GameState {
        if !self.idle_is_fresh() {
            return GameState::DesktopInterrupted;
        }
        self.probe.poll()
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
        let unreadable = proc_with(&["zsh"]);
        std::fs::remove_file(unreadable.path().join("100/comm")).unwrap();
        assert!(host_has_game_or_steam(unreadable.path()));
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
        let mut port = ProductionSteamPort::new(
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
        port.observe_desktop_idle(Some(Instant::now()));
        assert!(port.safe_window());
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
        assert!(port.launch(&slot).is_err());
    }
}
