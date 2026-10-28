//! The soak's world: one workload at a time on a contended CPU, the real controller and the real
//! privileged worker as separate processes, an isolated state directory, and a delegated cgroup
//! subtree that holds the administrator's game cgroup.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, bail, ensure};

use super::plan::{Operations, Timing};
use crate::live_acceptance::Unprivileged;

/// A child that never outlives its owner.
pub(super) struct OwnedChild(pub(super) Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl OwnedChild {
    pub(super) fn pid(&self) -> u32 {
        self.0.id()
    }
}

pub(super) fn signal(pid: u32, signal: i32) {
    // SAFETY: kill(2) takes plain integers and has no memory effects.
    unsafe {
        libc::kill(pid as i32, signal);
    }
}

pub(super) struct World {
    pub(super) root: PathBuf,
    pub(super) dir: PathBuf,
    pub(super) stutter: PathBuf,
    pub(super) xtask: PathBuf,
    /// The workload binary, installed under a Steam-like path so the scorer classes it a game.
    pub(super) game: PathBuf,
    /// The game's CPU and the quiet destination CPU.
    pub(super) cpus: (u32, u32),
    /// Short: unix socket paths are limited to about 100 bytes.
    pub(super) socket: PathBuf,
    pub(super) worker: Option<OwnedChild>,
}

pub(super) struct Controller {
    pub(super) child: OwnedChild,
    pub(super) started_unix_ms: u64,
}

pub(super) struct Game {
    pub(super) child: OwnedChild,
    pub(super) ticks: u64,
}

impl World {
    pub(super) fn new(
        root: &Path,
        dir: &Path,
        stutter: &Path,
        cpus: (u32, u32),
    ) -> anyhow::Result<Self> {
        let game_dir = dir.join("game/steamapps/common/StutterAcceptanceGame");
        fs::create_dir_all(&game_dir)?;
        let game = game_dir.join("StutterAcceptanceGame");
        let xtask = std::env::current_exe()?;
        if !game.exists() {
            fs::copy(&xtask, &game).context("install the workload binary")?;
        }
        let socket = PathBuf::from(format!("/tmp/stutter-soak-{}/w.sock", std::process::id()));
        fs::create_dir_all(socket.parent().context("socket parent")?)?;
        fs::create_dir_all(dir.join("home"))?;
        Ok(Self {
            root: root.to_owned(),
            dir: dir.to_owned(),
            stutter: stutter.to_owned(),
            xtask,
            game,
            cpus,
            socket,
            worker: None,
        })
    }

    pub(super) fn state_dir(&self) -> PathBuf {
        self.dir.join("home/.local/state/stutter")
    }

    pub(super) fn autotune_dir(&self) -> PathBuf {
        self.state_dir().join("autotune")
    }

    pub(super) fn journal_path(&self) -> PathBuf {
        self.autotune_dir().join("controller_journal.json")
    }

    pub(super) fn ledger_path(&self) -> PathBuf {
        self.autotune_dir().join("kept-actions.json")
    }

    pub(super) fn history_path(&self) -> PathBuf {
        self.autotune_dir().join("history.jsonl")
    }

    pub(super) fn audit_path(&self) -> PathBuf {
        self.state_dir().join("audit/actions.jsonl")
    }

    /// The administrator's game cgroup: a sibling of the leaf this process runs in, with its own
    /// CPU (the quiet one).
    pub(super) fn game_cgroup(&self) -> anyhow::Result<PathBuf> {
        let text = fs::read_to_string("/proc/self/cgroup")?;
        let own = text
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .context("cgroup v2 path missing")?;
        let parent = Path::new(own.trim())
            .parent()
            .filter(|parent| parent.components().count() > 1)
            .context("the soak does not run in its delegated cgroup subtree")?;
        Ok(parent.join("target"))
    }

    pub(super) fn prepare_game_cgroup(&self) -> anyhow::Result<()> {
        let relative = self.game_cgroup()?;
        let dir = Path::new("/sys/fs/cgroup").join(relative.strip_prefix("/")?);
        fs::create_dir_all(&dir)?;
        let parent = dir.parent().context("target has no parent")?;
        fs::write(
            dir.join("cpuset.mems"),
            fs::read_to_string(parent.join("cpuset.mems.effective"))?,
        )?;
        fs::write(dir.join("cpuset.cpus"), self.cpus.1.to_string())?;
        Ok(())
    }

    /// Every task currently in the game cgroup.
    pub(super) fn game_cgroup_procs(&self) -> Vec<u32> {
        let Ok(relative) = self.game_cgroup() else {
            return Vec::new();
        };
        let path = Path::new("/sys/fs/cgroup")
            .join(relative.strip_prefix("/").unwrap_or(&relative))
            .join("cgroup.procs");
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }

    pub(super) fn ensure_worker(&mut self) -> anyhow::Result<bool> {
        if let Some(worker) = self.worker.as_mut()
            && worker.0.try_wait()?.is_none()
            && self.socket.exists()
        {
            return Ok(false);
        }
        self.worker = None;
        let _ = fs::remove_file(&self.socket);
        let child = Command::new(&self.stutter)
            .args(["privileged-worker", "--socket"])
            .arg(&self.socket)
            .current_dir(&self.root)
            .env("HOME", self.dir.join("home"))
            .stdout(fs::File::create(self.dir.join("worker.out"))?)
            .stderr(fs::File::create(self.dir.join("worker.err"))?)
            .spawn()
            .context("start the privileged worker")?;
        self.worker = Some(OwnedChild(child));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.socket.exists() {
            ensure!(
                Instant::now() < deadline,
                "the privileged worker never bound its socket"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(true)
    }

    pub(super) fn kill_worker(&mut self) {
        self.worker = None;
    }

    pub(super) fn spawn_game(&self, dir: &Path, name: &str, seconds: u64) -> anyhow::Result<Game> {
        let ready = dir.join(format!("{name}.ready"));
        let child = Command::new("taskset")
            .args(["-c", &self.cpus.0.to_string()])
            .arg(&self.game)
            .args([
                "live-workload",
                "--seconds",
                &seconds.to_string(),
                "--ready",
            ])
            .arg(&ready)
            .current_dir(&self.root)
            .stdout(Stdio::null())
            .stderr(fs::File::create(dir.join(format!("{name}.stderr")))?)
            .unprivileged()
            .spawn()
            .context("start the workload")?;
        let mut game = OwnedChild(child);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            ensure!(
                game.0.try_wait()?.is_none(),
                "the workload exited before it was ready"
            );
            ensure!(Instant::now() < deadline, "the workload was never ready");
            std::thread::sleep(Duration::from_millis(20));
        }
        let ticks = start_ticks(game.pid())?;
        Ok(Game { child: game, ticks })
    }

    /// A SCHED_FIFO contender holding `burst_ms` of every 20 ms of `cpu`; none for 0.
    pub(super) fn spawn_burst(
        &self,
        cpu: u32,
        burst_ms: u64,
        seconds: u64,
    ) -> anyhow::Result<Option<OwnedChild>> {
        if burst_ms == 0 {
            return Ok(None);
        }
        let child = Command::new("chrt")
            .args(["-f", "50", "taskset", "-c", &cpu.to_string()])
            .arg(&self.xtask)
            .args([
                "live-burst",
                "--burst-ms",
                &burst_ms.to_string(),
                "--period-ms",
                "20",
            ])
            .args(["--seconds", &seconds.to_string()])
            .current_dir(&self.root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("start the contention")?;
        Ok(Some(OwnedChild(child)))
    }

    /// The round's config: the administrator's game cgroup, the families the controller may
    /// choose from, and the externally managed worker.
    pub(super) fn write_config(
        &self,
        dir: &Path,
        operations: Operations,
    ) -> anyhow::Result<PathBuf> {
        // An empty list is invalid: name the denied family only when there is one.
        let denied = match operations {
            Operations::Affinity => "daemon_denied_action_families = [\"cgroup_placement\"]\n",
            Operations::Cgroup => "daemon_denied_action_families = [\"cpu_affinity_profile\"]\n",
            Operations::Both => "",
        };
        let config = dir.join("config.toml");
        fs::write(
            &config,
            format!(
                "daemon_game_cgroup = \"{}\"\n{denied}\n[autotune]\nallow_medium_risk_apply = true\nmanage_privileged_worker = false\nprivileged_worker_socket = \"{}\"\n",
                self.game_cgroup()?.display(),
                self.socket.display()
            ),
        )?;
        Ok(config)
    }

    /// The administrator's affinity profile: the game's render and worker threads to the quiet CPU.
    pub(super) fn write_profiles(&self, dir: &Path) -> anyhow::Result<PathBuf> {
        let path = dir.join("profiles.toml");
        fs::write(
            &path,
            format!(
                "[[profile]]\nname = \"game-isolate-render-benefit\"\n\n[[profile.rules]]\naffinity = \"{}\"\nmatch_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\"]\n",
                self.cpus.1
            ),
        )?;
        Ok(path)
    }

    pub(super) fn spawn_controller(
        &self,
        dir: &Path,
        name: &str,
        pid: u32,
        operations: Operations,
        timing: Timing,
    ) -> anyhow::Result<Controller> {
        let config = self.write_config(dir, operations)?;
        let mut command = Command::new(&self.stutter);
        command
            .args([
                "autotune",
                "--mode",
                "apply-medium-risk",
                "--allow-medium-risk",
            ])
            .args(["--tree-pid", &pid.to_string(), "--preset", "lightweight"])
            .args(["--washout-seconds", "3", "--summary-ms", "1000"])
            .args(["--duration-seconds", &timing.candidate_window_s.to_string()])
            .arg("--config")
            .arg(&config);
        if operations != Operations::Cgroup {
            command.arg("--profiles").arg(self.write_profiles(dir)?);
        }
        let child = command
            .current_dir(&self.root)
            .env("HOME", self.dir.join("home"))
            .env("RUST_LOG", "info")
            .stdout(fs::File::create(dir.join(format!("{name}.stdout")))?)
            .stderr(fs::File::create(dir.join(format!("{name}.stderr")))?)
            .spawn()
            .context("start the controller")?;
        Ok(Controller {
            child: OwnedChild(child),
            started_unix_ms: super::records::unix_ms(),
        })
    }
}

/// The process start time in clock ticks, the identity half of a pid.
pub(super) fn start_ticks(pid: u32) -> anyhow::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    stat.rsplit_once(')')
        .context("malformed /proc stat")?
        .1
        .split_whitespace()
        .nth(19)
        .context("no start time")?
        .parse()
        .context("start time is not a number")
}

/// Wait for a child to exit, up to `timeout`; `None` if it is still running.
pub(super) fn wait_exit(
    child: &mut OwnedChild,
    timeout: Duration,
) -> anyhow::Result<Option<std::process::ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.0.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn require_tools() -> anyhow::Result<()> {
    for tool in ["taskset", "chrt", "sudo", "rtcwake"] {
        let found = std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|path| path.join(tool).is_file())
        }) || Path::new("/run/wrappers/bin").join(tool).is_file()
            || Path::new("/run/current-system/sw/bin").join(tool).is_file();
        if !found {
            bail!("the soak needs `{tool}` on PATH");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(pid: u32) -> Option<char> {
        fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()?
            .rsplit_once(')')?
            .1
            .trim()
            .chars()
            .next()
    }

    /// The long round once left its exited workload unreaped: a zombie still has a `/proc` entry,
    /// so the controller kept following a dead target and never exited. Only `try_wait`/`wait`
    /// by the launcher removes it.
    #[test]
    fn an_exited_workload_stays_visible_until_its_launcher_reaps_it() {
        let mut child = OwnedChild(Command::new("true").spawn().unwrap());
        let pid = child.pid();
        let deadline = Instant::now() + Duration::from_secs(10);
        while state(pid) != Some('Z') && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            state(pid),
            Some('Z'),
            "an exited, unreaped child is a zombie"
        );
        assert!(
            wait_exit(&mut child, Duration::from_secs(5))
                .unwrap()
                .is_some()
        );
        assert_eq!(state(pid), None, "reaping removes it");
    }
}
