//! Runs the slots of a study under the existing supervisor, one owned process group per role.
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use stutter::api::workload_progress::FixedWorkIdentity;

use super::manifest::{Manifest, Placement, Slot, effective_cpus_of_self, parse_cpulist};
use crate::process::supervisor::{self, ExecutionPolicy, Outcome};

/// What each target prints to stderr before it execs: the kernel's own view of its CPU set and the
/// cpu.weight of the cgroup it is in. The role is `$0`; the real command is `"$@"`.
const READBACK: &str = r#"echo "READBACK $0 cpus=$(sed -n 's/^Cpus_allowed_list:[[:space:]]*//p' /proc/self/status) weight=$(cat /sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)/cpu.weight 2>/dev/null)" >&2; exec "$@""#;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub study_id: String,
    pub manifest_sha256: String,
    pub slot: String,
    pub block: u32,
    pub arm: String,
    pub valid: bool,
    pub reason: Option<String>,
    pub elapsed_ns: Option<u64>,
    /// Digest of the kernel, boot, effective CPUs, sched_ext state and work identity the slot ran under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Readback {
    pub cpus: BTreeSet<u32>,
    pub weight: Option<u32>,
}

pub fn parse_readback(stderr: &str, role: &str) -> anyhow::Result<Readback> {
    let prefix = format!("READBACK {role} ");
    let line = stderr
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .with_context(|| format!("no readback from {role}"))?;
    let cpus = line
        .split_whitespace()
        .find_map(|field| field.strip_prefix("cpus="))
        .context("no cpus field")?;
    let weight = line
        .split_whitespace()
        .find_map(|field| field.strip_prefix("weight="))
        .and_then(|value| value.parse().ok());
    Ok(Readback {
        cpus: parse_cpulist(cpus)?,
        weight,
    })
}

/// Whether the kernel's readback equals what was requested.
pub fn readback_matches(
    placement: &Placement,
    allowed: &BTreeSet<u32>,
    readback: &Readback,
) -> bool {
    match placement {
        Placement::Unrestricted => &readback.cpus == allowed,
        Placement::Cpus(list) => parse_cpulist(list).is_ok_and(|cpus| cpus == readback.cpus),
        Placement::ScopeWeight(weight) => readback.weight == Some(*weight),
    }
}

/// A competing game or game launcher is running (a shell that merely mentions one is not).
pub fn competing_game(proc_root: &Path, own_pid: u32) -> bool {
    const MARKERS: [&str; 4] = [
        "SteamLaunch AppId",
        "steamapps/common",
        "gamescope",
        "pressure-vessel",
    ];
    const SHELLS: [&str; 6] = ["zsh", "bash", "sh", "grep", "pgrep", "xtask"];
    let Ok(entries) = fs::read_dir(proc_root) else {
        return true;
    };
    entries.flatten().any(|entry| {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            return false;
        };
        if pid == own_pid {
            return false;
        }
        let comm = fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        let cmdline = fs::read(entry.path().join("cmdline"))
            .map(|bytes| String::from_utf8_lossy(&bytes).replace('\0', " "))
            .unwrap_or_default();
        !SHELLS.contains(&comm.trim()) && MARKERS.iter().any(|marker| cmdline.contains(marker))
    })
}

/// The command that starts `inner` in `placement`, reading its placement back from inside it.
pub fn placed_command(role: &str, placement: &Placement, unit: &str, inner: &[String]) -> Command {
    let script = ["-c", READBACK, role];
    let mut command;
    match placement {
        Placement::Unrestricted => {
            command = Command::new("sh");
        }
        Placement::Cpus(list) => {
            command = Command::new("taskset");
            command.args(["-c", list, "sh"]);
        }
        Placement::ScopeWeight(weight) => {
            command = Command::new("systemd-run");
            command.args([
                "--user".to_owned(),
                "--scope".to_owned(),
                "--quiet".to_owned(),
                format!("--unit={unit}"),
                "-p".to_owned(),
                format!("CPUWeight={weight}"),
                "--".to_owned(),
                "sh".to_owned(),
            ]);
        }
    }
    command.args(script).args(inner);
    command
}

