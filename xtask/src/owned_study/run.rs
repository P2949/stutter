//! Runs the slots of a study under the existing supervisor, one owned process group per role.
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::DirBuilderExt,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, bail, ensure};
use stutter::api::workload_progress::FixedWorkIdentity;

pub use super::journal::Record;
use super::{
    diagnostics,
    host::{HostState, inspect},
    journal::{self, Mode},
    manifest::{Manifest, Placement, Slot, effective_cpus_of_self, parse_cpulist},
    workspace::{self, Workspace},
};
use crate::process::supervisor::{self, ExecutionPolicy, Outcome};

/// What each target prints to stderr before it execs: the kernel's own view of its CPU set and the
/// cpu.weight of the cgroup it is in. The role is `$0`; the real command is `"$@"`.
const READBACK: &str = r#"echo "READBACK $0 cpus=$(sed -n 's/^Cpus_allowed_list:[[:space:]]*//p' /proc/self/status) weight=$(cat /sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)/cpu.weight 2>/dev/null)" >&2; exec "$@""#;

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

/// Runs one slot's workload. The real runner starts owned processes; tests substitute a fake that
/// counts launches.
/// A finished, valid slot: the elapsed time and, when the study names an output artifact, its hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completed {
    pub elapsed_ns: u64,
    pub output_sha256: Option<String>,
}

pub trait SlotRunner {
    /// Called BEFORE the slot is marked started: failing here launches nothing and records nothing.
    fn prepare(&self, slot: &Slot) -> anyhow::Result<()>;
    /// `Ok(Err(reason))` is an Invalid slot (recorded); `Err` is a harness failure.
    fn run_slot(&self, slot: &Slot) -> anyhow::Result<Result<Completed, String>>;
}

fn stop_reason(state: &HostState) -> Option<String> {
    match state {
        HostState::Safe => None,
        HostState::Busy(what) => Some(format!("a game or launcher is running ({what})")),
        HostState::Incomplete(what) => Some(format!("host inspection is incomplete ({what})")),
    }
}

pub struct RealRunner<'a> {
    pub manifest: &'a Manifest,
    pub identity_path: &'a Path,
    pub workspace: &'a Workspace,
    pub out: &'a Path,
    pub allowed: &'a BTreeSet<u32>,
    pub host: &'a dyn Fn() -> HostState,
}

impl SlotRunner for RealRunner<'_> {
    fn prepare(&self, _slot: &Slot) -> anyhow::Result<()> {
        self.workspace.verify_unchanged_source()?;
        self.workspace.reset_target()?;
        Ok(())
    }

    fn run_slot(&self, slot: &Slot) -> anyhow::Result<Result<Completed, String>> {
        let manifest = self.manifest;
        let arm = manifest.arms()[slot.arm];
        let markers = self.out.join(format!("{}.markers.ndjson", slot.id));
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
            std::env::current_exe()?.display().to_string(),
            "fixed-work".into(),
            "--identity".into(),
            self.identity_path.display().to_string(),
            "--markers".into(),
            markers.display().to_string(),
        ];
        let diag_csv = self.out.join(format!("{}.perf.csv", slot.id));
        let compile_inner = if manifest.diagnostics {
            diagnostics::wrap_with_perf(compile_inner, &diag_csv)
        } else {
            compile_inner
        };
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
        let setup_stop = stop_reason(&(self.host)());
        let mut compile_command = placed_command(
            "compile",
            &arm.compile,
            &format!("stutter-study-{pid}-{}-compile", slot.id),
            &compile_inner,
        );
        compile_command
            .env("CARGO_INCREMENTAL", "0")
            .env("CARGO_TARGET_DIR", self.workspace.target_dir());
        let mut policy = ExecutionPolicy::bounded(Duration::from_secs(manifest.arm_timeout_secs));
        policy.tee = false;
        let psi_before = manifest
            .diagnostics
            .then(diagnostics::psi_cpu_some_total_us)
            .flatten();
        let compiled = match &setup_stop {
            None => Some(supervisor::run(compile_command, &policy)),
            Some(_) => None,
        };
        let psi_after = manifest
            .diagnostics
            .then(diagnostics::psi_cpu_some_total_us)
            .flatten();
        cancel.store(true, Ordering::SeqCst);
        let hogs = hogs
            .join()
            .map_err(|_| anyhow::anyhow!("the pressure thread panicked"))??;
        if let Some(why) = setup_stop {
            return Ok(Err(format!("stopped before the compile: {why}")));
        }
        let compiled = compiled.context("the compile did not start")??;
        if let Some(why) = stop_reason(&(self.host)()) {
            return Ok(Err(format!("the host became unsafe during the arm: {why}")));
        }
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
            let readback = match parse_readback(stderr, role) {
                Ok(readback) => readback,
                Err(error) => return Ok(Err(format!("{role} readback missing: {error:#}"))),
            };
            if !readback_matches(placement, self.allowed, &readback) {
                return Ok(Err(format!(
                    "{role} readback {readback:?} does not match the request"
                )));
            }
        }
        if let Err(error) = self.workspace.verify_unchanged_source() {
            return Ok(Err(format!("{error}")));
        }
        let elapsed = match elapsed_ns(&fs::read_to_string(&markers)?) {
            Ok(elapsed) => elapsed,
            Err(error) => return Ok(Err(error.to_string())),
        };
        let output_sha256 = match &manifest.output_artifact {
            None => None,
            Some(relative) => match hash_output(&self.workspace.target_dir(), relative) {
                Ok(hash) => Some(hash),
                Err(error) => return Ok(Err(format!("output artifact: {error:#}"))),
            },
        };
        if manifest.diagnostics {
            let diag = self.out.join(format!("{}.diag.json", slot.id));
            diagnostics::write_slot_diag(&diag, &diag_csv, psi_before, psi_after)?;
        }
        Ok(Ok(Completed {
            elapsed_ns: elapsed,
            output_sha256,
        }))
    }
}

