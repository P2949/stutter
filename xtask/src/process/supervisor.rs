//! Supervised execution of agent-owned child processes.
//!
//! Every stage runs in a fresh process group that this module owns (the group id is the child's
//! pid, set before exec). On a deadline, a cancellation or the end of the stage the whole group
//! is signalled TERM, given a bounded grace, then KILLed and reaped, so a stage never leaves an
//! owned descendant behind. Only that verified group is ever signalled: never a name match, never
//! a process this module did not spawn.
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, Once,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::Context;

/// Environment marker every supervised child inherits, so a leftover is attributable to its owner.
pub const OWNER_ENV: &str = "STUTTER_XTASK_OWNER_PID";

const POLL: Duration = Duration::from_millis(20);

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static INSTALL_HANDLERS: Once = Once::new();

extern "C" fn on_signal(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Turns SIGINT/SIGTERM aimed at this process into a cancellation of the running stage, so an
/// interrupted agent run reaps its group instead of orphaning it.
pub fn install_interrupt_handlers() {
    INSTALL_HANDLERS.call_once(|| {
        // SAFETY: `on_signal` only stores to an atomic, which is async-signal-safe.
        unsafe {
            libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
            libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        }
    });
}

#[derive(Clone, Debug)]
pub struct ExecutionPolicy {
    /// Wall-clock deadline for the stage; `None` only for callers that own a larger deadline.
    pub timeout: Option<Duration>,
    /// Time between TERM and KILL.
    pub grace: Duration,
    /// Most bytes of combined output retained in memory; the rest is drained and counted.
    pub output_cap: usize,
    /// Echo the child's output to this process's stdout/stderr as it arrives.
    pub tee: bool,
    /// Cooperative cancellation requested by the caller.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl ExecutionPolicy {
    /// A stage that may build for a long time: bounded, but generous.
    pub fn long_build() -> Self {
        Self::bounded(Duration::from_secs(3 * 60 * 60))
    }

    /// A short test or probe.
    pub fn short_test() -> Self {
        Self::bounded(Duration::from_secs(15 * 60))
    }

    pub fn bounded(timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
            grace: Duration::from_secs(5),
            output_cap: 4 * 1024 * 1024,
            tee: true,
            cancel: None,
        }
    }
}

#[derive(Debug)]
pub enum Outcome {
    Exited(ExitStatus),
    TimedOut,
    Cancelled,
}

#[derive(Debug)]
pub struct Supervised {
    pub outcome: Outcome,
    pub stdout: String,
    pub stderr: String,
    /// Output bytes that were drained but not retained because of the cap.
    pub dropped_bytes: u64,
    /// The process group that was supervised (equal to the child's pid).
    pub group: i32,
}

struct Captured {
    text: Vec<u8>,
    dropped: u64,
}

fn drain<R: Read + Send + 'static>(
    mut source: R,
    cap: usize,
    tee: Option<bool>,
) -> thread::JoinHandle<Captured> {
    thread::spawn(move || {
        let mut captured = Captured {
            text: Vec::new(),
            dropped: 0,
        };
        let mut chunk = [0u8; 8192];
        while let Ok(read) = source.read(&mut chunk) {
            if read == 0 {
                break;
            }
            if let Some(to_stderr) = tee {
                let _ = if to_stderr {
                    std::io::stderr().write_all(&chunk[..read])
                } else {
                    std::io::stdout().write_all(&chunk[..read])
                };
            }
            let room = cap.saturating_sub(captured.text.len());
            let keep = room.min(read);
            captured.text.extend_from_slice(&chunk[..keep]);
            captured.dropped += (read - keep) as u64;
        }
        captured
    })
}

fn signal_group(group: i32, signal: libc::c_int) {
    // SAFETY: `group` is the process group this module created for its own child; a negative pid
    // addresses exactly that group.
    unsafe { libc::kill(-group, signal) };
}

fn group_alive(group: i32) -> bool {
    // SAFETY: signal 0 only probes existence of the owned group.
    unsafe { libc::kill(-group, 0) == 0 }
}

/// TERM, wait up to `grace` for the whole group to be gone, then KILL.
fn stop_group(group: i32, grace: Duration) {
    signal_group(group, libc::SIGTERM);
    let deadline = Instant::now() + grace;
    while group_alive(group) && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    signal_group(group, libc::SIGKILL);
}

