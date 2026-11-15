//! Bounded cleanup of a research-owned game process. A signal is only ever sent to a process whose
//! identity (PID and start time) was just re-read and matches the attributed one, through a pidfd
//! so a recycled PID can never be hit. The Steam client, an ambiguous game sighting, and anything
//! that cannot be attributed are never signalled: unknown ownership means no signal, and the
//! attempt stays unsettled.

use std::{
    fs::File,
    os::fd::AsFd,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use stutter_core::ids::Pid;

use super::{
    dispatch_context::DispatchContextV1,
    native_source::{GameFacts, Sighting},
    steam_port::OwnedTerminator,
};

const SIGTERM: i32 = libc::SIGTERM;
const SIGKILL: i32 = libc::SIGKILL;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedProcess {
    pub pid: Pid,
    pub start_ticks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Termination {
    /// The process was already gone (or a zombie) before any signal was sent.
    Gone,
    /// The process ended after a signal.
    Terminated,
}

/// The process's start time and whether it is a zombie, or `None` when it no longer exists.
fn read_process(proc_root: &Path, pid: u32) -> Option<(u64, bool)> {
    let stat = std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    // The command name is parenthesized and may itself contain spaces or parentheses.
    let after_name = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_name.split_whitespace();
    let state = fields.next()?;
    // starttime is field 22 overall: 20 more fields follow the state (field 3).
    let start_ticks = fields.nth(18)?.parse().ok()?;
    Some((start_ticks, state == "Z"))
}

fn gone(proc_root: &Path, target: &OwnedProcess) -> bool {
    !matches!(
        read_process(proc_root, target.pid.as_u32()),
        Some((ticks, false)) if ticks == target.start_ticks
    )
}

fn wait_gone(proc_root: &Path, target: &OwnedProcess, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if gone(proc_root, target) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    gone(proc_root, target)
}

/// Terminate exactly `target`: SIGTERM, a bounded grace period, then SIGKILL. A mismatched start
/// time is an error and nothing is signalled.
pub fn terminate_exact(
    proc_root: &Path,
    target: &OwnedProcess,
    grace: Duration,
) -> anyhow::Result<Termination> {
    let pidfd = match crate::syscall::pidfd_open(target.pid.as_u32()) {
        Ok(fd) => fd,
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(Termination::Gone),
        Err(error) => return Err(error.into()),
    };
    match read_process(proc_root, target.pid.as_u32()) {
        None => return Ok(Termination::Gone),
        Some((ticks, _)) if ticks != target.start_ticks => {
            anyhow::bail!("the process identity changed; nothing was signalled")
        }
        Some((_, true)) => return Ok(Termination::Gone),
        Some(_) => {}
    }
    for (signal, wait) in [(SIGTERM, grace), (SIGKILL, Duration::from_secs(2))] {
        match crate::syscall::pidfd_send_signal(pidfd.as_fd(), signal) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                return Ok(Termination::Terminated);
            }
            Err(error) => return Err(error.into()),
        }
        if wait_gone(proc_root, target, wait) {
            return Ok(Termination::Terminated);
        }
    }
    anyhow::bail!("the owned process did not exit after SIGKILL")
}

/// Terminates the attributed game process, and only that one. The attribution comes from the
/// independent observer (name, owner, Steam app and installed executable all agree); an
/// ambiguous or unreadable observation signals nothing.
pub struct ProcfsTerminator<F> {
    facts: F,
    proc_root: PathBuf,
    grace: Duration,
}

impl<F> ProcfsTerminator<F> {
    pub fn new(facts: F, proc_root: PathBuf, grace: Duration) -> Self {
        Self {
            facts,
            proc_root,
            grace,
        }
    }
}

