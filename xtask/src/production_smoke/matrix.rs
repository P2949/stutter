//! The production behaviour cases of the smoke: an idle controller, a workload that exits while a
//! mutation is applied, and a controller that dies while a mutation is applied. Each is bounded by
//! its own timeout and none adds a dwell floor.
//!
//! The controller will not apply the same action again for 300 seconds after applying it, whichever
//! workload it was applied to. Every applied case therefore starts that cooldown. The remaining
//! cooldown is computed from the history, waited out once per gap, and the gap is used for checks that
//! need no new mutation. Candidate memory is never cleared and the policy is never overridden.
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde_json::Value;

use super::{
    Check, Outcome, allowed_lists, fold_outcome, is_active, is_inactive, ledger, new_records,
    run_text, spawn_fixture, wait_for,
};

/// An idle always-on controller may use at most this share of one core, averaged over the window.
pub(super) const IDLE_CPU_PERCENT_LIMIT: f64 = 5.0;
/// ...and this much resident memory.
pub(super) const IDLE_RSS_KB_LIMIT: u64 = 256 * 1024;
/// The current history file stays within its rotation cap plus one rotation's slack.
pub(super) const HISTORY_BYTES_LIMIT: u64 = 160 * 1024 * 1024;
/// The controller does not apply the same action again for this long after applying it.
pub(super) const ACTION_COOLDOWN_SECONDS: u64 = 300;
/// How much of the history's tail is read to find the last application.
const HISTORY_TAIL_BYTES: u64 = 8 * 1024 * 1024;

// The idle limits are small but not zero, and the history limit sits above the rotation cap.
const _: () = assert!(IDLE_CPU_PERCENT_LIMIT > 0.0 && IDLE_CPU_PERCENT_LIMIT <= 10.0);
const _: () = assert!(IDLE_RSS_KB_LIMIT >= 64 * 1024);
const _: () = assert!(HISTORY_BYTES_LIMIT > 128 * 1024 * 1024);

pub(super) fn cpu_percent(ticks: u64, clock_ticks_per_second: u64, seconds: f64) -> f64 {
    ticks as f64 / clock_ticks_per_second as f64 / seconds * 100.0
}

/// Decisions that change state. Everything else (`observed`, `noop`, `cooldown_entered`, ...) is
/// policy activity: the controller deciding, not acting.
pub(super) fn is_state_changing(decision: &str) -> bool {
    matches!(
        decision,
        "candidate_applied"
            | "candidate_started"
            | "candidate_kept"
            | "candidate_reverted"
            | "restored"
    )
}

/// How long until the same action may be applied again, given the time of the last application.
pub(super) fn cooldown_remaining(
    last_applied_unix_nanos: Option<u128>,
    now_unix_nanos: u128,
) -> Duration {
    let Some(applied) = last_applied_unix_nanos else {
        return Duration::ZERO;
    };
    let until = applied.saturating_add(u128::from(ACTION_COOLDOWN_SECONDS) * 1_000_000_000);
    Duration::from_nanos(
        until
            .saturating_sub(now_unix_nanos)
            .min(u128::from(u64::MAX)) as u64,
    )
}

fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

/// The time of the last `candidate_applied` record in the history's tail, if any.
fn last_application(history: &Path) -> Option<u128> {
    let mut file = fs::File::open(history).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(HISTORY_TAIL_BYTES)))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["decision"]["decision"] == "candidate_applied")
        .filter_map(|record| {
            record["unix_nanos"]
                .as_u64()
                .map(u128::from)
                .or_else(|| record["unix_nanos"].as_str()?.parse().ok())
        })
        .max()
}

/// Waits only as long as the product's own cooldown still requires, once, and says so.
pub(super) fn wait_out_cooldown(history: &Path) -> Duration {
    let remaining = cooldown_remaining(last_application(history), now_unix_nanos());
    if !remaining.is_zero() {
        std::thread::sleep(remaining + Duration::from_secs(3));
    }
    remaining
}

fn process_ticks(pid: u32) -> anyhow::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // The command name may contain spaces; the numeric fields start after its closing parenthesis.
    let rest = stat.rsplit_once(')').context("malformed stat")?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    Ok(fields[11].parse::<u64>()? + fields[12].parse::<u64>()?)
}

fn rss_kb(pid: u32) -> u64 {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmRSS:"))
                .and_then(|value| value.split_whitespace().next()?.parse().ok())
        })
        .unwrap_or(0)
}

fn main_pid(unit: &str) -> anyhow::Result<u32> {
    Ok(
        run_text("systemctl", &["show", "-p", "MainPID", "--value", unit])?
            .trim()
            .parse()?,
    )
}

/// Temporaries of the candidate memory whose process no longer exists.
pub(super) fn stale_temporaries(state_dir: &Path) -> usize {
    fs::read_dir(state_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()?
                .strip_prefix("candidate-memory.tmp-")?
                .parse::<u32>()
                .ok()
        })
        .filter(|pid| !Path::new(&format!("/proc/{pid}")).exists())
        .count()
}

