//! A short exact-build smoke of the installed production service.
//!
//! It runs against the units the host already has, never installs or replaces one, and answers
//! one question for a release candidate: is the service that is running this exact, immutable
//! binary, with the intended accounts and capabilities, and does it still discover a workload,
//! apply a prepared mutation through the root worker and settle it, then stop and start cleanly
//! with nothing left outstanding? It is not the targeted acceptance (which exercises every fault)
//! and not endurance; it is the short check that a new binary has not broken the installed
//! service.
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, bail, ensure};
use clap::Args;
use serde_json::{Value, json};

use crate::evidence_metadata::{self as shared, SUITE_PACKAGED_SERVICE_SMOKE, Source};

/// Effective capabilities of the controller: BPF, PERFMON, SYS_RESOURCE, DAC_READ_SEARCH.
const CONTROLLER_CAPS: &str = "000000c001000004";
/// Effective capabilities of the worker: CHOWN, DAC_OVERRIDE, SYS_PTRACE, SYS_NICE.
const WORKER_CAPS: &str = "0000000000880003";

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct SmokeArgs {
    /// SHA-256 (hex) of the release binary this build must be running.
    #[arg(long)]
    pub binary_sha256: String,
    /// The monitored desktop account the controller runs as (default: the invoking user).
    #[arg(long)]
    pub user: Option<String>,
    /// Directory the raw evidence is written to; it must not exist.
    #[arg(long)]
    pub out: PathBuf,
    /// How long to wait for the fixture's experiment to resolve.
    #[arg(long, default_value_t = 180)]
    pub resolve_seconds: u64,
}

#[derive(Debug, Clone)]
struct Check {
    name: String,
    passed: bool,
    detail: String,
}

impl Check {
    fn new(name: &str, passed: bool, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            passed,
            detail: detail.into(),
        }
    }
}

