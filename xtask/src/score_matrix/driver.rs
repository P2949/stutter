//! `xtask score-matrix`: init / status / continue / report. Resumable: progress is recomputed
//! from the append-only run log, so a killed run continues at the first missing recording.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use stutter::api::{
    recorder::{FrameEvent, IntervalRecord},
    scorer::{
        score_from_interval_records_and_frames_for_explicit_targets,
        score_from_interval_records_for_explicit_targets,
    },
};

use super::{
    analyze::{Measured, analyze},
    model::{CONFIG_FILE, MatrixConfig, Profile, STUDY_ROOT, Slot, slots},
};
use crate::{
    diagnosis_study::{
        driver::{EXIT_BLOCKED_PRIVILEGE, EXIT_BLOCKED_RESOURCE, blocked, integrity, socket},
        facts::freeze_study_cpu_only,
        metric::{merged_runnable_over_rate, merged_runnable_p99_ns},
        store::{Import, StudyDir, StudyHeader, append_ndjson, read_ndjson},
    },
    evidence_bundle::sha256::sha256_hex,
    score_evaluation::load_observation,
    validation_campaign::{
        self,
        bootstrap::{self, BootstrapOutcome},
        frame_score::capture_level,
        model::CampaignMode,
        repository::git,
        scenarios::model::{ControlType, Family, InterventionKind, Placement, ScenarioSpec},
        state::{save, unix_ms},
        system_host::{HostSettings, SystemCaptureHost, TrafficPlan},
    },
};

const COMPANION_MANIFEST: &str = "docs/validation/core-validation.toml";

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct MatrixArgs {
    #[command(subcommand)]
    pub command: MatrixCommand,
}

#[derive(clap::Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum MatrixCommand {
    /// Create a matrix study at the clean `main` HEAD (and its runner-authorization campaign).
    Init {
        /// Take over every accepted recording of an earlier study with the same definition, so
        /// only its missing cells are taken again.
        #[arg(long)]
        topup_of: Option<PathBuf>,
    },
    Status {
        #[arg(long)]
        study: Option<PathBuf>,
    },
    /// Take recordings until the matrix is complete, blocked or `--max-captures` is reached.
    Continue {
        #[arg(long)]
        study: Option<PathBuf>,
        #[arg(long)]
        max_captures: Option<usize>,
    },
    Report {
        #[arg(long)]
        study: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MatrixRun {
    pub seq: u64,
    pub profile: String,
    pub repetition: usize,
    pub position: usize,
    pub level_ms: u32,
    pub accepted: bool,
    pub reasons: Vec<String>,
    pub measured: Option<Measured>,
    pub unix_ms: u64,
}

fn runs_path(dir: &StudyDir) -> PathBuf {
    dir.0.join("matrix.ndjson")
}

pub fn run(root: &Path, args: MatrixArgs) -> anyhow::Result<()> {
    match args.command {
        MatrixCommand::Init { topup_of } => init(root, topup_of),
        MatrixCommand::Status { study } => {
            let (dir, header, config) = open(root, study)?;
            let runs: Vec<MatrixRun> = read_ndjson(&runs_path(&dir))?;
            let done = slots(&config).iter().filter(|s| accepted(&runs, s)).count();
            println!(
                "{}: {done} of {} recordings, {} attempts, complete {}",
                header.study_id,
                slots(&config).len(),
                runs.len(),
                dir.0.join("complete.json").is_file()
            );
            Ok(())
        }
        MatrixCommand::Continue {
            study,
            max_captures,
        } => continue_matrix(root, study, max_captures),
        MatrixCommand::Report { study } => {
            let (dir, header, config) = open(root, study)?;
            report(&dir, &header, &config).map(|_| ())
        }
    }
}

fn locate(root: &Path, explicit: Option<PathBuf>) -> anyhow::Result<StudyDir> {
    if let Some(path) = explicit {
        return Ok(StudyDir(if path.is_absolute() {
            path
        } else {
            root.join(path)
        }));
    }
    let mut found: Vec<(u64, PathBuf)> = fs::read_dir(root.join(STUDY_ROOT))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let dir = StudyDir(e.path());
            Some((dir.load_header().ok()?.created_unix_ms, e.path()))
        })
        .collect();
    found.sort();
    match found.pop() {
        Some((_, path)) => Ok(StudyDir(path)),
        None => bail!("no matrix study; run `score-matrix init`"),
    }
}

