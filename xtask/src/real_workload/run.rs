//! The supervised execution engine: baseline and treatment arms in a counterbalanced sequence,
//! each one a fresh `stutter game-run` execution under the process supervisor, with independent
//! capture directories, kernel GPU-health checks and resumable immutable results.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use super::{
    frames::{FrameMetrics, metric_value, metrics},
    health::gpu_faults_since,
    manifest::{Arm, Manifest},
    outcome::{PairResult, RunFacts, Verdict, classify},
};
use crate::{
    evidence_bundle::sha256::sha256_hex,
    process::supervisor::{ExecutionPolicy, Outcome, run as supervised},
};

/// A treatment this runner knows how to apply to an owned process, without privilege.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Candidate {
    /// Pin the workload to a CPU list with `taskset`.
    Taskset(String),
}

pub fn parse_candidate(text: &str) -> Option<Candidate> {
    let list = text.strip_prefix("taskset:")?;
    (!list.is_empty()
        && list
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b',' || byte == b'-'))
    .then(|| Candidate::Taskset(list.to_owned()))
}

/// Everything recorded about one execution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Execution {
    pub slot: u32,
    pub pair: u32,
    pub arm: String,
    pub exit: String,
    pub frames: usize,
    pub metrics: std::collections::BTreeMap<String, f64>,
    pub gpu_faults: Vec<String>,
    pub gpu_health_observed: bool,
}

fn arm_name(arm: Arm) -> &'static str {
    match arm {
        Arm::Baseline => "baseline",
        Arm::Treatment => "treatment",
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The argv of one execution: the wrapper, then the optional treatment prefix, then the workload.
pub fn execution_argv(
    stutter_bin: &Path,
    capture_root: &Path,
    manifest: &Manifest,
    arm: Arm,
    candidate: &Candidate,
) -> Vec<String> {
    let mut argv = vec![
        stutter_bin.display().to_string(),
        "game-run".into(),
        "--capture-root".into(),
        capture_root.display().to_string(),
        "--mangohud".into(),
        "--".into(),
    ];
    if arm == Arm::Treatment {
        let Candidate::Taskset(list) = candidate;
        argv.extend(["taskset".into(), "-c".into(), list.clone()]);
    }
    argv.extend(manifest.command.iter().cloned());
    argv
}

fn newest_log(capture_root: &Path) -> Option<PathBuf> {
    let mut logs: Vec<(SystemTime, PathBuf)> = Vec::new();
    for capture in fs::read_dir(capture_root).ok()?.flatten() {
        for file in fs::read_dir(capture.path()).ok()?.flatten() {
            let name = file.file_name().into_string().ok()?;
            if name.ends_with(".csv") && !name.contains("summary") {
                logs.push((file.metadata().ok()?.modified().ok()?, file.path()));
            }
        }
    }
    logs.sort();
    logs.pop().map(|(_, path)| path)
}

fn execute_one(
    manifest: &Manifest,
    stutter_bin: &Path,
    out: &Path,
    slot: u32,
    candidate: &Candidate,
) -> anyhow::Result<(Execution, Option<FrameMetrics>)> {
    let arm = manifest.arm_at(slot);
    let capture_root = out.join("captures").join(format!("slot-{slot:03}"));
    fs::create_dir_all(&capture_root)?;
    let argv = execution_argv(stutter_bin, &capture_root, manifest, arm, candidate);
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]).current_dir(out);
    let measured = Duration::from_secs(manifest.warmup_seconds + manifest.measure_seconds);
    // MangoHud stops and flushes its log at the end of the measured window; game-run appends the
    // output folder and per-frame interval to this.
    command.env(
        "MANGOHUD_CONFIG",
        format!("log_duration={}", measured.as_secs()),
    );
    let mut policy = ExecutionPolicy::bounded(if manifest.time_boxed {
        measured + Duration::from_secs(4)
    } else {
        Duration::from_secs(manifest.timeout_seconds)
    });
    policy.tee = false;
    let started = now_unix();
    let run = supervised(command, &policy).context("supervised execution")?;
    let exit = match &run.outcome {
        Outcome::Exited(status) => format!("exited:{}", status.code().unwrap_or(-1)),
        Outcome::TimedOut if manifest.time_boxed => "ended_by_deadline".to_owned(),
        Outcome::TimedOut => "timed_out".to_owned(),
        Outcome::Cancelled => "cancelled".to_owned(),
    };
    fs::write(
        out.join(format!("slot-{slot:03}.log")),
        format!("{}\n{}", run.stdout, run.stderr),
    )?;
    let faults = gpu_faults_since(started);
    let parsed = newest_log(&capture_root)
        .and_then(|log| fs::read_to_string(log).ok())
        .and_then(|csv| metrics(&csv, manifest.warmup_seconds as f64 * 1000.0));
    let mut values = std::collections::BTreeMap::new();
    if let Some(parsed) = &parsed {
        for name in std::iter::once(&manifest.primary.name)
            .chain(manifest.guardrails.iter().map(|guard| &guard.metric.name))
        {
            if let Some(value) = metric_value(name, parsed) {
                values.insert(name.clone(), value);
            }
        }
    }
    Ok((
        Execution {
            slot,
            pair: slot / 2,
            arm: arm_name(arm).to_owned(),
            exit,
            frames: parsed.as_ref().map_or(0, |parsed| parsed.frames),
            metrics: values,
            gpu_health_observed: faults.is_some(),
            gpu_faults: faults.unwrap_or_default(),
        },
        parsed,
    ))
}