/// `key=value` lines of `systemctl show`.
fn parse_show(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// The binary a unit executes, from the `ExecStart` property (`{ path=/x ; argv[]=... }`).
fn exec_start_path(property: &str) -> Option<String> {
    let rest = property.split("path=").nth(1)?;
    let path = rest.split(" ;").next()?.trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// A field of `/proc/<pid>/status`, as written.
fn status_field(status: &str, name: &str) -> Option<String> {
    status
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{name}:")))
        .map(|value| value.trim().to_owned())
}

/// Tokens a root ledger holds that have not been restored, and the total it holds.
fn ledger_counts(ledger: &str) -> anyhow::Result<(usize, usize)> {
    let entries: Vec<Value> =
        serde_json::from_str(ledger).context("the token ledger is not JSON")?;
    let outstanding = entries
        .iter()
        .filter(|entry| entry["restored"] != true)
        .count();
    Ok((outstanding, entries.len()))
}

/// The executable and every directory above it belong to root and nobody else can write them,
/// so no writable path can ever become root code.
fn immutable_chain(path: &Path) -> anyhow::Result<()> {
    for ancestor in path.ancestors() {
        let metadata =
            fs::metadata(ancestor).with_context(|| format!("inspect {}", ancestor.display()))?;
        // Nobody else may write the entry. A directory may be group-writable only when it is
        // sticky, which is how the Nix store lets builders add entries without being able to
        // alter or replace an existing one.
        let mode = metadata.mode();
        let group_write_is_sticky = mode & 0o020 == 0 || mode & 0o1000 != 0;
        ensure!(
            metadata.uid() == 0 && mode & 0o002 == 0 && group_write_is_sticky,
            "{} is not root-owned and protected",
            ancestor.display()
        );
    }
    Ok(())
}

fn sha256_file(path: &Path) -> anyhow::Result<String> {
    let output = Command::new("sha256sum").arg("--").arg(path).output()?;
    ensure!(output.status.success(), "sha256sum failed");
    Ok(String::from_utf8(output.stdout)?
        .split_whitespace()
        .next()
        .context("no digest")?
        .to_owned())
}

fn run_text(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program).args(args).output()?;
    ensure!(
        output.status.success(),
        "{program} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn unit_properties(unit: &str) -> anyhow::Result<BTreeMap<String, String>> {
    Ok(parse_show(&run_text(
        "systemctl",
        &[
            "show",
            "--property=ActiveState,SubState,MainPID,User,FragmentPath,ExecStart,NRestarts",
            unit,
        ],
    )?))
}

/// The unit's state checks: running, declared by the host (not transient), executing the pinned
/// immutable binary, as the intended account, with exactly the intended capabilities.
fn unit_checks(
    label: &str,
    properties: &BTreeMap<String, String>,
    expected_sha256: &str,
    expected_user: &str,
    expected_caps: &str,
    expected_uid: u32,
    require_no_new_privs: bool,
) -> Vec<Check> {
    let get = |key: &str| properties.get(key).map_or("", String::as_str);
    let mut checks = vec![Check::new(
        &format!("{label}_running"),
        get("ActiveState") == "active" && get("SubState") == "running",
        format!("{} / {}", get("ActiveState"), get("SubState")),
    )];
    let fragment = get("FragmentPath");
    checks.push(Check::new(
        &format!("{label}_declared_by_the_host"),
        fragment.starts_with("/etc/systemd/system/") || fragment.starts_with("/nix/store/"),
        fragment.to_owned(),
    ));
    let user = if get("User").is_empty() {
        "root"
    } else {
        get("User")
    };
    checks.push(Check::new(
        &format!("{label}_account"),
        user == expected_user,
        format!("runs as {user}, expected {expected_user}"),
    ));
    match exec_start_path(get("ExecStart")) {
        Some(binary) => {
            let path = PathBuf::from(&binary);
            let chain = immutable_chain(&path);
            checks.push(Check::new(
                &format!("{label}_binary_is_immutable"),
                binary.starts_with("/nix/store/") && chain.is_ok(),
                match &chain {
                    Ok(()) => binary.clone(),
                    Err(error) => format!("{binary}: {error:#}"),
                },
            ));
            let digest = sha256_file(&path).unwrap_or_default();
            checks.push(Check::new(
                &format!("{label}_binary_is_the_pinned_build"),
                digest == expected_sha256,
                format!("{digest} (expected {expected_sha256})"),
            ));
        }
        None => checks.push(Check::new(
            &format!("{label}_binary_is_immutable"),
            false,
            "the unit has no ExecStart path",
        )),
    }
    let pid = get("MainPID");
    match fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(status) => {
            let uid = status_field(&status, "Uid")
                .and_then(|value| value.split_whitespace().next().map(str::to_owned));
            checks.push(Check::new(
                &format!("{label}_process_identity"),
                uid.as_deref() == Some(&expected_uid.to_string()),
                format!("real uid {uid:?}, expected {expected_uid}"),
            ));
            let caps = status_field(&status, "CapEff").unwrap_or_default();
            checks.push(Check::new(
                &format!("{label}_capabilities"),
                caps == expected_caps,
                format!("CapEff {caps}, expected {expected_caps}"),
            ));
            if require_no_new_privs {
                let flag = status_field(&status, "NoNewPrivs").unwrap_or_default();
                checks.push(Check::new(
                    &format!("{label}_no_new_privileges"),
                    flag == "1",
                    format!("NoNewPrivs {flag}"),
                ));
            }
        }
        Err(error) => checks.push(Check::new(
            &format!("{label}_process_identity"),
            false,
            format!("/proc/{pid}/status: {error}"),
        )),
    }
    checks
}

fn ledger(user: &str) -> anyhow::Result<(usize, usize)> {
    let path = format!("/var/lib/stutter/worker-{user}/issued-tokens.json");
    ledger_counts(&run_text("sudo", &["-n", "cat", &path])?)
}

fn wait_for(what: &str, seconds: u64, mut condition: impl FnMut() -> bool) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if condition() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("timed out waiting for {what}")
}

fn is_active(unit: &str) -> bool {
    unit_properties(unit)
        .is_ok_and(|properties| properties.get("ActiveState").map(String::as_str) == Some("active"))
}

fn is_inactive(unit: &str) -> bool {
    unit_properties(unit).is_ok_and(|properties| {
        properties.get("ActiveState").map(String::as_str) == Some("inactive")
    })
}

/// History lines appended since `offset`, as JSON.
fn new_records(path: &Path, offset: &mut u64) -> Vec<Value> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map_or(0, |metadata| metadata.len());
    // A rotated or replaced file starts again.
    if length < *offset {
        *offset = 0;
    }
    if file.seek(SeekFrom::Start(*offset)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    // Only complete lines count; an in-flight append is read next time.
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    *offset += complete as u64;
    String::from_utf8_lossy(&bytes[..complete])
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// What the controller did to the fixture: it was applied, and how that experiment ended.
#[derive(Debug, Default, PartialEq, Eq)]
struct Outcome {
    applied: bool,
    resolution: Option<String>,
}

fn fold_outcome(outcome: &mut Outcome, records: &[Value], pid: u32) {
    for record in records {
        if record["target"]["root_pid"] != pid {
            continue;
        }
        match record["decision"]["decision"].as_str() {
            Some("candidate_applied") => outcome.applied = true,
            Some(resolved @ ("candidate_kept" | "candidate_reverted")) if outcome.applied => {
                outcome.resolution = Some(resolved.to_owned());
            }
            _ => {}
        }
    }
}

fn allowed_lists(pid: u32) -> Vec<String> {
    fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| fs::read_to_string(entry.path().join("status")).ok())
        .filter_map(|status| status_field(&status, "Cpus_allowed_list"))
        .collect()
}

