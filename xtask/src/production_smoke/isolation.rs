//! Proof that the smoke can only act on its own disposable fixture.
//!
//! The installed controller finds its target with a *heuristic* focus source, so a real game on
//! the host is a candidate for the very mutation the smoke exercises. The smoke therefore refuses
//! to run, and never reports a product verdict, while anything that could be a competing
//! interactive workload is present:
//!
//! * a process whose command line marks it as a game or its runtime (a Steam library path with
//!   either slash, Steam's launcher, Proton, pressure-vessel, gamescope, wine and its servers or a
//!   Windows executable), which is matched on the whole command line and not on one launcher name;
//! * a host that is simply busy (more than [`MAX_HOST_BUSY_PERCENT`] of all CPU over a sample), since
//!   a quiet host is also what the idle-cost and fixture-discovery checks need.
//!
//! The check runs before anything is measured or mutated, before every fixture is started, and after
//! it is ready. Any competitor seen at any of those points makes the whole run `BlockedResource`: it
//! is an environmental block, not a pass and not a product failure, and no earlier or later
//! result in the same run is trusted. Afterwards the controller's own history is checked: every state
//! change it made must be on a fixture this run started.
use std::{fs, path::Path, sync::Mutex, time::Duration};

use serde_json::Value;

use super::{Check, matrix::is_state_changing};

/// Substrings that mark a command line as a game or its runtime.
const COMPETITOR_MARKERS: &[&str] = &[
    "steamapps/common",
    "steamapps\\common",
    "steamlaunch",
    "pressure-vessel",
    "steam-runtime",
    "proton",
    "gamescope",
    "wineserver",
    "wine64",
    "wine-preloader",
    ".exe",
];

/// More than this much of the whole host's CPU, over one second, is not a quiet host.
pub(super) const MAX_HOST_BUSY_PERCENT: f64 = 40.0;

static BLOCKS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FIXTURES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// A short description of every process that looks like a game or its runtime, skipping anything
/// whose command line lies under `own_files` (the smoke's own fixtures).
pub(super) fn competing_workloads(proc_root: &Path, own_files: &Path) -> Vec<String> {
    let own = own_files.to_string_lossy().to_ascii_lowercase();
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(raw) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        let line: String = raw
            .split(|byte| *byte == 0)
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let lower = line.to_ascii_lowercase();
        if lower.trim().is_empty() || (!own.is_empty() && lower.contains(&own)) {
            continue;
        }
        if COMPETITOR_MARKERS
            .iter()
            .any(|marker| lower.contains(marker))
        {
            let shown: String = line.chars().take(120).collect();
            found.push(format!("pid {pid}: {shown}"));
        }
    }
    found.sort();
    found
}

/// Busy and total jiffies from the aggregate `cpu` line of `/proc/stat`.
pub(super) fn parse_cpu_line(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|field| field.parse().ok())
        .collect();
    if fields.len() < 5 {
        return None;
    }
    let total: u64 = fields.iter().take(8).sum();
    // idle + iowait are not busy.
    let idle = fields[3].saturating_add(fields[4]);
    Some((total.saturating_sub(idle), total))
}

/// The share of all CPU used between two samples, in percent.
pub(super) fn busy_percent(before: (u64, u64), after: (u64, u64)) -> Option<f64> {
    let total = after.1.checked_sub(before.1)?;
    let busy = after.0.checked_sub(before.0)?;
    (total > 0).then(|| busy as f64 / total as f64 * 100.0)
}

fn host_busy_percent(proc_root: &Path) -> Option<f64> {
    let read = || parse_cpu_line(&fs::read_to_string(proc_root.join("stat")).ok()?);
    let before = read()?;
    std::thread::sleep(Duration::from_secs(1));
    busy_percent(before, read()?)
}

/// Every reason the host is not quiet: competing workloads under `proc_root`, and a busy reading
/// above the limit. Pure, so it can be tested against a fake `/proc`.
pub(super) fn reasons_at(proc_root: &Path, own_files: &Path, busy: Option<f64>) -> Vec<String> {
    let mut reasons = competing_workloads(proc_root, own_files);
    if let Some(busy) = busy
        && busy > MAX_HOST_BUSY_PERCENT
    {
        reasons.push(format!(
            "the host is {busy:.0}% busy (limit {MAX_HOST_BUSY_PERCENT:.0}%)"
        ));
    }
    reasons
}

/// Fails, and records the run as blocked, if anything could compete with the fixture.
pub(super) fn ensure_quiet(own_files: &Path, stage: &str) -> anyhow::Result<()> {
    let proc_root = Path::new("/proc");
    let reasons = reasons_at(proc_root, own_files, host_busy_percent(proc_root));
    if reasons.is_empty() {
        return Ok(());
    }
    if let Ok(mut blocks) = BLOCKS.lock() {
        blocks.extend(reasons.iter().map(|reason| format!("{stage}: {reason}")));
    }
    anyhow::bail!(
        "BlockedResource ({stage}): a competing interactive workload may be selected instead of the fixture: {}",
        reasons.join("; ")
    )
}

/// Everything recorded as a competitor during this run.
pub(super) fn blocked() -> Vec<String> {
    BLOCKS
        .lock()
        .map(|blocks| blocks.clone())
        .unwrap_or_default()
}

pub(super) fn record_fixture(pid: u32) {
    if let Ok(mut fixtures) = FIXTURES.lock() {
        fixtures.push(pid);
    }
}

pub(super) fn fixtures() -> Vec<u32> {
    FIXTURES.lock().map(|pids| pids.clone()).unwrap_or_default()
}

/// Every state change the controller made in `records` must be on a fixture this run started.
pub(super) fn only_fixture_targets(records: &[Value], fixture_pids: &[u32]) -> Check {
    let strangers: Vec<String> = records
        .iter()
        .filter(|record| {
            record["decision"]["decision"]
                .as_str()
                .is_some_and(is_state_changing)
        })
        .filter_map(|record| record["target"]["root_pid"].as_u64())
        .filter(|pid| {
            !fixture_pids
                .iter()
                .any(|fixture| u64::from(*fixture) == *pid)
        })
        .map(|pid| pid.to_string())
        .collect();
    let changes = records
        .iter()
        .filter(|record| {
            record["decision"]["decision"]
                .as_str()
                .is_some_and(is_state_changing)
        })
        .count();
    Check::new(
        "controller_changed_only_the_disposable_fixtures",
        strangers.is_empty(),
        if strangers.is_empty() {
            format!(
                "{changes} state change(s), all on the {} fixture(s) this run started",
                fixture_pids.len()
            )
        } else {
            format!(
                "the controller changed a process that is not a fixture of this run: pid {}",
                strangers.join(", ")
            )
        },
    )
}

#[cfg(test)]
#[path = "isolation_tests.rs"]
mod tests;