fn cold_build_directory(identity: &FixedWorkIdentity) -> anyhow::Result<PathBuf> {
    let root = PathBuf::from(&identity.working_directory);
    let target = root.join("target");
    ensure!(
        root.is_absolute() && root.join("Cargo.toml").is_file(),
        "not a cargo tree"
    );
    if let Ok(meta) = fs::symlink_metadata(&target) {
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "target is not a plain directory"
        );
    }
    Ok(target)
}

pub fn elapsed_ns(markers: &str) -> anyhow::Result<u64> {
    let mut starts = 0;
    let mut elapsed = Vec::new();
    for line in markers.lines().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value =
            serde_json::from_str(line).context("a marker is malformed")?;
        match value["phase"].as_str() {
            Some("WORKLOAD_START") => starts += 1,
            Some("WORKLOAD_COMPLETE") => elapsed.push(value["elapsed_ns"].as_u64()),
            _ => {}
        }
    }
    ensure!(
        starts == 1 && elapsed.len() == 1,
        "markers are not exactly one start and one completion"
    );
    elapsed[0].context("the completion marker has no elapsed_ns")
}

fn execute_slot(
    manifest: &Manifest,
    slot: &Slot,
    identity_path: &Path,
    identity: &FixedWorkIdentity,
    out: &Path,
    allowed: &BTreeSet<u32>,
) -> anyhow::Result<Result<u64, String>> {
    let arm = manifest.arms()[slot.arm];
    let target = cold_build_directory(identity)?;
    if target.exists() {
        fs::remove_dir_all(&target).context("remove the previous build for a cold start")?;
    }
    let markers = out.join(format!("{}.markers.ndjson", slot.id));
    let exe = std::env::current_exe()?;
    let pid = std::process::id();
    let hog_inner: Vec<String> = ["stress-ng", "--cpu"]
        .into_iter()
        .map(str::to_owned)
        .chain([
            manifest.hog_threads.to_string(),
            "--timeout".into(),
            "300s".into(),
            "--quiet".into(),
        ])
        .collect();
    let compile_inner = vec![
        exe.display().to_string(),
        "fixed-work".into(),
        "--identity".into(),
        identity_path.display().to_string(),
        "--markers".into(),
        markers.display().to_string(),
    ];
    let cancel = Arc::new(AtomicBool::new(false));
    let mut hog_policy =
        ExecutionPolicy::bounded(Duration::from_secs(manifest.arm_timeout_secs + 30));
    hog_policy.tee = false;
    hog_policy.cancel = Some(cancel.clone());
    let hog_command = placed_command(
        "hogs",
        &arm.hogs,
        &format!("stutter-study-{pid}-{}-hogs", slot.id),
        &hog_inner,
    );
    let hogs = thread::spawn(move || supervisor::run(hog_command, &hog_policy));
    thread::sleep(Duration::from_secs(2));
    let mut policy = ExecutionPolicy::bounded(Duration::from_secs(manifest.arm_timeout_secs));
    policy.tee = false;
    let mut compile_command = placed_command(
        "compile",
        &arm.compile,
        &format!("stutter-study-{pid}-{}-compile", slot.id),
        &compile_inner,
    );
    compile_command.env("CARGO_INCREMENTAL", "0");
    let compiled = supervisor::run(compile_command, &policy);
    cancel.store(true, Ordering::SeqCst);
    let hogs = hogs
        .join()
        .map_err(|_| anyhow::anyhow!("the pressure thread panicked"))??;
    let compiled = compiled?;
    if !matches!(compiled.outcome, Outcome::Exited(status) if status.success()) {
        return Ok(Err(format!(
            "compile did not succeed: {:?}",
            compiled.outcome
        )));
    }
    if !matches!(hogs.outcome, Outcome::Cancelled) {
        return Ok(Err(
            "the pressure generator did not run for the whole compile".into(),
        ));
    }
    for (role, placement, stderr) in [
        ("compile", &arm.compile, &compiled.stderr),
        ("hogs", &arm.hogs, &hogs.stderr),
    ] {
        let readback = parse_readback(stderr, role)?;
        if !readback_matches(placement, allowed, &readback) {
            return Ok(Err(format!(
                "{role} readback {readback:?} does not match the request"
            )));
        }
    }
    Ok(elapsed_ns(&fs::read_to_string(&markers)?).map_err(|error| error.to_string()))
}