struct Fixture {
    child: std::process::Child,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_fixture(out: &Path, cpu: u32, seconds: u64) -> anyhow::Result<Fixture> {
    let directory = out.join("steamapps/common/StutterAcceptanceGame");
    fs::create_dir_all(&directory)?;
    let game = directory.join("StutterAcceptanceGame");
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
    wait_for("the fixture to be ready", 20, || ready.exists())?;
    Ok(fixture)
}

pub fn run(root: &Path, args: &SmokeArgs) -> anyhow::Result<()> {
    ensure!(!args.out.exists(), "{} already exists", args.out.display());
    ensure!(
        args.binary_sha256.len() == 64 && args.binary_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "--binary-sha256 must be the 64 hex digit SHA-256 of the build under test"
    );
    fs::create_dir_all(&args.out)?;
    let user = match &args.user {
        Some(user) => user.clone(),
        None => run_text("id", &["-un"])?.trim().to_owned(),
    };
    let uid: u32 = run_text("id", &["-u", &user])?.trim().parse()?;
    let controller = format!("stutter-autotune-bounded@{user}.service");
    let worker = format!("stutter-privileged-worker@{user}.service");
    let mut checks = Vec::new();

    checks.push(Check::new(
        "root_ledger_readable_through_sudo",
        run_text("sudo", &["-n", "true"]).is_ok(),
        "the root token ledger can only be read through sudo",
    ));
    let controller_properties = unit_properties(&controller)?;
    let worker_properties = unit_properties(&worker)?;
    checks.extend(unit_checks(
        "controller",
        &controller_properties,
        &args.binary_sha256,
        &user,
        CONTROLLER_CAPS,
        uid,
        true,
    ));
    checks.extend(unit_checks(
        "worker",
        &worker_properties,
        &args.binary_sha256,
        "root",
        WORKER_CAPS,
        0,
        false,
    ));
    fs::write(
        args.out.join("unit-state.json"),
        serde_json::to_vec_pretty(&json!({
            "controller": controller_properties,
            "worker": worker_properties,
        }))?,
    )?;

    let (baseline_outstanding, baseline_entries) = ledger(&user).unwrap_or((usize::MAX, 0));
    checks.push(Check::new(
        "nothing_outstanding_before",
        baseline_outstanding == 0,
        format!("{baseline_outstanding} outstanding of {baseline_entries} ledger entries"),
    ));

    // A real workload: discovered, a prepared mutation applied through the worker, and settled.
    let history = PathBuf::from(format!(
        "/var/lib/stutter/autotune-{user}/.local/state/stutter/autotune/history.jsonl"
    ));
    let mut offset = fs::metadata(&history).map_or(0, |metadata| metadata.len());
    let cpus = std::thread::available_parallelism().map_or(1, |count| count.get());
    ensure!(cpus >= 4, "the fixture needs at least four CPUs");
    let fixture = spawn_fixture(&args.out, 2, args.resolve_seconds + 60)?;
    let pid = fixture.child.id();
    let original = allowed_lists(pid);
    let mut outcome = Outcome::default();
    let mut excerpt = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(args.resolve_seconds);
    let mut changed_while_applied = false;
    while Instant::now() < deadline && outcome.resolution.is_none() {
        let records = new_records(&history, &mut offset);
        excerpt.extend(
            records
                .iter()
                .filter(|record| record["target"]["root_pid"] == pid)
                .filter(|record| record["decision"]["decision"] != "observed")
                .cloned(),
        );
        fold_outcome(&mut outcome, &records, pid);
        if outcome.applied && outcome.resolution.is_none() && allowed_lists(pid) != original {
            changed_while_applied = true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    checks.push(Check::new(
        "workload_discovered_and_mutated_through_the_worker",
        outcome.applied,
        format!("applied={}", outcome.applied),
    ));
    checks.push(Check::new(
        "the_mutation_was_physically_applied",
        changed_while_applied || outcome.resolution.is_some(),
        format!("affinity changed while applied: {changed_while_applied}"),
    ));
    checks.push(Check::new(
        "the_experiment_resolved",
        outcome.resolution.is_some(),
        format!("{:?}", outcome.resolution),
    ));
    if outcome.resolution.as_deref() == Some("candidate_reverted") {
        checks.push(Check::new(
            "a_revert_restored_the_original_placement",
            allowed_lists(pid) == original,
            format!("{:?} against {:?}", allowed_lists(pid), original),
        ));
    }
    drop(fixture);
    fs::write(
        args.out.join("history-excerpt.jsonl"),
        excerpt
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )?;
    let mut settled = Check::new("tokens_settled_after_the_workload", false, "not settled");
    let _ = wait_for("the root ledger to settle", 60, || {
        ledger(&user).is_ok_and(|(outstanding, _)| outstanding == 0)
    });
    if let Ok((outstanding, entries)) = ledger(&user) {
        settled = Check::new(
            "tokens_settled_after_the_workload",
            outstanding == 0 && entries >= baseline_entries,
            format!("{outstanding} outstanding of {entries} entries (was {baseline_entries})"),
        );
    }
    checks.push(settled);

    // The lifecycle an administrator relies on.
    let lifecycle = (|| -> anyhow::Result<()> {
        run_text("sudo", &["-n", "systemctl", "restart", &controller])?;
        wait_for("the controller to restart", 30, || is_active(&controller))?;
        run_text("sudo", &["-n", "systemctl", "stop", &worker])?;
        wait_for("both units to stop", 30, || {
            is_inactive(&worker) && is_inactive(&controller)
        })?;
        ensure!(
            ledger(&user)?.0 == 0,
            "stopping the worker left a token outstanding"
        );
        run_text("sudo", &["-n", "systemctl", "start", &worker])?;
        wait_for("both units to start again", 60, || {
            is_active(&worker) && is_active(&controller)
        })?;
        Ok(())
    })();
    checks.push(Check::new(
        "restart_stop_and_start_leave_a_clean_service",
        lifecycle.is_ok(),
        match &lifecycle {
            Ok(()) => {
                "controller restart, worker stop (controller followed), worker start".to_owned()
            }
            Err(error) => format!("{error:#}"),
        },
    ));

    write_evidence(root, args, &checks)
}

fn write_evidence(root: &Path, args: &SmokeArgs, checks: &[Check]) -> anyhow::Result<()> {
    let passed = checks.iter().all(|check| check.passed);
    let binary = checks
        .iter()
        .find(|check| check.name == "controller_binary_is_immutable")
        .map(|check| PathBuf::from(&check.detail))
        .filter(|path| path.exists())
        .unwrap_or_else(|| root.join("target/release/stutter"));
    let environment = shared::capture(
        root,
        &binary,
        SUITE_PACKAGED_SERVICE_SMOKE,
        "controlled_real",
        Source::Checkout,
    )?;
    shared::write_environment(&args.out, &environment)?;
    let cases: Vec<Value> = checks
        .iter()
        .map(|check| {
            json!({
                "name": check.name,
                "status": if check.passed { "PASS" } else { "FAIL" },
                "detail": check.detail,
            })
        })
        .collect();
    shared::write_result(
        &args.out,
        SUITE_PACKAGED_SERVICE_SMOKE,
        json!({
            "status": if passed { "PASS" } else { "FAIL" },
            "evidence_kind": "controlled_real",
            "scope": "Exact-build smoke of the installed production bounded service: the running controller and worker execute the pinned immutable binary as the intended accounts with the intended capabilities, a workload fixture is discovered and a prepared mutation is applied through the root worker and settled, and the units restart, stop and start cleanly with no token outstanding.",
            "not_claimed": [
                "every dangerous and service event (the targeted acceptance suite covers those)",
                "extended endurance",
                "boot persistence (checked separately on the production service)",
                "any operation other than cpu_affinity_profile and cgroup_placement",
                "any system-wide operation",
                "real games or real user sessions",
            ],
            "cases": cases,
        }),
    )?;
    println!(
        "production-smoke: {} ({} checks); raw evidence in {}",
        if passed { "PASS" } else { "FAIL" },
        checks.len(),
        args.out.display()
    );
    for check in checks.iter().filter(|check| !check.passed) {
        println!("FAIL {}: {}", check.name, check.detail);
    }
    ensure!(passed, "the production service smoke failed");
    Ok(())
}

#[cfg(test)]
mod tests;