/// The resume loop. Verification happens first and strictly: on any failure it returns before a
/// single process is launched or a single marker written.
pub fn run_with(
    manifest: &Manifest,
    out: &Path,
    context: &str,
    host: &dyn Fn() -> HostState,
    runner: &dyn SlotRunner,
) -> anyhow::Result<()> {
    let slots = out.join("slots");
    if !slots.exists() {
        ensure!(
            !out.join("results.jsonl").exists(),
            "a legacy results.jsonl study is read-only; start a new study directory"
        );
        fs::DirBuilder::new().mode(0o700).create(&slots)?;
    }
    let verified = journal::load(out, manifest, &Mode::Resume(context.to_owned()), false)?;
    let mut reference = verified
        .records
        .iter()
        .find(|r| r.valid)
        .and_then(|r| r.output_sha256.clone());
    for slot in verified.pending {
        if let Some(why) = stop_reason(&host()) {
            bail!("stopping before {}: {why}", slot.id);
        }
        runner.prepare(&slot)?;
        journal::begin_slot(&slots, &slot.id)?;
        let result = runner
            .run_slot(&slot)
            .unwrap_or_else(|error| Err(format!("harness error: {error:#}")));
        let (outcome, output) = match result {
            Ok(done) if manifest.output_artifact.is_some() => {
                match (&reference, done.output_sha256) {
                    (_, None) => (Err("the build produced no output hash".to_owned()), None),
                    (Some(first), Some(hash)) if *first != hash => (
                        Err(
                            "the output hash differs from the study's first completed build"
                                .to_owned(),
                        ),
                        Some(hash),
                    ),
                    (_, Some(hash)) => {
                        reference.get_or_insert_with(|| hash.clone());
                        (Ok(done.elapsed_ns), Some(hash))
                    }
                }
            }
            Ok(done) => (Ok(done.elapsed_ns), None),
            Err(reason) => (Err(reason), None),
        };
        let record = Record {
            study_id: manifest.study_id.clone(),
            manifest_sha256: manifest.digest(),
            slot: slot.id.clone(),
            block: slot.block,
            arm: manifest.arms()[slot.arm].name.clone(),
            valid: outcome.is_ok(),
            reason: outcome.as_ref().err().cloned(),
            elapsed_ns: outcome.ok(),
            context: Some(context.to_owned()),
            output_sha256: output,
        };
        journal::commit_slot(&slots, &record)?;
        println!(
            "{} valid={} elapsed_ns={:?}",
            record.slot, record.valid, record.elapsed_ns
        );
    }
    Ok(())
}

pub fn run(manifest: &Manifest, out: &Path) -> anyhow::Result<()> {
    let allowed = effective_cpus_of_self()?;
    manifest.validate(&allowed)?;
    let digest = manifest.digest();
    if !out.exists() {
        fs::DirBuilder::new().mode(0o700).create(out)?;
    }
    let identity: FixedWorkIdentity = serde_json::from_slice(&fs::read(&manifest.identity)?)?;
    let frozen = out.join("manifest.json");
    if frozen.exists() {
        ensure!(
            super::manifest::load(&frozen)?.digest() == digest,
            "the manifest changed after the study started"
        );
    } else {
        fs::write(&frozen, serde_json::to_vec_pretty(manifest)?)?;
    }
    let workspace = workspace::prepare(out, &manifest.study_id, &digest, &identity)?;
    let context = super::manifest::context_digest(&manifest.identity, &workspace.tree)?;
    let host = || inspect(Path::new("/proc"), std::process::id());
    let runner = RealRunner {
        manifest,
        identity_path: &manifest.identity,
        workspace: &workspace,
        out,
        allowed: &allowed,
        host: &host,
    };
    run_with(manifest, out, &context, &host, &runner)
}

/// sha256 of the study's output artifact, which must be a regular file inside the owned target.
pub fn hash_output(target: &Path, relative: &str) -> anyhow::Result<String> {
    let real_target = fs::canonicalize(target).context("the build target is missing")?;
    let real = fs::canonicalize(target.join(relative)).context("the output artifact is missing")?;
    ensure!(
        real.starts_with(&real_target),
        "the output artifact resolves outside the owned target"
    );
    ensure!(
        fs::metadata(&real)?.is_file(),
        "the output artifact is not a regular file"
    );
    Ok(crate::evidence_bundle::sha256::sha256_hex(&fs::read(
        &real,
    )?))
}
