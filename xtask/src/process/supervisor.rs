//! Supervised execution of agent-owned child processes.
//!
//! Every stage runs in a fresh process group that this module owns (the group id is the child's
//! pid, set before exec). On a deadline, a cancellation or the end of the stage the whole group
//! is signalled TERM, given a bounded grace, then KILLed and reaped, so a stage never leaves an
//! owned descendant behind. Only that verified group is ever signalled: never a name match, never
//! a process this module did not spawn.
use std::{
    io::{Read, Write},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::Path,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, Once,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::Context;

use super::ownership::{self, OWNER_ENV};

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
    complete: bool,
}

fn drain<R: Read + AsRawFd + Send + 'static>(
    mut source: R,
    cap: usize,
    tee: Option<bool>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<thread::JoinHandle<Captured>> {
    // A detached descendant can keep a pipe open after the process group is gone. Never join a
    // reader blocked on that pipe: nonblocking reads let cleanup end it within one poll interval.
    let fd = source.as_raw_fd();
    // SAFETY: fd is a live pipe owned by source; these calls only change its file status flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(thread::spawn(move || {
        let mut captured = Captured {
            text: Vec::new(),
            dropped: 0,
            complete: false,
        };
        let mut chunk = [0u8; 8192];
        loop {
            let read = match source.read(&mut chunk) {
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    thread::sleep(POLL);
                    continue;
                }
                Err(_) => break,
            };
            if read == 0 {
                captured.complete = true;
                break;
            }
            let room = cap.saturating_sub(captured.text.len());
            let keep = room.min(read);
            if let Some(to_stderr) = tee {
                // Tee has the same immutable byte cap as capture. Flooding cannot fill an
                // unbounded parent log after the in-memory limit has been reached.
                let _ = if to_stderr {
                    std::io::stderr().write_all(&chunk[..keep])
                } else {
                    std::io::stdout().write_all(&chunk[..keep])
                };
            }
            captured.text.extend_from_slice(&chunk[..keep]);
            captured.dropped += (read - keep) as u64;
            if stop.load(Ordering::SeqCst) {
                // Bound draining even if an escaped writer continuously floods the pipe.
                break;
            }
        }
        captured
    }))
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

static STAGE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// This stage's owner identity: the process and a unique stage number.
fn stage_owner() -> Option<ownership::Owner> {
    let stage = STAGE_COUNTER.fetch_add(1, Ordering::SeqCst);
    ownership::current_owner(Path::new("/proc")).map(|owner| owner.with_stage(stage))
}

static ORPHANS_RECONCILED: Once = Once::new();

/// Runs `command` as an owned process group under `policy` and reaps it.
pub fn run(mut command: Command, policy: &ExecutionPolicy) -> anyhow::Result<Supervised> {
    install_interrupt_handlers();
    // Once per process: leftovers of a crashed earlier xtask are killed before new work starts.
    ORPHANS_RECONCILED.call_once(|| {
        let uid = unsafe_uid();
        let killed = ownership::reconcile_orphans(Path::new("/proc"), uid, Duration::from_secs(2));
        if killed > 0 {
            eprintln!("supervisor: reaped {killed} orphaned process(es) of a crashed xtask");
        }
    });
    let stage = stage_owner();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            OWNER_ENV,
            stage.map_or_else(|| std::process::id().to_string(), ownership::Owner::render),
        )
        .process_group(0);
    // SAFETY: only async-signal-safe prctl before exec; the direct child dies if this process does.
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died between fork and prctl. Never exec after that race.
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .context("failed to start supervised command")?;
    let group = child.id() as i32;
    let stop_readers = Arc::new(AtomicBool::new(false));
    let out = drain(
        child.stdout.take().expect("piped stdout"),
        policy.output_cap / 2,
        policy.tee.then_some(false),
        Arc::clone(&stop_readers),
    );
    let err = drain(
        child.stderr.take().expect("piped stderr"),
        policy.output_cap / 2,
        policy.tee.then_some(true),
        Arc::clone(&stop_readers),
    );
    let (out, err) = match (out, err) {
        (Ok(out), Ok(err)) => (out, err),
        (out, err) => {
            stop_group(group, policy.grace);
            let _ = child.wait();
            stop_readers.store(true, Ordering::SeqCst);
            if let Ok(reader) = out {
                let _ = reader.join();
            }
            if let Ok(reader) = err {
                let _ = reader.join();
            }
            anyhow::bail!("could not configure bounded output readers");
        }
    };
    let started = Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(Outcome::Exited(status)),
            Ok(None) => {}
            Err(error) => break Err(error),
        }
        if INTERRUPTED.load(Ordering::SeqCst)
            || policy
                .cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::SeqCst))
        {
            break Ok(Outcome::Cancelled);
        }
        if policy
            .timeout
            .is_some_and(|limit| started.elapsed() >= limit)
        {
            break Ok(Outcome::TimedOut);
        }
        thread::sleep(POLL);
    };
    // Whatever the outcome, no member of the owned group may outlive the stage.
    stop_group(group, policy.grace);
    // A child that detached with setsid left the group but kept the owner marker.
    if let Some(owner) = stage {
        ownership::sweep_owned(Path::new("/proc"), unsafe_uid(), owner, policy.grace);
    }
    let _ = child.wait();
    // Killed members become zombies until init reaps them; give that a bounded moment.
    let gone = Instant::now() + Duration::from_secs(2);
    while group_alive(group) && Instant::now() < gone {
        thread::sleep(POLL);
    }
    // Allow the finite pipe backlog to drain after cleanup, then stop even if a writer escaped.
    let drain_deadline = Instant::now() + Duration::from_secs(1);
    while !(out.is_finished() && err.is_finished()) && Instant::now() < drain_deadline {
        thread::sleep(POLL);
    }
    stop_readers.store(true, Ordering::SeqCst);
    let out = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader panicked"))?;
    let err = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader panicked"))?;
    anyhow::ensure!(
        out.complete && err.complete,
        "output did not reach EOF after cleanup; an uncontained descendant or read failure invalidated the stage"
    );
    let outcome = outcome.context("failed to poll child (owned group cleaned up)")?;
    Ok(Supervised {
        outcome,
        stdout: String::from_utf8_lossy(&out.text).into_owned(),
        stderr: String::from_utf8_lossy(&err.text).into_owned(),
        dropped_bytes: out.dropped + err.dropped,
        group,
    })
}