pub fn run(manifest: &Manifest, out: &Path) -> anyhow::Result<()> {
    let allowed = effective_cpus_of_self()?;
    manifest.validate(&allowed)?;
    let digest = manifest.digest();
    if !out.exists() {
        fs::DirBuilder::new().mode(0o700).create(out)?;
    }
    let frozen = out.join("manifest.json");
    if frozen.exists() {
        ensure!(
            crate::evidence_bundle::sha256::sha256_hex(&serde_json::to_vec(
                &super::manifest::load(&frozen)?
            )?) == digest,
            "the manifest changed after the study started"
        );
    } else {
        fs::write(&frozen, serde_json::to_vec_pretty(manifest)?)?;
    }
    let results = out.join("results.jsonl");
    let recorded = fs::read_to_string(&results).unwrap_or_default();
    let context = super::manifest::context_digest(&manifest.identity)?;
    ensure_same_context(&recorded, &context)?;
    let identity: FixedWorkIdentity = serde_json::from_slice(&fs::read(&manifest.identity)?)?;
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&results)?;
    for slot in pending_slots(manifest, &recorded) {
        if competing_game(Path::new("/proc"), std::process::id()) {
            bail!(
                "a game or launcher is running: stopping before {} (nothing was recorded for it)",
                slot.id
            );
        }
        let outcome = execute_slot(
            manifest,
            &slot,
            &manifest.identity,
            &identity,
            out,
            &allowed,
        )?;
        let record = Record {
            study_id: manifest.study_id.clone(),
            manifest_sha256: digest.clone(),
            slot: slot.id.clone(),
            block: slot.block,
            arm: manifest.arms()[slot.arm].name.clone(),
            valid: outcome.is_ok(),
            reason: outcome.as_ref().err().cloned(),
            elapsed_ns: outcome.ok(),
            context: Some(context.clone()),
        };
        writeln!(log, "{}", serde_json::to_string(&record)?)?;
        log.flush()?;
        println!(
            "{} valid={} elapsed_ns={:?}",
            record.slot, record.valid, record.elapsed_ns
        );
    }
    Ok(())
}

/// The slots with no recorded result yet, in schedule order: a resumed study never repeats a slot.
pub fn pending_slots(manifest: &Manifest, recorded: &str) -> Vec<Slot> {
    let done: BTreeSet<String> = recorded
        .lines()
        .filter_map(|line| serde_json::from_str::<Record>(line).ok())
        .map(|record| record.slot)
        .collect();
    manifest
        .schedule()
        .into_iter()
        .filter(|slot| !done.contains(&slot.id))
        .collect()
}

/// A resumed study must run under the context it started in: another boot, kernel, scheduler,
/// CPU set or work identity would mix incomparable timings.
pub fn ensure_same_context(recorded: &str, current: &str) -> anyhow::Result<()> {
    for record in recorded
        .lines()
        .filter_map(|line| serde_json::from_str::<Record>(line).ok())
    {
        ensure!(
            record
                .context
                .as_deref()
                .is_none_or(|context| context == current),
            "the context changed since slot {} was recorded: start a new study directory",
            record.slot
        );
    }
    Ok(())
}
