//! The validation session around the installed worker: its deny-all / exact-fixture fence and the
//! worker's own read-only acknowledgment of it.
//!
//! Writing a fence file is not proof that the running worker enforces it: an older worker ignores
//! the file. So before any mutation case, and again once a fixture is fenced, the smoke asks the
//! worker itself (`fence_status`, read-only) what it has read, and refuses unless that is exactly
//! what was written. An older worker answers with an error, which refuses the run before it mutates.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

use anyhow::{Context, bail};
use stutter::api::daemon::{FenceReport, UnixSocketPrivilegedActionService};

use super::{isolation, wait_for};

/// How long a validation session fences the worker; a forgotten one expires and refuses mutation
/// until removed.
pub(super) const SESSION_SECONDS: u64 = 1800;

/// Ends the validation session (lifts the fence) when the run is over, however it ends, after every
/// token has settled.
pub(super) struct Session;

impl Drop for Session {
    fn drop(&mut self) {
        isolation::remove_fence();
    }
}

/// What the worker must report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Expected {
    DenyAll,
    Fixture { pid: u32, start_ticks: u64 },
}

/// The worker's socket and the account it serves, once known.
static WORKER: Mutex<Option<(PathBuf, u32)>> = Mutex::new(None);

/// The `--socket` argument of the worker unit's `ExecStart` property.
pub(super) fn socket_of(exec_start: &str) -> Option<PathBuf> {
    let rest = exec_start.split("argv[]=").nth(1)?;
    let argv = rest.split(" ;").next()?;
    let mut words = argv.split_whitespace();
    words.find(|word| *word == "--socket")?;
    words
        .next()
        .filter(|path| path.starts_with('/'))
        .map(PathBuf::from)
}

pub(super) fn set_worker(socket: PathBuf, uid: u32) -> anyhow::Result<()> {
    *WORKER
        .lock()
        .map_err(|_| anyhow::anyhow!("the worker state is poisoned"))? = Some((socket, uid));
    Ok(())
}

/// Every reason the worker's report is not what the smoke wrote; empty when it is exactly that.
pub(super) fn mismatches(
    report: &FenceReport,
    expected: &Expected,
    boot_id: &str,
    uid: u32,
    written_sha256: &str,
) -> Vec<String> {
    let mut problems = Vec::new();
    if report.protocol != 1 {
        problems.push(format!("protocol {} is not 1", report.protocol));
    }
    let (state, pid, ticks) = match expected {
        Expected::DenyAll => ("deny_all", None, None),
        Expected::Fixture { pid, start_ticks } => ("active", Some(*pid), Some(*start_ticks)),
    };
    if report.state != state {
        problems.push(format!(
            "state {:?} (reason {:?}), expected {state:?}",
            report.state, report.reason
        ));
    }
    if report.root_pid != pid || report.start_ticks != ticks {
        problems.push(format!(
            "target {:?}/{:?}, expected {pid:?}/{ticks:?}",
            report.root_pid, report.start_ticks
        ));
    }
    if report.uid != uid {
        problems.push(format!("account {}, expected {uid}", report.uid));
    }
    if report.boot_id != boot_id.trim() {
        problems.push(format!(
            "boot {:?}, expected {:?}",
            report.boot_id,
            boot_id.trim()
        ));
    }
    if report.fence_sha256.as_deref() != Some(written_sha256) {
        problems.push(
            "the digest of the fence it read is not the digest of the fence written".to_owned(),
        );
    }
    problems
}

/// Asks the running worker what fence it enforces and refuses unless it is exactly `expected`.
pub(super) fn acknowledge(expected: &Expected, written_sha256: &str) -> anyhow::Result<()> {
    let (socket, uid) = WORKER
        .lock()
        .ok()
        .and_then(|worker| worker.clone())
        .context("the worker's socket is not known: refusing to proceed unacknowledged")?;
    let report = UnixSocketPrivilegedActionService::new(&socket)
        .fence_status()
        .with_context(|| {
            format!(
                "the worker at {} did not acknowledge the validation fence (an older build does not know the request)",
                socket.display()
            )
        })?;
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let problems = mismatches(&report, expected, &boot, uid, written_sha256);
    if !problems.is_empty() {
        bail!(
            "the worker's fence is not the one written: {}",
            problems.join("; ")
        );
    }
    Ok(())
}

pub(super) struct Fixture {
    pub(super) child: std::process::Child,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Back to deny-all, not to no fence: the worker must refuse every target until the session ends.
        let _ = isolation::deny_all(SESSION_SECONDS);
    }
}

/// A distinct `name` is a distinct workload identity: the controller does not retry a recently tried
/// candidate for the same identity, so each smoke case uses its own.
pub(super) fn spawn_fixture(
    out: &Path,
    name: &str,
    cpu: u32,
    seconds: u64,
) -> anyhow::Result<Fixture> {
    // The installed controller picks its target heuristically: nothing that could be chosen instead
    // of this fixture may exist when it starts, nor once it is ready.
    isolation::ensure_quiet("before starting a fixture")?;
    let directory = out.join("steamapps/common").join(name);
    fs::create_dir_all(&directory)?;
    let game = directory.join(name);
    let this = std::env::current_exe()?;
    fs::copy(&this, &game).context("install the workload fixture")?;
    fs::copy(&this, directory.join("wineserver")).context("install the workload helper")?;
    let ready = out.join("fixture.ready");
    let child = Command::new("taskset")
        .args(["-c", &cpu.to_string()])
        .arg(&game)
        .args([
            "live-workload",
            "--interactive-root",
            "--wine-helper",
            "--seconds",
        ])
        .arg(seconds.to_string())
        .arg("--ready")
        .arg(&ready)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("start the workload fixture")?;
    let fixture = Fixture { child };
    // The fixture is known by exact identity (pid and start time), or the run cannot proceed.
    isolation::record_fixture(Path::new("/proc"), fixture.child.id())?;
    wait_for("the fixture to be ready", 20, || ready.exists())?;
    isolation::ensure_quiet("after the fixture was ready")?;
    // Fence the worker to this fixture, by exact identity, for as long as it runs.
    let written = isolation::write_fence(fixture.child.id(), seconds)?;
    let live = stutter::api::telemetry::live_process(Path::new("/proc"), fixture.child.id())
        .context("the fixture vanished before the worker acknowledged its fence")?;
    acknowledge(
        &Expected::Fixture {
            pid: fixture.child.id(),
            start_ticks: live.start_ticks,
        },
        &written,
    )?;
    Ok(fixture)
}