/// The outcome of a whole run.
#[derive(Debug, Serialize)]
pub struct RunReport {
    pub manifest_digest: String,
    pub executions: usize,
    pub verdict: Verdict,
}

/// Runs, or resumes, the manifest's comparison. A slot whose result file exists is not repeated, so
/// a crash costs at most the interrupted execution, and nothing is ever re-run to chase a result.
/// `require_gpu_health`: refuse to call a run clean when the kernel log could not be read.
pub fn run(
    manifest_path: &Path,
    out: &Path,
    stutter_bin: &Path,
    require_gpu_health: bool,
) -> anyhow::Result<RunReport> {
    let manifest: Manifest = serde_json::from_slice(&fs::read(manifest_path)?)
        .with_context(|| format!("{} is not a manifest", manifest_path.display()))?;
    manifest.validate()?;
    let digest = manifest.frozen_digest()?;
    let binary_hash = sha256_hex(&fs::read(stutter_bin).context("read the stutter binary")?);
    ensure!(
        binary_hash == manifest.binary_sha256,
        "the stutter binary hash {binary_hash} differs from the frozen manifest {}",
        manifest.binary_sha256
    );
    let Some(candidate) = parse_candidate(&manifest.candidate) else {
        // An unsupported treatment is a blocked cohort, recorded rather than guessed at.
        fs::create_dir_all(out)?;
        let facts = RunFacts {
            blocked: Some(format!(
                "candidate {:?} has no unprivileged runner",
                manifest.candidate
            )),
            ..RunFacts::default()
        };
        let verdict = classify(&manifest, &facts, &[]);
        fs::write(
            out.join("verdict.json"),
            serde_json::to_string_pretty(&verdict)?,
        )?;
        return Ok(RunReport {
            manifest_digest: digest,
            executions: 0,
            verdict,
        });
    };
    if !Path::new("/proc").exists()
        || (std::env::var_os("WAYLAND_DISPLAY").is_none()
            && std::env::var_os("DISPLAY").is_none()
            && manifest.command.iter().any(|arg| arg.contains("vkcube")))
    {
        bail!(
            "vkcube needs a display session (WAYLAND_DISPLAY or DISPLAY); this cohort is BlockedResource"
        );
    }
    fs::create_dir_all(out)?;
    let frozen = out.join("manifest.json");
    match fs::read(&frozen) {
        Ok(existing) => ensure!(
            serde_json::from_slice::<Manifest>(&existing)?.frozen_digest()? == digest,
            "{} holds a different manifest: a changed manifest is a different study",
            out.display()
        ),
        Err(_) => fs::write(&frozen, serde_json::to_string_pretty(&manifest)?)?,
    }
    let mut executions = Vec::new();
    for slot in 0..manifest.max_pairs * 2 {
        let result_path = out.join(format!("slot-{slot:03}.json"));
        if let Ok(text) = fs::read_to_string(&result_path) {
            executions.push(serde_json::from_str::<Execution>(&text)?);
            continue;
        }
        let (execution, _) = execute_one(&manifest, stutter_bin, out, slot, &candidate)?;
        fs::write(&result_path, serde_json::to_string_pretty(&execution)?)?;
        let fault = !execution.gpu_faults.is_empty();
        executions.push(execution);
        if fault {
            break;
        }
    }
    let report = assemble(&manifest, &executions, require_gpu_health);
    fs::write(
        out.join("verdict.json"),
        serde_json::to_string_pretty(&report)?,
    )?;
    Ok(RunReport {
        manifest_digest: digest,
        executions: executions.len(),
        verdict: report,
    })
}

/// Pairs the executions and classifies them by the frozen manifest.
pub fn assemble(
    manifest: &Manifest,
    executions: &[Execution],
    require_gpu_health: bool,
) -> Verdict {
    let fault = executions
        .iter()
        .flat_map(|e| e.gpu_faults.first())
        .next()
        .cloned();
    let unobserved = require_gpu_health && executions.iter().any(|e| !e.gpu_health_observed);
    let names: Vec<&str> = manifest
        .guardrails
        .iter()
        .map(|g| g.metric.name.as_str())
        .collect();
    let mut pairs = Vec::new();
    let mut damaged = false;
    for pair in 0..manifest.max_pairs {
        let of = |arm: &str| executions.iter().find(|e| e.pair == pair && e.arm == arm);
        let (Some(base), Some(treat)) = (of("baseline"), of("treatment")) else {
            continue;
        };
        let value = |e: &Execution, name: &str| e.metrics.get(name).copied();
        let row = (|| {
            Some(PairResult {
                baseline_primary: value(base, &manifest.primary.name)?,
                treatment_primary: value(treat, &manifest.primary.name)?,
                baseline_guardrails: names
                    .iter()
                    .map(|n| value(base, n))
                    .collect::<Option<_>>()?,
                treatment_guardrails: names
                    .iter()
                    .map(|n| value(treat, n))
                    .collect::<Option<_>>()?,
            })
        })();
        match row {
            Some(row) if base.frames > 0 && treat.frames > 0 => pairs.push(row),
            _ => damaged = true,
        }
    }
    let facts = RunFacts {
        engineering_correct: !damaged && !pairs.is_empty(),
        safety_fault: fault,
        blocked: unobserved
            .then(|| "the kernel log could not be read, so GPU health was not observed".to_owned()),
    };
    classify(manifest, &facts, &pairs)
}