/// Idle behaviour: bounded CPU and memory, no state change, and a clean state directory.
pub(super) fn idle(controller: &str, history: &Path, window_seconds: u64) -> Vec<Check> {
    let mut checks = Vec::new();
    let result = (|| -> anyhow::Result<()> {
        let pid = main_pid(controller)?;
        let ticks_per_second: u64 = run_text("getconf", &["CLK_TCK"])?.trim().parse()?;
        let mut offset = fs::metadata(history).map_or(0, |metadata| metadata.len());
        let before = process_ticks(pid)?;
        let started = Instant::now();
        std::thread::sleep(Duration::from_secs(window_seconds));
        let seconds = started.elapsed().as_secs_f64();
        let used = cpu_percent(process_ticks(pid)? - before, ticks_per_second, seconds);
        let records = new_records(history, &mut offset);
        let changes = records
            .iter()
            .filter(|record| {
                is_state_changing(record["decision"]["decision"].as_str().unwrap_or(""))
            })
            .count();
        let rss = rss_kb(pid);
        checks.push(Check::new(
            "idle_controller_cpu_is_bounded",
            used <= IDLE_CPU_PERCENT_LIMIT,
            format!("{used:.2}% of one core over {seconds:.0}s (limit {IDLE_CPU_PERCENT_LIMIT}%)"),
        ));
        checks.push(Check::new(
            "idle_controller_memory_is_bounded",
            rss <= IDLE_RSS_KB_LIMIT,
            format!("{rss} kB resident (limit {IDLE_RSS_KB_LIMIT} kB)"),
        ));
        checks.push(Check::new(
            "an_idle_controller_changes_nothing",
            changes == 0,
            format!(
                "{changes} state-changing decisions among {} records (policy activity such as cooldown is not counted)",
                records.len()
            ),
        ));
        Ok(())
    })();
    if let Err(error) = result {
        checks.push(Check::new(
            "idle_window_measured",
            false,
            format!("{error:#}"),
        ));
    }
    let state_dir = history.parent().map(Path::to_path_buf).unwrap_or_default();
    let stale = stale_temporaries(&state_dir);
    checks.push(Check::new(
        "no_stale_temporary_files_in_the_state_directory",
        stale == 0,
        format!(
            "{stale} candidate-memory temporaries from dead processes in {}",
            state_dir.display()
        ),
    ));
    let bytes = fs::metadata(history).map_or(0, |metadata| metadata.len());
    checks.push(Check::new(
        "history_stays_bounded",
        bytes <= HISTORY_BYTES_LIMIT,
        format!(
            "{bytes} bytes in {} (limit {HISTORY_BYTES_LIMIT})",
            history.display()
        ),
    ));
    checks
}

/// A fixture the controller has applied a mutation to, with the placement before and during.
struct Applied {
    fixture: super::Fixture,
    pid: u32,
    original: Vec<String>,
    /// The workload's placement was read back as different from the original while applied.
    physically_changed: bool,
    offset: u64,
}

/// Starts a fixture, waits until the controller applied a mutation, and reads the placement back.
/// The cooldown from an earlier application has already been waited out by the caller.
fn spawn_until_applied(
    out: &Path,
    name: &str,
    history: &Path,
    seconds: u64,
) -> anyhow::Result<Applied> {
    let mut offset = fs::metadata(history).map_or(0, |metadata| metadata.len());
    let fixture = spawn_fixture(out, name, 2, seconds + 60)?;
    let pid = fixture.child.id();
    let original = allowed_lists(pid);
    let mut outcome = Outcome::default();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline && !outcome.applied {
        fold_outcome(&mut outcome, &new_records(history, &mut offset), pid);
        std::thread::sleep(Duration::from_millis(500));
    }
    anyhow::ensure!(
        outcome.applied,
        "the controller never applied a mutation to the fixture"
    );
    // The kernel's own view of the placement, not the controller's report of what it did.
    let mut physically_changed = allowed_lists(pid) != original;
    let readback = Instant::now() + Duration::from_secs(10);
    while !physically_changed && Instant::now() < readback {
        std::thread::sleep(Duration::from_millis(250));
        physically_changed = allowed_lists(pid) != original;
    }
    Ok(Applied {
        fixture,
        pid,
        original,
        physically_changed,
        offset,
    })
}