fn open(
    root: &Path,
    explicit: Option<PathBuf>,
) -> anyhow::Result<(StudyDir, StudyHeader, MatrixConfig)> {
    let dir = locate(root, explicit)?;
    let header = dir.load_header()?;
    let config = MatrixConfig::parse(&fs::read_to_string(dir.0.join("config.toml"))?)?;
    Ok((dir, header, config))
}

fn init(root: &Path, topup_of: Option<PathBuf>) -> anyhow::Result<()> {
    let bytes = fs::read(root.join(CONFIG_FILE)).context("read the matrix definition")?;
    MatrixConfig::parse(std::str::from_utf8(&bytes)?)?;
    let companion = validation_campaign::init_campaign(root, Path::new(COMPANION_MANIFEST))?;
    let commit = git(root, &["rev-parse", "HEAD"])?;
    let id = format!("m-{}-{}", &commit[..7], unix_ms());
    let dir = StudyDir(root.join(STUDY_ROOT).join(&id));
    fs::create_dir_all(&dir.0)?;
    fs::write(dir.0.join("config.toml"), &bytes)?;
    let mut header = StudyHeader {
        schema_version: 1,
        study_id: id.clone(),
        base_commit: commit,
        config_sha256: sha256_hex(&bytes),
        companion_campaign: companion,
        frozen: Default::default(),
        imports: Vec::new(),
        created_unix_ms: unix_ms(),
    };
    if let Some(from) = topup_of {
        let (from_dir, from_header, _) = open(root, Some(from))?;
        anyhow::ensure!(
            from_header.config_sha256 == header.config_sha256,
            "the earlier study used a different matrix definition; its recordings cannot be taken over"
        );
        let kept: Vec<MatrixRun> = read_ndjson(&runs_path(&from_dir))?;
        let mut lines = String::new();
        for run in &kept {
            lines.push_str(&serde_json::to_string(run)?);
            lines.push('\n');
            append_ndjson(&runs_path(&dir), run)?;
        }
        header.imports.push(Import {
            from_study: from_header.study_id,
            from_base_commit: from_header.base_commit,
            families: Vec::new(),
            runs: kept.len(),
            runs_sha256: sha256_hex(lines.as_bytes()),
        });
    }
    dir.save_header(&header)?;
    println!("study_id={id}\nstudy={}", dir.0.display());
    Ok(())
}

fn accepted(runs: &[MatrixRun], slot: &Slot) -> bool {
    runs.iter().any(|r| {
        r.accepted
            && r.profile == slot.profile.id()
            && r.repetition == slot.repetition
            && r.level_ms == slot.level_ms
    })
}

fn failures(runs: &[MatrixRun], slot: &Slot) -> usize {
    runs.iter()
        .filter(|r| {
            !r.accepted
                && r.profile == slot.profile.id()
                && r.repetition == slot.repetition
                && r.level_ms == slot.level_ms
        })
        .count()
}

fn spec_of(slot: &Slot, config: &MatrixConfig) -> ScenarioSpec {
    let control = slot.level_ms == 0;
    ScenarioSpec {
        id: format!("score-{}-l{}", slot.profile.id(), slot.level_ms),
        control: if control {
            ControlType::NegativeControl
        } else {
            ControlType::Positive
        },
        covers: vec![Family::Scheduler],
        target: slot.profile.target(),
        interventions: if control {
            Vec::new()
        } else {
            vec![InterventionKind::SchedulerBurst]
        },
        expected_primary_cause: if control {
            "Unknown".to_owned()
        } else {
            Family::Scheduler.positive_cause().to_owned()
        },
        seconds: config.seconds,
        count: 1,
        accepted_wakeup_coalescing: slot.profile.wakeup_allowance(),
        accepted_block_collisions: 16,
        placement: Placement::StudyCpu,
        informational: true,
    }
}