/// Runs `command` as an owned process group under `policy` and reaps it.
pub fn run(mut command: Command, policy: &ExecutionPolicy) -> anyhow::Result<Supervised> {
    install_interrupt_handlers();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(OWNER_ENV, std::process::id().to_string())
        .process_group(0);
    // SAFETY: only async-signal-safe prctl before exec; the direct child dies if this process does.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .context("failed to start supervised command")?;
    let group = child.id() as i32;
    let out = drain(
        child.stdout.take().expect("piped stdout"),
        policy.output_cap / 2,
        policy.tee.then_some(false),
    );
    let err = drain(
        child.stderr.take().expect("piped stderr"),
        policy.output_cap / 2,
        policy.tee.then_some(true),
    );
    let started = Instant::now();
    let outcome = loop {
        if let Some(status) = child.try_wait().context("failed to poll child")? {
            break Outcome::Exited(status);
        }
        if INTERRUPTED.load(Ordering::SeqCst)
            || policy
                .cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            break Outcome::Cancelled;
        }
        if policy
            .timeout
            .is_some_and(|limit| started.elapsed() >= limit)
        {
            break Outcome::TimedOut;
        }
        thread::sleep(POLL);
    };
    // Whatever the outcome, no member of the owned group may outlive the stage.
    stop_group(group, policy.grace);
    let _ = child.wait();
    // Killed members become zombies until init reaps them; give that a bounded moment.
    let gone = Instant::now() + Duration::from_secs(2);
    while group_alive(group) && Instant::now() < gone {
        thread::sleep(POLL);
    }
    let out = out.join().unwrap_or(Captured {
        text: Vec::new(),
        dropped: 0,
    });
    let err = err.join().unwrap_or(Captured {
        text: Vec::new(),
        dropped: 0,
    });
    Ok(Supervised {
        outcome,
        stdout: String::from_utf8_lossy(&out.text).into_owned(),
        stderr: String::from_utf8_lossy(&err.text).into_owned(),
        dropped_bytes: out.dropped + err.dropped,
        group,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    fn quiet(timeout_ms: u64) -> ExecutionPolicy {
        ExecutionPolicy {
            timeout: Some(Duration::from_millis(timeout_ms)),
            grace: Duration::from_millis(300),
            output_cap: 64 * 1024,
            tee: false,
            cancel: None,
        }
    }

    fn exit_code(run: &Supervised) -> Option<i32> {
        match &run.outcome {
            Outcome::Exited(status) => status.code(),
            _ => None,
        }
    }

    #[test]
    fn exit_codes_are_reported_exactly() {
        assert_eq!(
            exit_code(&run(sh("exit 0"), &quiet(5000)).unwrap()),
            Some(0)
        );
        assert_eq!(
            exit_code(&run(sh("exit 7"), &quiet(5000)).unwrap()),
            Some(7)
        );
    }

    #[test]
    fn an_infinite_wait_times_out_and_is_reaped() {
        let started = Instant::now();
        let result = run(sh("sleep 600"), &quiet(200)).unwrap();
        assert!(matches!(result.outcome, Outcome::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!group_alive(result.group));
    }

    #[test]
    fn a_pipe_flood_is_bounded_and_the_rest_drained() {
        let result = run(sh("yes x | head -c 5000000"), &quiet(20_000)).unwrap();
        assert_eq!(exit_code(&result), Some(0));
        assert!(result.stdout.len() <= 32 * 1024);
        assert!(result.dropped_bytes > 4_000_000);
    }

    #[test]
    fn a_forked_background_child_does_not_outlive_the_stage() {
        let result = run(sh("sleep 600 & echo started; exit 0"), &quiet(5000)).unwrap();
        assert_eq!(exit_code(&result), Some(0));
        assert!(!group_alive(result.group), "owned descendant survived");
    }

    #[test]
    fn a_term_ignoring_descendant_is_killed_after_the_grace() {
        let script = "trap '' TERM; (trap '' TERM; sleep 600) & sleep 600";
        let started = Instant::now();
        let result = run(sh(script), &quiet(200)).unwrap();
        assert!(matches!(result.outcome, Outcome::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(!group_alive(result.group), "TERM-ignoring member survived");
    }

    #[test]
    fn a_cancelled_run_is_reaped() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut policy = quiet(60_000);
        policy.cancel = Some(Arc::clone(&cancel));
        let flag = Arc::clone(&cancel);
        let trigger = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            flag.store(true, Ordering::SeqCst);
        });
        let result = run(sh("sleep 600"), &policy).unwrap();
        trigger.join().unwrap();
        assert!(matches!(result.outcome, Outcome::Cancelled));
        assert!(!group_alive(result.group));
    }

    #[test]
    fn an_unrelated_sentinel_is_never_touched() {
        let mut sentinel = Command::new("sleep").arg("600").spawn().unwrap();
        let result = run(sh("sleep 600"), &quiet(150)).unwrap();
        assert!(matches!(result.outcome, Outcome::TimedOut));
        assert!(
            sentinel.try_wait().unwrap().is_none(),
            "sentinel was killed"
        );
        sentinel.kill().unwrap();
        sentinel.wait().unwrap();
    }

    #[test]
    fn children_carry_the_owner_marker() {
        let result = run(sh(&format!("echo ${OWNER_ENV}")), &quiet(5000)).unwrap();
        assert_eq!(result.stdout.trim(), std::process::id().to_string());
    }
}
