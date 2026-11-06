//! The production behaviour cases of the smoke: an idle controller, a workload that exits while a
//! mutation is applied, and a controller that dies while a mutation is applied. Each is bounded by
//! its own timeout and none adds a dwell floor.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::Context;

use super::{
    Check, Outcome, allowed_lists, fold_outcome, is_active, ledger, new_records, run_text,
    spawn_fixture, wait_for,
};

/// An idle always-on controller may use at most this share of one core, averaged over the window.
pub(super) const IDLE_CPU_PERCENT_LIMIT: f64 = 5.0;
/// ...and this much resident memory.
pub(super) const IDLE_RSS_KB_LIMIT: u64 = 256 * 1024;
/// The current history file stays within its rotation cap plus one rotation's slack.
pub(super) const HISTORY_BYTES_LIMIT: u64 = 160 * 1024 * 1024;

// The idle limits are small but not zero, and the history limit sits above the rotation cap.
const _: () = assert!(IDLE_CPU_PERCENT_LIMIT > 0.0 && IDLE_CPU_PERCENT_LIMIT <= 10.0);
const _: () = assert!(IDLE_RSS_KB_LIMIT >= 64 * 1024);
const _: () = assert!(HISTORY_BYTES_LIMIT > 128 * 1024 * 1024);

pub(super) fn cpu_percent(ticks: u64, clock_ticks_per_second: u64, seconds: f64) -> f64 {
    ticks as f64 / clock_ticks_per_second as f64 / seconds * 100.0
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

/// Idle behaviour: bounded CPU and memory, no mutation of anything, and a clean state directory.
pub(super) fn idle(
    user: &str,
    controller: &str,
    history: &Path,
    window_seconds: u64,
) -> Vec<Check> {
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
        let mutations = records
            .iter()
            .filter(|record| {
                !matches!(
                    record["decision"]["decision"].as_str(),
                    Some("observed" | "noop" | "Noop" | "NoOp")
                )
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
            mutations == 0,
            format!(
                "{mutations} non-observation decisions among {} records",
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
    let _ = user;
    checks
}

/// Starts a fixture and waits until the controller has applied its mutation.
fn spawn_until_applied(
    out: &Path,
    history: &Path,
    seconds: u64,
) -> anyhow::Result<(super::Fixture, u32, Vec<String>, u64)> {
    let mut offset = fs::metadata(history).map_or(0, |metadata| metadata.len());
    let fixture = spawn_fixture(out, 2, seconds + 60)?;
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
    Ok((fixture, pid, original, offset))
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
    let result = (|| -> anyhow::Result<(bool, usize)> {
        let (fixture, pid, _, mut offset) = spawn_until_applied(&case, history, seconds)?;
        drop(fixture);
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
        Ok((settled, started_on_dead))
    })();
    match result {
        Ok((settled, started)) => vec![
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

/// The controller dying while a mutation is applied must be recovered after its restart: the root
/// ledger settles and the workload's original placement returns.
pub(super) fn controller_killed_while_applied(
    out: &Path,
    user: &str,
    controller: &str,
    history: &Path,
    seconds: u64,
) -> Vec<Check> {
    let case = out.join("controller-killed-while-applied");
    let result = (|| -> anyhow::Result<(bool, bool, Vec<String>, Vec<String>)> {
        let (fixture, pid, original, _) = spawn_until_applied(&case, history, seconds)?;
        run_text(
            "sudo",
            &["-n", "systemctl", "kill", "--signal=SIGKILL", controller],
        )?;
        let restarted = wait_for("the controller to restart", 90, || is_active(controller)).is_ok();
        let settled = wait_for("the ledger to settle after the kill", 120, || {
            ledger(user).is_ok_and(|(outstanding, _)| outstanding == 0)
        })
        .is_ok();
        let after = allowed_lists(pid);
        drop(fixture);
        Ok((restarted && settled, settled, original, after))
    })();
    match result {
        Ok((recovered, settled, original, after)) => vec![
            Check::new(
                "controller_killed_while_applied_settles_the_token",
                recovered && settled,
                "the controller restarted and the root ledger had nothing outstanding",
            ),
            Check::new(
                "controller_killed_while_applied_restores_the_placement",
                after == original,
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

pub(super) fn history_path(user: &str) -> PathBuf {
    PathBuf::from(format!(
        "/var/lib/stutter/autotune-{user}/.local/state/stutter/autotune/history.jsonl"
    ))
}
