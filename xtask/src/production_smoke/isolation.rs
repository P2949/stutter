//! Proof that the smoke can only act on its own disposable fixture.
//!
//! The installed controller finds its target with a *heuristic* focus source, so a real game on
//! the host is a candidate for the very mutation the smoke exercises. The smoke therefore refuses
//! to run, and never reports a product verdict, while it cannot show the host is free of anything
//! that could be a competing interactive workload. Every inspection **fails closed**: a check that
//! could not be completed is a reason to block, never a reason to proceed.
//!
//! * `/proc` that cannot be enumerated, an entry that cannot be read, a command line that stays
//!   unreadable after bounded retries, a candidate whose identity cannot be read, and a CPU-load
//!   sample that is missing, malformed, non-finite or has no elapsed time all block. Only a process
//!   that genuinely disappeared between listing and reading is skipped.
//! * A process is a competitor if its whole command line marks it as a game or its runtime (a Steam
//!   library path with either slash, Steam's launcher, Proton, pressure-vessel, gamescope, wine and
//!   its servers, a Windows executable) and it is not one of this run's fixtures.
//! * A process is one of this run's fixtures only by *exact identity*: its pid and start time, or an
//!   ancestor's, equal those recorded when the smoke started the fixture. A command line that merely
//!   lies under the smoke's output directory proves nothing.
//! * The host must be under [`MAX_HOST_BUSY_PERCENT`] busy over a sample.
//!
//! The check runs before anything is measured or mutated, before every fixture is started, and after
//! it is ready. Any competitor, or any incomplete inspection, at any of those points makes the whole
//! run `BlockedResource`: an environmental block, not a pass and not a product failure. Afterwards the
//! controller's history is audited strictly: every state-changing record must name a fixture of this
//! run, an unauditable record fails, and a history that shrank or has unparsable lines fails.
//!
//! What this does not give: proof that the heuristic controller can only select the fixture. A game
//! that matches no marker, or one that starts between checks, can still evade a preflight. The
//! authority boundary the plan wants is an exact validation-only target fence at the privileged
//! mutation boundary; until one exists, the installed-service *mutation* smoke is only as safe as
//! these checks plus a host the operator knows is free.
use std::{fs, io, path::Path, sync::Mutex, time::Duration};

use serde_json::Value;
use stutter::api::telemetry::live_process;

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

/// How many times an unreadable command line is retried before it counts as unreadable.
const READ_ATTEMPTS: u32 = 3;

/// A fixture this run started, by exact identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FixtureIdentity {
    pub pid: u32,
    pub start_ticks: u64,
    /// The kernel's `comm` for the fixture (at most 15 characters).
    pub comm: String,
}

static BLOCKS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FIXTURES: Mutex<Vec<FixtureIdentity>> = Mutex::new(Vec::new());

/// What the inspection of the host found.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Scan {
    /// Processes that look like a game or its runtime and are not a fixture of this run.
    pub competitors: Vec<String>,
    /// Inspections that could not be completed.
    pub incomplete: Vec<String>,
}

fn is_fixture(proc_root: &Path, pid: u32, fixtures: &[FixtureIdentity]) -> Result<bool, String> {
    let Some(live) = live_process(proc_root, pid) else {
        return Err(format!("pid {pid}: its identity (stat) could not be read"));
    };
    let matches = |candidate: u32, ticks: u64| {
        fixtures
            .iter()
            .any(|fixture| fixture.pid == candidate && fixture.start_ticks == ticks)
    };
    Ok(matches(pid, live.start_ticks)
        || live
            .ancestors
            .iter()
            .any(|(ancestor, ticks)| matches(*ancestor, *ticks)))
}

/// Reads a command line, retrying a read that fails for a reason other than the process having
/// gone. `Ok(None)` means the process is gone.
fn read_cmdline(proc_root: &Path, pid: u32) -> Result<Option<Vec<u8>>, String> {
    let path = proc_root.join(pid.to_string()).join("cmdline");
    let mut last = String::new();
    for attempt in 0..READ_ATTEMPTS {
        match fs::read(&path) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Gone for good only if its directory is gone too.
                if !proc_root.join(pid.to_string()).exists() {
                    return Ok(None);
                }
                last = error.to_string();
            }
            Err(error) => last = error.to_string(),
        }
        if attempt + 1 < READ_ATTEMPTS {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    Err(format!("pid {pid}: command line unreadable ({last})"))
}