/// Score and direct metrics of one recording. The target's own runnable population is merged
/// across task classes: the matrix targets are not games, so the game-class signal is empty.
fn measure(raw: &Path, level_ms: u32) -> anyhow::Result<Measured> {
    let observation = load_observation(f64::from(level_ms), raw, true)?;
    let pid = observation
        .identity
        .map(|i| i.pid)
        .context("no target identity")?;
    let records = fs::read_to_string(raw.join("interval.json"))?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<Vec<_>, _>>()?;
    let frames = load_frames(raw)?;
    let explicit = explicit_target_scores(&records, &frames, pid);
    Ok(Measured {
        level_ms: f64::from(level_ms),
        explicit_target_score_total: explicit.0,
        explicit_target_runnable_score_total: explicit.1,
        score_total: observation.score_total,
        runnable_score_total: observation.runnable_score_total,
        over_1ms: observation.over_1ms,
        over_2ms: observation.over_2ms,
        over_5ms: observation.over_5ms,
        runnable_p99_ns: merged_runnable_p99_ns(&records, pid),
        runnable_over_2ms_rate: merged_runnable_over_rate(&records, pid, 2_000_000),
        runnable_over_5ms_rate: merged_runnable_over_rate(&records, pid, 5_000_000),
        frame_p99_ms: observation.frame_p99_ms,
        frame_p99_9_ms: observation.frame_p99_9_ms,
        frame_over_16_6_rate: observation.frame_over_16_6_rate,
        frame_over_33_3_rate: observation.frame_over_33_3_rate,
        frame_samples: observation.frame_samples,
    })
}

/// The production score of the target's records after generic-class tasks of the named process
/// count as the target: `(frame-inclusive total, runnable-only total)`.
fn explicit_target_scores(
    records: &[IntervalRecord],
    frames: &[FrameEvent],
    pid: u32,
) -> (f64, f64) {
    let target: Vec<IntervalRecord> = records
        .iter()
        .filter(|r| r.process_pid == Some(pid))
        .cloned()
        .collect();
    let runnable = score_from_interval_records_for_explicit_targets(&target, &[pid]).total as f64;
    let total = if frames.is_empty() {
        runnable
    } else {
        score_from_interval_records_and_frames_for_explicit_targets(&target, frames, &[pid]).total
            as f64
    };
    (total, runnable)
}

fn load_frames(raw: &Path) -> anyhow::Result<Vec<FrameEvent>> {
    for name in ["frame_correlation.json", "frame_events.json"] {
        let path = raw.join(name);
        if path.is_file() {
            return fs::read_to_string(&path)?
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).map_err(Into::into))
                .collect();
        }
    }
    Ok(Vec::new())
}