fn unsafe_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;

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
    fn a_markerless_pipe_holder_invalidates_the_stage_without_hanging_the_reader() {
        let file = std::env::temp_dir().join(format!("stutter-pipe-holder-{}", std::process::id()));
        let script = format!(
            "setsid env -u {OWNER_ENV} sh -c 'echo $$ > {}; exec sleep 30' & sleep 0.3",
            file.display()
        );
        let started = Instant::now();
        let result = run(sh(&script), &quiet(2000));
        let elapsed = started.elapsed();
        // This fixture deliberately escapes the marker-based cleanup. Clean it up by the pid
        // recorded by our own helper before any assertion, including when the test fails.
        if let Ok(pid) = std::fs::read_to_string(&file) {
            let pid: i32 = pid.trim().parse().unwrap();
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let _ = std::fs::remove_file(file);
        assert!(elapsed < Duration::from_secs(10));
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("uncontained descendant")
        );
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
        let owner = ownership::current_owner(Path::new("/proc")).unwrap();
        let marker = ownership::Owner::parse(result.stdout.trim()).unwrap();
        assert_eq!(
            (marker.pid, marker.start_ticks),
            (owner.pid, owner.start_ticks)
        );
        assert!(marker.stage > 0);
    }

    fn alive(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
            && std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| !stat.contains(") Z "))
    }

    #[test]
    fn a_setsid_escapee_is_found_by_its_marker_and_killed() {
        let file = std::env::temp_dir().join(format!("stutter-escapee-{}", std::process::id()));
        let script = format!("setsid sleep 600 & echo $! > {}; sleep 0.3", file.display());
        let result = run(sh(&script), &quiet(5000)).unwrap();
        assert_eq!(exit_code(&result), Some(0));
        let pid: u32 = std::fs::read_to_string(&file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(!alive(pid), "the detached child survived the stage");
        std::fs::remove_file(&file).unwrap();
    }

    #[test]
    fn orphans_of_a_dead_owner_are_reaped_but_a_live_owner_is_left_alone() {
        let uid = unsafe_uid();
        let mut orphan = Command::new("sleep")
            .arg("600")
            .env(OWNER_ENV, "4000000:1")
            .spawn()
            .unwrap();
        let live = ownership::current_owner(Path::new("/proc")).unwrap();
        let mut kept = Command::new("sleep")
            .arg("600")
            .env(OWNER_ENV, live.render())
            .spawn()
            .unwrap();
        let mut unrelated = Command::new("sleep").arg("600").spawn().unwrap();
        // Another test thread may already have run the once-per-process reconcile; either way the
        // orphan must end up killed by a signal.
        ownership::reconcile_orphans(Path::new("/proc"), uid, Duration::from_millis(300));
        let orphan_status = orphan.wait().unwrap();
        let kept_alive = kept.try_wait().unwrap().is_none();
        let unrelated_alive = unrelated.try_wait().unwrap().is_none();
        for child in [&mut kept, &mut unrelated] {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(
            orphan_status.signal().is_some(),
            "the orphan was not reaped"
        );
        assert!(kept_alive, "a live owner's child was killed");
        assert!(unrelated_alive, "an unmarked process was killed");
    }
}