/// Every process that could compete with the fixture, and every inspection that could not be made.
pub(super) fn scan(proc_root: &Path, fixtures: &[FixtureIdentity]) -> Scan {
    let mut result = Scan::default();
    let entries = match fs::read_dir(proc_root) {
        Ok(entries) => entries,
        Err(error) => {
            result.incomplete.push(format!(
                "{} cannot be enumerated: {error}",
                proc_root.display()
            ));
            return result;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                result.incomplete.push(format!(
                    "a {} entry is unreadable: {error}",
                    proc_root.display()
                ));
                continue;
            }
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let raw = match read_cmdline(proc_root, pid) {
            Ok(Some(raw)) => raw,
            Ok(None) => continue,
            Err(reason) => {
                result.incomplete.push(reason);
                continue;
            }
        };
        let line = raw
            .split(|byte| *byte == 0)
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let lower = line.to_ascii_lowercase();
        if lower.trim().is_empty()
            || !COMPETITOR_MARKERS
                .iter()
                .any(|marker| lower.contains(marker))
        {
            continue;
        }
        match is_fixture(proc_root, pid, fixtures) {
            Ok(true) => {}
            Ok(false) => {
                let shown: String = line.chars().take(120).collect();
                result.competitors.push(format!("pid {pid}: {shown}"));
            }
            Err(reason) => {
                // A process that vanished since its command line was read is not a competitor.
                if proc_root.join(pid.to_string()).exists() {
                    result.incomplete.push(reason);
                }
            }
        }
    }
    result.competitors.sort();
    result.incomplete.sort();
    result
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

fn host_busy_percent(proc_root: &Path) -> Result<f64, String> {
    let read = || -> Result<(u64, u64), String> {
        let text = fs::read_to_string(proc_root.join("stat"))
            .map_err(|error| format!("{} unreadable: {error}", proc_root.join("stat").display()))?;
        parse_cpu_line(&text).ok_or_else(|| "its cpu line is malformed".to_owned())
    };
    let before = read()?;
    std::thread::sleep(Duration::from_secs(1));
    busy_percent(before, read()?)
        .ok_or_else(|| "the counters did not advance, so there is no reading".to_owned())
}

/// Every reason the host cannot be shown to be quiet. A load reading that is missing, malformed,
/// non-finite or negative is a reason, exactly like a reading above the limit. Pure, so it can be
/// tested against a fake `/proc`.
pub(super) fn reasons_at(
    proc_root: &Path,
    fixtures: &[FixtureIdentity],
    busy: Result<f64, String>,
) -> Vec<String> {
    let scan = scan(proc_root, fixtures);
    let mut reasons = scan.competitors;
    reasons.extend(
        scan.incomplete
            .into_iter()
            .map(|reason| format!("inspection incomplete: {reason}")),
    );
    match busy {
        Ok(busy) if busy.is_finite() && busy >= 0.0 => {
            if busy > MAX_HOST_BUSY_PERCENT {
                reasons.push(format!(
                    "the host is {busy:.0}% busy (limit {MAX_HOST_BUSY_PERCENT:.0}%)"
                ));
            }
        }
        Ok(busy) => reasons.push(format!(
            "inspection incomplete: the CPU load reading {busy} is not a valid percentage"
        )),
        Err(error) => reasons.push(format!(
            "inspection incomplete: CPU load could not be measured: {error}"
        )),
    }
    reasons
}

/// Fails, and records the run as blocked, if anything could compete with the fixture or the host
/// could not be fully inspected.
pub(super) fn ensure_quiet(stage: &str) -> anyhow::Result<()> {
    let proc_root = Path::new("/proc");
    let fixtures = fixtures();
    let reasons = reasons_at(proc_root, &fixtures, host_busy_percent(proc_root));
    if reasons.is_empty() {
        return Ok(());
    }
    if let Ok(mut blocks) = BLOCKS.lock() {
        blocks.extend(reasons.iter().map(|reason| format!("{stage}: {reason}")));
    }
    anyhow::bail!(
        "BlockedResource ({stage}): the host is not shown to be free of competing interactive workloads: {}",
        reasons.join("; ")
    )
}

/// Everything recorded as a competitor or an incomplete inspection during this run.
pub(super) fn blocked() -> Vec<String> {
    BLOCKS
        .lock()
        .map(|blocks| blocks.clone())
        .unwrap_or_default()
}

/// Registers a fixture by exact identity. A fixture whose identity cannot be read is refused: it
/// could not be told apart from a stranger.
pub(super) fn record_fixture(proc_root: &Path, pid: u32) -> anyhow::Result<()> {
    let live = live_process(proc_root, pid)
        .ok_or_else(|| anyhow::anyhow!("the fixture's identity (pid {pid}) cannot be read"))?;
    let comm = fs::read_to_string(proc_root.join(pid.to_string()).join("comm"))
        .map(|text| text.trim().to_owned())
        .map_err(|error| anyhow::anyhow!("the fixture's comm cannot be read: {error}"))?;
    anyhow::ensure!(!comm.is_empty(), "the fixture has an empty comm");
    if let Ok(mut fixtures) = FIXTURES.lock() {
        fixtures.push(FixtureIdentity {
            pid,
            start_ticks: live.start_ticks,
            comm,
        });
    }
    Ok(())
}

pub(super) fn fixtures() -> Vec<FixtureIdentity> {
    FIXTURES.lock().map(|list| list.clone()).unwrap_or_default()
}

/// The controller's history since the smoke began, read strictly.
#[derive(Debug, Default)]
pub(super) struct HistoryAudit {
    pub records: Vec<Value>,
    /// Lines that were complete but not valid JSON.
    pub unparsable: usize,
    /// The file is shorter than when the smoke began: it was rotated or replaced, so records may be lost.
    pub shrank: bool,
    /// The file could not be read at all.
    pub unreadable: Option<String>,
}

pub(super) fn audit_history(path: &Path, start_length: u64) -> HistoryAudit {
    use std::io::{Read, Seek, SeekFrom};
    let mut audit = HistoryAudit::default();
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        // A history that never existed has no records; one that vanished is handled by `shrank`.
        Err(error) if error.kind() == io::ErrorKind::NotFound && start_length == 0 => return audit,
        Err(error) => {
            audit.unreadable = Some(error.to_string());
            return audit;
        }
    };
    let length = file.metadata().map_or(0, |metadata| metadata.len());
    if length < start_length {
        audit.shrank = true;
        return audit;
    }
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(start_length)).is_err() || file.read_to_end(&mut bytes).is_err() {
        audit.unreadable = Some("could not read the history".to_owned());
        return audit;
    }
    // Only complete lines count; an append still in flight is not a record yet.
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    for line in String::from_utf8_lossy(&bytes[..complete]).lines() {
        match serde_json::from_str::<Value>(line) {
            Ok(value) => audit.records.push(value),
            Err(_) => audit.unparsable += 1,
        }
    }
    audit
}