fn continue_matrix(
    root: &Path,
    explicit: Option<PathBuf>,
    max_captures: Option<usize>,
) -> anyhow::Result<()> {
    let (dir, mut header, config) = open(root, explicit)?;
    let companion = validation_campaign::load_campaign(
        root,
        Some(
            root.join(".llm-runs/validation")
                .join(&header.companion_campaign),
        ),
    )?;
    anyhow::ensure!(
        companion.state.mode != CampaignMode::Invalidated,
        "the matrix's authorization campaign was cancelled; it is finished"
    );
    if let Err(reason) = integrity(root, &header, &companion.state) {
        blocked(
            EXIT_BLOCKED_RESOURCE,
            &format!("BLOCKED_RESOURCE: {reason}"),
        );
    }
    let outcome = bootstrap::bootstrap(
        &companion.dir,
        &companion.state,
        &companion.manifest,
        &socket(),
        false,
    )?;
    if outcome == BootstrapOutcome::BlockedPrivilege {
        blocked(
            EXIT_BLOCKED_PRIVILEGE,
            "BLOCKED_PRIVILEGE_BOOTSTRAP: no privilege",
        );
    }
    if let Err(reason) = freeze_study_cpu_only(&mut header.frozen) {
        blocked(
            EXIT_BLOCKED_RESOURCE,
            &format!("BLOCKED_RESOURCE: {reason}"),
        );
    }
    dir.save_header(&header)?;
    let path = runs_path(&dir);
    let mut taken = 0;
    // Until nothing is missing: a rejected attempt is retried within its budget, never skipped.
    loop {
        let runs: Vec<MatrixRun> = read_ndjson(&path)?;
        let Some(slot) = slots(&config).into_iter().find(|s| !accepted(&runs, s)) else {
            break;
        };
        if failures(&runs, &slot) >= config.failed_attempts_per_capture {
            blocked(
                EXIT_BLOCKED_RESOURCE,
                &format!(
                    "BLOCKED_RESOURCE: {} L{} #{} failed repeatedly",
                    slot.profile.id(),
                    slot.level_ms,
                    slot.repetition
                ),
            );
        }
        if max_captures.is_some_and(|max| taken >= max) {
            println!("stopped after {taken} recordings");
            return Ok(());
        }
        if integrity(root, &header, &companion.state).is_err() {
            blocked(
                EXIT_BLOCKED_RESOURCE,
                "BLOCKED_RESOURCE: source or binaries changed",
            );
        }
        let seq = runs.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
        let raw = companion
            .dir
            .attempt_dir("corpus", u32::try_from(seq)?)
            .join("captures/0001/raw");
        let spec = spec_of(&slot, &config);
        let mut host = SystemCaptureHost::new(HostSettings {
            socket: socket(),
            campaign_id: companion.state.campaign_id.clone(),
            manifest_sha256: companion.state.manifest_sha256.clone(),
            task: "corpus",
            task_attempt: 0,
            capture: 0,
            scratch: companion.dir.0.join("workdir"),
            vkmark: config.vkmark.clone(),
            nic: None,
            irq: None,
            workload_version: "score-matrix-v1".to_owned(),
            burst_ms: u64::from(slot.level_ms),
            burst_period_ms: config.period_ms,
            study_cpu: header.frozen.study_cpu,
            irq_cpu: None,
            fixed_work_ios: None,
            gpu_stressors: 1,
            traffic: TrafficPlan::v0_1(),
        });
        let outcome = capture_level(&mut host, &spec, &raw)
            .map_err(|e| e.to_string())
            .and_then(|()| measure(&raw, slot.level_ms).map_err(|e| format!("{e:#}")));
        let (ok, reasons, measured) = match outcome {
            Ok(m) => (true, Vec::new(), Some(m)),
            Err(reason) => (false, vec![reason], None),
        };
        println!(
            "recording {seq} {} L{} rep {}: {}{}",
            slot.profile.id(),
            slot.level_ms,
            slot.repetition,
            if ok { "accepted" } else { "rejected" },
            reasons
                .first()
                .map_or_else(String::new, |r| format!(" ({r})"))
        );
        append_ndjson(
            &path,
            &MatrixRun {
                seq,
                profile: slot.profile.id().to_owned(),
                repetition: slot.repetition,
                position: slot.position,
                level_ms: slot.level_ms,
                accepted: ok,
                reasons,
                measured,
                unix_ms: unix_ms(),
            },
        )?;
        taken += 1;
    }
    report(&dir, &header, &config)?;
    fs::write(
        dir.0.join("complete.json"),
        json!({"completed_unix_ms": unix_ms()}).to_string(),
    )?;
    let mut state = companion.state;
    state.mode = CampaignMode::Invalidated;
    save(&companion.dir, &state)?;
    if let Some(message) = validation_campaign::kernels::cancel_runner(&state) {
        println!("{message}");
    }
    println!("MATRIX COMPLETE: {}", header.study_id);
    Ok(())
}

fn report(
    dir: &StudyDir,
    header: &StudyHeader,
    config: &MatrixConfig,
) -> anyhow::Result<serde_json::Value> {
    let runs: Vec<MatrixRun> = read_ndjson(&runs_path(dir))?;
    let mut by_profile: BTreeMap<Profile, Vec<Measured>> = BTreeMap::new();
    for slot in slots(config) {
        if let Some(run) = runs.iter().find(|r| {
            r.accepted
                && r.profile == slot.profile.id()
                && r.repetition == slot.repetition
                && r.level_ms == slot.level_ms
        }) && let Some(measured) = &run.measured
        {
            by_profile
                .entry(slot.profile)
                .or_default()
                .push(measured.clone());
        }
    }
    let rejected: Vec<_> = runs.iter().filter(|r| !r.accepted).map(|r| json!({"profile": r.profile, "level_ms": r.level_ms, "repetition": r.repetition, "reasons": r.reasons})).collect();
    let report = json!({
        "study_id": header.study_id,
        "base_commit": header.base_commit,
        "recordings": by_profile.values().map(Vec::len).sum::<usize>(),
        "planned": slots(config).len(),
        "rejected_attempts": rejected,
        "analysis": analyze(&by_profile),
        "limitations": [
            "single host; the desktop was in use during collection",
            "the contender is a SCHED_FIFO burst pinned to the target's CPU; levels order scheduler degradation only",
            "vkcube is a second rendering workload but a trivial scene: its 'heavy' profile is unpaced and CPU-bound, not GPU-heavy",
        ],
    });
    fs::write(
        dir.0.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(report)
}