/// A workload that exits while its mutation is applied must leave nothing outstanding and nothing
/// started on its dead process id.
pub(super) fn exit_while_applied(
    out: &Path,
    user: &str,
    history: &Path,
    seconds: u64,
) -> Vec<Check> {
    let case = out.join("exit-while-applied");
    let result = (|| -> anyhow::Result<(bool, bool, usize)> {
        let applied = spawn_until_applied(&case, "StutterAcceptanceGameExit", history, seconds)?;
        let (pid, mut offset, changed) = (applied.pid, applied.offset, applied.physically_changed);
        drop(applied.fixture);
        let settled = wait_for("the ledger to settle after the exit", 90, || {
            ledger(user).is_ok_and(|(outstanding, _)| outstanding == 0)
        })
        .is_ok();
        // A little longer, watching for any action begun on the dead process id.
        let watch = Instant::now() + Duration::from_secs(10);
        let mut started_on_dead = 0;
        while Instant::now() < watch {
            started_on_dead += new_records(history, &mut offset)
                .iter()
                .filter(|record| record["target"]["root_pid"] == pid)
                .filter(|record| record["decision"]["decision"] == "candidate_applied")
                .count();
            std::thread::sleep(Duration::from_millis(500));
        }
        Ok((changed, settled, started_on_dead))
    })();
    match result {
        Ok((changed, settled, started)) => vec![
            Check::new(
                "exit_while_applied_had_a_physical_mutation",
                changed,
                "the workload's placement was read back as changed before it exited",
            ),
            Check::new(
                "exit_while_applied_settles_the_token",
                settled,
                "the root ledger had nothing outstanding after the workload exited",
            ),
            Check::new(
                "no_action_is_started_on_an_exited_workload",
                started == 0,
                format!("{started} actions started on the exited process id"),
            ),
        ],
        Err(error) => vec![Check::new(
            "exit_while_applied",
            false,
            format!("{error:#}"),
        )],
    }
}

/// What the kill case observed.
struct KillOutcome {
    changed: bool,
    restarted: bool,
    settled: bool,
    restored: bool,
    original: Vec<String>,
    after: Vec<String>,
}

/// The controller dying while a mutation is applied must be recovered after its restart: it runs as a
/// new process, the root ledger settles and the workload's original placement returns.
pub(super) fn controller_killed_while_applied(
    out: &Path,
    user: &str,
    controller: &str,
    history: &Path,
    seconds: u64,
) -> Vec<Check> {
    let case = out.join("controller-killed-while-applied");
    let result = (|| -> anyhow::Result<KillOutcome> {
        let applied = spawn_until_applied(&case, "StutterAcceptanceGameKill", history, seconds)?;
        let old_pid = main_pid(controller)?;
        run_text(
            "sudo",
            &["-n", "systemctl", "kill", "--signal=SIGKILL", controller],
        )?;
        // A new process, not an `active` state observed before the old one was reaped.
        let restarted = wait_for("the controller to restart as a new process", 90, || {
            is_active(controller)
                && main_pid(controller).is_ok_and(|pid| pid != 0 && pid != old_pid)
        })
        .is_ok()
            && !Path::new(&format!("/proc/{old_pid}")).exists();
        let settled = wait_for("the ledger to settle after the kill", 120, || {
            ledger(user).is_ok_and(|(outstanding, _)| outstanding == 0)
        })
        .is_ok();
        let after = allowed_lists(applied.pid);
        Ok(KillOutcome {
            changed: applied.physically_changed,
            restarted,
            settled,
            restored: after == applied.original,
            original: applied.original,
            after,
        })
    })();
    match result {
        Ok(KillOutcome {
            changed,
            restarted,
            settled,
            restored,
            original,
            after,
        }) => vec![
            Check::new(
                "controller_killed_while_applied_had_a_physical_mutation",
                changed,
                "the workload's placement was read back as changed before the kill",
            ),
            Check::new(
                "controller_restarts_as_a_new_process_after_a_kill",
                restarted,
                "a different main pid, the old pid reaped, the unit active",
            ),
            Check::new(
                "controller_killed_while_applied_settles_the_token",
                settled,
                "the root ledger had nothing outstanding",
            ),
            Check::new(
                "controller_killed_while_applied_restores_the_placement",
                restored,
                format!("{after:?} against {original:?}"),
            ),
        ],
        Err(error) => vec![Check::new(
            "controller_killed_while_applied",
            false,
            format!("{error:#}"),
        )],
    }
}

/// The lifecycle an administrator relies on, run while the action cooldown elapses.
pub(super) fn lifecycle(user: &str, controller: &str, worker: &str) -> Check {
    let result = (|| -> anyhow::Result<()> {
        run_text("sudo", &["-n", "systemctl", "restart", controller])?;
        wait_for("the controller to restart", 30, || is_active(controller))?;
        run_text("sudo", &["-n", "systemctl", "stop", worker])?;
        wait_for("both units to stop", 30, || {
            is_inactive(worker) && is_inactive(controller)
        })?;
        anyhow::ensure!(
            ledger(user)?.0 == 0,
            "stopping the worker left a token outstanding"
        );
        run_text("sudo", &["-n", "systemctl", "start", worker])?;
        wait_for("both units to start again", 60, || {
            is_active(worker) && is_active(controller)
        })?;
        Ok(())
    })();
    Check::new(
        "restart_stop_and_start_leave_a_clean_service",
        result.is_ok(),
        match &result {
            Ok(()) => {
                "controller restart, worker stop (controller followed), worker start".to_owned()
            }
            Err(error) => format!("{error:#}"),
        },
    )
}

pub(super) fn history_path(user: &str) -> PathBuf {
    PathBuf::from(format!(
        "/var/lib/stutter/autotune-{user}/.local/state/stutter/autotune/history.jsonl"
    ))
}