/// Every state change the controller made must be on a fixture this run started, by exact
/// identity. A state-changing record that cannot be attributed fails the check, as does a history
/// that is incomplete.
pub(super) fn only_fixture_targets(audit: &HistoryAudit, fixtures: &[FixtureIdentity]) -> Check {
    const NAME: &str = "controller_changed_only_the_disposable_fixtures";
    if let Some(error) = &audit.unreadable {
        return Check::new(
            NAME,
            false,
            format!("the controller's history could not be read: {error}"),
        );
    }
    if audit.shrank {
        return Check::new(
            NAME,
            false,
            "the controller's history is shorter than when the smoke began: it was rotated or replaced, so changes may be unaccounted for",
        );
    }
    if audit.unparsable > 0 {
        return Check::new(
            NAME,
            false,
            format!(
                "{} history line(s) are not valid JSON: changes may be unaccounted for",
                audit.unparsable
            ),
        );
    }
    let mut changes = 0_usize;
    let mut problems = Vec::new();
    for (index, record) in audit.records.iter().enumerate() {
        let Some(decision) = record["decision"]["decision"].as_str() else {
            continue;
        };
        if !is_state_changing(decision) {
            continue;
        }
        changes += 1;
        let target = &record["target"];
        let Some(pid) = target["root_pid"].as_u64() else {
            problems.push(format!(
                "record {index} ({decision}) names no parseable target pid"
            ));
            continue;
        };
        let Some(fixture) = fixtures
            .iter()
            .find(|fixture| u64::from(fixture.pid) == pid)
        else {
            problems.push(format!(
                "record {index} ({decision}) changed pid {pid}, which is not a fixture of this run"
            ));
            continue;
        };
        match target["process_comm"].as_str() {
            Some(comm) if comm.trim() == fixture.comm => {}
            Some(comm) => problems.push(format!(
                "record {index} ({decision}) names comm {comm:?} for pid {pid}, but the fixture is {:?}",
                fixture.comm
            )),
            None => problems.push(format!("record {index} ({decision}) names no process comm for pid {pid}")),
        }
        match &target["process_starttime_ticks"] {
            Value::Null => problems.push(format!(
                "record {index} ({decision}) carries no process start time for pid {pid}: a pid and comm alone cannot rule out pid reuse"
            )),
            ticks if ticks.as_u64() == Some(fixture.start_ticks) => {}
            other => problems.push(format!(
                "record {index} ({decision}) names start time {other} for pid {pid}, but the fixture started at {}",
                fixture.start_ticks
            )),
        }
    }
    if problems.is_empty() {
        Check::new(
            NAME,
            true,
            format!(
                "{changes} state change(s), all on the {} fixture(s) this run started",
                fixtures.len()
            ),
        )
    } else {
        Check::new(NAME, false, problems.join("; "))
    }
}