impl<F: GameFacts + Send> OwnedTerminator for ProcfsTerminator<F> {
    fn terminate_owned(&mut self, _: &File, _: &DispatchContextV1) -> anyhow::Result<()> {
        match self.facts.sight()? {
            // Nothing attributable is running: there is nothing to terminate.
            Sighting::None => Ok(()),
            Sighting::Multiple => {
                anyhow::bail!("the game sighting is ambiguous; no process was signalled")
            }
            Sighting::One(identity) => {
                let target = OwnedProcess {
                    pid: identity.pid,
                    start_ticks: identity.start_ticks,
                };
                terminate_exact(&self.proc_root, &target, self.grace).map(|_| ())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command};

    use super::*;
    use crate::autotune::research::cyberpunk::CyberpunkProcessIdentity;

    fn sleeper() -> (Child, OwnedProcess) {
        let child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = Pid::new(child.id());
        let (start_ticks, _) = read_process(Path::new("/proc"), pid.as_u32()).unwrap();
        (child, OwnedProcess { pid, start_ticks })
    }

    fn alive(target: &OwnedProcess) -> bool {
        !gone(Path::new("/proc"), target)
    }

    #[test]
    fn the_exact_owned_process_is_terminated_and_nothing_else() {
        let (mut child, target) = sleeper();
        let (mut bystander, other) = sleeper();
        let outcome = terminate_exact(Path::new("/proc"), &target, Duration::from_secs(2)).unwrap();
        assert_eq!(outcome, Termination::Terminated);
        child.wait().unwrap();
        // A second process of the same program was never touched.
        assert!(alive(&other));
        bystander.kill().unwrap();
        bystander.wait().unwrap();
    }

    #[test]
    fn a_process_that_is_already_gone_needs_no_signal() {
        let (mut child, target) = sleeper();
        child.kill().unwrap();
        child.wait().unwrap();
        let outcome = terminate_exact(Path::new("/proc"), &target, Duration::from_secs(1)).unwrap();
        assert_eq!(outcome, Termination::Gone);
    }

    #[test]
    fn a_changed_identity_signals_nothing() {
        let (mut child, mut target) = sleeper();
        // A reused PID would carry a different start time.
        target.start_ticks += 1;
        assert!(terminate_exact(Path::new("/proc"), &target, Duration::from_secs(1)).is_err());
        target.start_ticks -= 1;
        assert!(alive(&target));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    struct Facts(Option<Sighting>);
    impl GameFacts for Facts {
        fn sight(&mut self) -> anyhow::Result<Sighting> {
            self.0
                .clone()
                .ok_or_else(|| anyhow::anyhow!("procfs unreadable"))
        }
        fn settings_sha256(&mut self) -> anyhow::Result<String> {
            Ok("c".repeat(64))
        }
        fn now_unix_nanos(&mut self) -> anyhow::Result<u128> {
            Ok(1)
        }
    }

    fn identity(target: &OwnedProcess) -> CyberpunkProcessIdentity {
        CyberpunkProcessIdentity {
            pid: target.pid,
            start_ticks: target.start_ticks,
            uid: crate::syscall::geteuid(),
            app_id: 1_091_500,
            build_id: 7,
            executable_sha256: "a".repeat(64),
        }
    }

    fn test_context() -> DispatchContextV1 {
        use crate::autotune::research::{AttemptSlot, dispatch_context::ArmSpec, tests::intent};
        let intent = intent();
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: intent.tools[0].source,
        };
        DispatchContextV1 {
            schema_version: 1,
            intent,
            slot,
            tickets: Default::default(),
            spec: ArmSpec {
                tool_dir: "/t".into(),
                study_compat_data: "/s".into(),
                arm_env: Default::default(),
                steamapps: "/a".into(),
                proton_manifest: "/m".into(),
            },
        }
    }

    fn run(facts: Facts) -> anyhow::Result<()> {
        let dir = tempfile::tempdir().unwrap();
        let file = File::open(dir.path()).unwrap();
        let context = test_context();
        ProcfsTerminator::new(facts, PathBuf::from("/proc"), Duration::from_secs(2))
            .terminate_owned(&file, &context)
    }

    #[test]
    fn only_a_single_attributed_game_is_terminated() {
        let (mut child, target) = sleeper();
        // Ambiguous or unreadable observations signal nothing.
        assert!(run(Facts(Some(Sighting::Multiple))).is_err());
        assert!(run(Facts(None)).is_err());
        assert!(alive(&target));
        // No game means nothing to do.
        assert!(run(Facts(Some(Sighting::None))).is_ok());
        assert!(alive(&target));
        // One attributed game is terminated.
        run(Facts(Some(Sighting::One(identity(&target))))).unwrap();
        child.wait().unwrap();
        assert!(!alive(&target));
    }
}