/// Where the worker looks for its validation fence, and the account it is for. Armed once the
/// smoke knows the user; until then no fence is written.
static FENCE: Mutex<Option<(std::path::PathBuf, u32)>> = Mutex::new(None);

pub(super) fn arm_fence(user: &str, uid: u32) -> anyhow::Result<()> {
    let mut fence = FENCE
        .lock()
        .map_err(|_| anyhow::anyhow!("the validation fence state is poisoned"))?;
    *fence = Some((
        std::path::PathBuf::from(format!(
            "/var/lib/stutter/worker-{user}/validation-fence.json"
        )),
        uid,
    ));
    Ok(())
}

/// The fence file's content: one process (pid and start time), the account, the boot and an expiry.
pub(super) fn fence_json(
    uid: u32,
    root_pid: u32,
    start_ticks: u64,
    boot_id: &str,
    expires_unix: u64,
) -> String {
    serde_json::json!({
        "version": 1,
        "uid": uid,
        "root_pid": root_pid,
        "start_ticks": start_ticks,
        "boot_id": boot_id.trim(),
        "expires_unix": expires_unix,
    })
    .to_string()
}

/// Fences the privileged worker to one fixture, by exact identity, for `seconds` plus a margin. A
/// worker build that honours the fence then refuses any other target while it stands; an older one
/// ignores the file. Written as root (`sudo -n`, as the smoke already reads the ledger) so no
/// unprivileged caller can write or lift one.
pub(super) fn write_fence(pid: u32, seconds: u64) -> anyhow::Result<String> {
    use std::io::Write;
    let armed = FENCE.lock().ok().and_then(|fence| fence.clone());
    let Some((path, uid)) = armed else {
        anyhow::bail!("no validation fence is armed: refusing to run a mutation case unfenced");
    };
    let ticks = if pid == 0 {
        0
    } else {
        live_process(Path::new("/proc"), pid)
            .ok_or_else(|| anyhow::anyhow!("the fixture's identity cannot be read to fence it"))?
            .start_ticks
    };
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .saturating_add(seconds)
        .saturating_add(120);
    let body = fence_json(uid, pid, ticks, &boot, expires);
    let mut tee = std::process::Command::new("sudo")
        .args(["-n", "tee"])
        .arg(&path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()?;
    tee.stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no stdin for the fence writer"))?
        .write_all(body.as_bytes())?;
    anyhow::ensure!(
        tee.wait()?.success(),
        "the validation fence could not be written"
    );
    let chmod = std::process::Command::new("sudo")
        .args(["-n", "chmod", "0644"])
        .arg(&path)
        .status()?;
    anyhow::ensure!(
        chmod.success(),
        "the validation fence could not be made readable"
    );
    Ok(stutter_core::hash::sha256_hex(body.as_bytes()))
}

/// The validation session between and around fixtures: the worker refuses every new target, while
/// restoring what it already holds stays possible.
pub(super) fn deny_all(seconds: u64) -> anyhow::Result<String> {
    write_fence(0, seconds)
}

/// Lifts the fence. Best effort: a fence that cannot be removed only makes the worker stricter.
pub(super) fn remove_fence() {
    let armed = FENCE.lock().ok().and_then(|fence| fence.clone());
    if let Some((path, _)) = armed {
        let _ = std::process::Command::new("sudo")
            .args(["-n", "rm", "-f"])
            .arg(&path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(test)]
#[path = "isolation_tests.rs"]
mod tests;
