//! `xtask diagnosis-study`: init / status / continue / verify / report. The LLM loop is
//! `status --json` then `continue` until the study is complete or genuinely blocked.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use serde_json::json;
use stutter_validation_runner::client::SOCKET_ENV;

use super::{
    capture::{Session, capture_once},
    facts::freeze_missing,
    impact::evaluate,
    inspect,
    model::{CONFIG_FILE, StudyConfig},
    plan::{Action, Slot, next_action},
    promote::promote,
    store::{Decision, Outcome, Role, RunRecord, StudyDir, StudyHeader},
};
use crate::{
    evidence_bundle::sha256::sha256_hex,
    validation_campaign::{
        self,
        bootstrap::{self, BootstrapOutcome, DEFAULT_SOCKET},
        host_factory::msi_irq_names,
        model::CampaignMode,
        repository::{Binaries, cached_file_sha256, git},
        scenarios::model::{ControlType, Family},
        state::{CampaignDir, save, unix_ms},
        system_host::{BURST_MS, BURST_PERIOD_MS, HostSettings, SystemCaptureHost},
        workloads::{SystemTools, network_irq},
    },
};

pub const STUDY_ROOT: &str = ".llm-runs/studies/diagnosis-v2";
const COMPANION_MANIFEST: &str = "docs/validation/core-validation.toml";
/// Exit codes shared with the campaign: scripts branch on them without parsing.
const EXIT_BLOCKED_RESOURCE: i32 = 11;
const EXIT_BLOCKED_PRIVILEGE: i32 = 12;

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct StudyArgs {
    #[command(subcommand)]
    pub command: StudyCommand,
}

#[derive(clap::Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum StudyCommand {
    /// Create a study at the clean `main` HEAD (and its runner-authorization campaign).
    Init,
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        study: Option<PathBuf>,
    },
    /// Take captures until the study is complete, blocked or `--max-captures` is reached.
    Continue {
        #[arg(long)]
        study: Option<PathBuf>,
        #[arg(long)]
        max_captures: Option<usize>,
    },
    Verify {
        #[arg(long)]
        study: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
    },
    Report {
        #[arg(long)]
        study: Option<PathBuf>,
    },
    /// Fixes the calibration/holdout split (once), sweeps both parts for every family whose
    /// impact study passed and records the threshold decisions. Never edits a threshold.
    Calibrate {
        #[arg(long)]
        study: Option<PathBuf>,
    },
    /// Takes one capture of `scenario` outside the plan (pair 1000+): proves a scenario's host
    /// behaviour without spending the plan's slots. Never counts toward a decision.
    #[command(hide = true)]
    Smoke {
        #[arg(long)]
        study: Option<PathBuf>,
        scenario: String,
    },
}

pub fn run(root: &Path, args: StudyArgs) -> anyhow::Result<()> {
    match args.command {
        StudyCommand::Init => init(root),
        StudyCommand::Status { json, study } => {
            let (dir, header, config) = open(root, study)?;
            let status = inspect::status(&dir, &header, &config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!(
                    "study {}: next {} blocked {}",
                    header.study_id, status["next"], status["blocked"]
                );
            }
            Ok(())
        }
        StudyCommand::Continue {
            study,
            max_captures,
        } => continue_study(root, study, max_captures),
        StudyCommand::Verify { study, strict } => {
            let (dir, header, config) = open(root, study)?;
            let problems = inspect::verify(&dir, &header, &config);
            for problem in &problems {
                println!("VERIFY FAIL: {problem}");
            }
            if problems.is_empty() {
                println!("VERIFY PASS: study {} verifies", header.study_id);
            }
            ensure!(!strict || problems.is_empty(), "strict verification failed");
            Ok(())
        }
        StudyCommand::Smoke { study, scenario } => smoke(root, study, &scenario),
        StudyCommand::Calibrate { study } => {
            let (dir, header, config) = open(root, study)?;
            let result = super::calibrate::calibrate(root, &dir, &header, &config)?;
            for (name, family) in result["families"].as_object().into_iter().flatten() {
                println!(
                    "{name}: {} selected {} changed {} logic_gap {}",
                    family["status"],
                    family["selected_threshold"],
                    family["changed"],
                    family["diagnosis_logic_gap"]
                );
            }
            println!(
                "decisions={}",
                dir.calibration().join("decisions.json").display()
            );
            Ok(())
        }
        StudyCommand::Report { study } => {
            let (dir, header, config) = open(root, study)?;
            inspect::report(&dir, &header, &config)?;
            println!("report_json={}", dir.0.join("report.json").display());
            Ok(())
        }
    }
}

fn load_config(root: &Path) -> anyhow::Result<(StudyConfig, String, Vec<u8>)> {
    let bytes = fs::read(root.join(CONFIG_FILE)).context("read the study definition")?;
    let config = StudyConfig::parse(std::str::from_utf8(&bytes)?)?;
    let sha = sha256_hex(&bytes);
    Ok((config, sha, bytes))
}

fn locate(root: &Path, explicit: Option<PathBuf>) -> anyhow::Result<StudyDir> {
    if let Some(path) = explicit {
        return Ok(StudyDir(if path.is_absolute() {
            path
        } else {
            root.join(path)
        }));
    }
    let base = root.join(STUDY_ROOT);
    // The newest study (by creation time, not by name) that has not finished.
    let mut found: Vec<(u64, PathBuf)> = fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter_map(|path| {
            let header = StudyDir(path.clone()).load_header().ok()?;
            Some((header.created_unix_ms, path))
        })
        .collect();
    found.sort();
    match found.pop() {
        Some((_, path)) => Ok(StudyDir(path)),
        None => bail!(
            "no study under {}; run `diagnosis-study init`",
            base.display()
        ),
    }
}

fn open(
    root: &Path,
    explicit: Option<PathBuf>,
) -> anyhow::Result<(StudyDir, StudyHeader, StudyConfig)> {
    let dir = locate(root, explicit)?;
    let header = dir.load_header()?;
    let text = fs::read_to_string(dir.0.join("config.toml")).context("read config.toml")?;
    let config = StudyConfig::parse(&text)?;
    Ok((dir, header, config))
}

fn init(root: &Path) -> anyhow::Result<()> {
    let (_, config_sha256, bytes) = load_config(root)?;
    let companion = validation_campaign::init_campaign(root, Path::new(COMPANION_MANIFEST))?;
    let commit = git(root, &["rev-parse", "HEAD"])?;
    let study_id = format!("v2-{}-{}", &commit[..7], unix_ms());
    let dir = StudyDir(root.join(STUDY_ROOT).join(&study_id));
    fs::create_dir_all(dir.fixtures())?;
    fs::create_dir_all(dir.impact())?;
    fs::write(dir.0.join("config.toml"), &bytes)?;
    dir.save_header(&StudyHeader {
        schema_version: super::store::SCHEMA_VERSION,
        study_id: study_id.clone(),
        base_commit: commit,
        config_sha256,
        companion_campaign: companion,
        frozen: Default::default(),
        created_unix_ms: unix_ms(),
    })?;
    println!("study_id={study_id}");
    println!("study={}", dir.0.display());
    Ok(())
}

/// Source and binaries are exactly what the study was initialized with.
fn integrity(
    root: &Path,
    header: &StudyHeader,
    companion: &validation_campaign::model::CampaignState,
) -> Result<(), String> {
    let head = git(root, &["rev-parse", "HEAD"]).map_err(|e| e.to_string())?;
    if head != header.base_commit {
        return Err(format!(
            "HEAD is {head}, the study requires {}",
            header.base_commit
        ));
    }
    if !git(root, &["status", "--porcelain"])
        .map_err(|e| e.to_string())?
        .is_empty()
    {
        return Err("the working tree is dirty".to_owned());
    }
    let binaries = Binaries::for_repository(root).map_err(|e| e.to_string())?;
    for (path, pinned, name) in [
        (&binaries.stutter, &companion.stutter_sha256, "stutter"),
        (&binaries.xtask, &companion.xtask_sha256, "xtask"),
    ] {
        if cached_file_sha256(path).map_err(|e| e.to_string())? != *pinned {
            return Err(format!("the {name} binary changed since init"));
        }
    }
    Ok(())
}

fn blocked(code: i32, message: &str) -> ! {
    println!("{message}");
    std::process::exit(code)
}

fn socket() -> PathBuf {
    std::env::var_os(SOCKET_ENV)
        .filter(|v| !v.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_SOCKET), PathBuf::from)
}

fn group_of(run: &RunRecord) -> String {
    match run.family {
        Some(family) if run.role != Role::Mixed => format!("{}-pair-{}", family.as_str(), run.pair),
        _ => format!("{}-{}", run.scenario, run.pair),
    }
}

fn promote_family(
    study: &StudyDir,
    study_id: &str,
    runs: &[RunRecord],
    family: Family,
    pairs: usize,
    digest: &str,
) -> anyhow::Result<usize> {
    let mut promoted = 0;
    for run in runs.iter().filter(|r| {
        r.accepted && r.family == Some(family) && r.role != Role::Mixed && r.pair < pairs
    }) {
        promote(study, study_id, run, &group_of(run), digest)?;
        promoted += 1;
    }
    Ok(promoted)
}

struct Prepared {
    study: StudyDir,
    header: StudyHeader,
    config: StudyConfig,
    companion: validation_campaign::Loaded,
    host: SystemCaptureHost,
}

/// Everything a capture needs: the verified study, the authorized runner and a host built from
/// the frozen facts. Blocks (and exits) with the shared exit codes when a resource is missing.
fn prepare(root: &Path, explicit: Option<PathBuf>) -> anyhow::Result<Prepared> {
    let (study, mut header, config) = open(root, explicit)?;
    let bytes = fs::read(study.0.join("config.toml"))?;
    ensure!(
        sha256_hex(&bytes) == header.config_sha256,
        "the retained study definition changed"
    );
    ensure!(
        sha256_hex(&fs::read(root.join(CONFIG_FILE))?) == header.config_sha256,
        "the committed study definition differs from the one the study started with"
    );
    let companion = validation_campaign::load_campaign(
        root,
        Some(
            root.join(".llm-runs/validation")
                .join(&header.companion_campaign),
        ),
    )?;
    ensure!(
        companion.state.mode != CampaignMode::Invalidated,
        "the study's authorization campaign was cancelled; the study is finished"
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
            "BLOCKED_PRIVILEGE_BOOTSTRAP: not root and passwordless sudo is unavailable",
        );
    }
    let scratch = companion.dir.0.join("workdir");
    if let Err(reason) = freeze_missing(&mut header.frozen, &scratch) {
        blocked(
            EXIT_BLOCKED_RESOURCE,
            &format!("BLOCKED_RESOURCE: {reason}"),
        );
    }
    study.save_header(&header)?;
    let nic = network_irq::discover(&SystemTools, &|iface| msi_irq_names(iface))
        .map_err(anyhow::Error::msg)?;
    let host = SystemCaptureHost::new(HostSettings {
        socket: socket(),
        campaign_id: companion.state.campaign_id.clone(),
        manifest_sha256: companion.state.manifest_sha256.clone(),
        task: "corpus",
        task_attempt: 0,
        capture: 0,
        scratch,
        vkmark: config.vkmark.clone(),
        nic: Some(nic),
        irq: header.frozen.irq,
        workload_version: "study-v2".to_owned(),
        burst_ms: BURST_MS,
        burst_period_ms: BURST_PERIOD_MS,
        study_cpu: header.frozen.study_cpu,
        irq_cpu: header.frozen.irq_cpu,
        fixed_work_ios: header.frozen.fixed_work_ios,
    });
    Ok(Prepared {
        study,
        header,
        config,
        companion,
        host,
    })
}

fn smoke(root: &Path, explicit: Option<PathBuf>, scenario: &str) -> anyhow::Result<()> {
    let Prepared {
        study,
        header,
        config,
        companion,
        mut host,
    } = prepare(root, explicit)?;
    let spec = config.scenario(scenario)?;
    let runs = study.runs()?;
    let seq = runs.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
    let pair = 1000 + runs.iter().filter(|r| r.pair >= 1000).count();
    let slot = Slot {
        scenario: scenario.to_owned(),
        family: spec
            .covers
            .first()
            .copied()
            .filter(|_| spec.covers.len() == 1),
        role: match spec.control {
            ControlType::NegativeControl => Role::Control,
            ControlType::Positive => Role::Positive,
            ControlType::MixedConfounded => Role::Mixed,
        },
        pair,
    };
    let session = Session {
        study: &study,
        study_id: &header.study_id,
        base_commit: &header.base_commit,
        config: &config,
        companion: &companion.dir,
    };
    let check = || integrity(root, &header, &companion.state).is_ok();
    let record = capture_once(&session, &mut host, &slot, seq, 0, &check)?;
    study.append_run(&record)?;
    println!(
        "smoke {scenario}: {} {} {}",
        if record.accepted {
            "accepted"
        } else {
            "rejected"
        },
        record.reasons.join("; "),
        record.metrics
    );
    Ok(())
}

fn continue_study(
    root: &Path,
    explicit: Option<PathBuf>,
    max_captures: Option<usize>,
) -> anyhow::Result<()> {
    let Prepared {
        study,
        header,
        config,
        companion,
        mut host,
    } = prepare(root, explicit)?;
    let session = Session {
        study: &study,
        study_id: &header.study_id,
        base_commit: &header.base_commit,
        config: &config,
        companion: &companion.dir,
    };
    let mut taken = 0_usize;
    loop {
        let (runs, decisions) = (study.runs()?, study.decisions()?);
        match next_action(&config, &runs, &decisions) {
            Action::Capture(slot) => {
                if max_captures.is_some_and(|max| taken >= max) {
                    println!("stopped after {taken} captures");
                    return Ok(());
                }
                let seq = runs.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
                let failed = runs
                    .iter()
                    .filter(|r| !r.accepted && r.scenario == slot.scenario && r.pair == slot.pair)
                    .count();
                let check = || integrity(root, &header, &companion.state).is_ok();
                if !check() {
                    blocked(
                        EXIT_BLOCKED_RESOURCE,
                        "BLOCKED_RESOURCE: source or binaries changed",
                    );
                }
                let record = capture_once(&session, &mut host, &slot, seq, failed, &check)?;
                study.append_run(&record)?;
                taken += 1;
                println!(
                    "capture {seq} {} #{}: {}{}",
                    record.scenario,
                    record.pair,
                    if record.accepted {
                        "accepted"
                    } else {
                        "rejected"
                    },
                    record
                        .reasons
                        .first()
                        .map_or_else(String::new, |r| format!(" ({r})"))
                );
                if record.accepted && record.role == Role::Mixed {
                    let digests = config
                        .mixed
                        .iter()
                        .find(|m| m.id == record.scenario)
                        .map(|m| {
                            m.covers
                                .iter()
                                .filter_map(|f| decisions.iter().rev().find(|d| d.family == *f))
                                .map(|d| d.report_sha256.clone())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default();
                    promote(
                        &study,
                        &header.study_id,
                        &record,
                        &group_of(&record),
                        &digests,
                    )?;
                }
            }
            Action::Evaluate { family, pairs } => {
                let def = config.family(family).context("family definition")?;
                let evaluation = evaluate(&config, def, &runs, pairs);
                fs::write(
                    study
                        .impact()
                        .join(format!("{}-{pairs}.json", family.as_str())),
                    serde_json::to_vec_pretty(&evaluation.report)?,
                )?;
                study.append_decision(&Decision {
                    family,
                    pairs,
                    outcome: evaluation.outcome,
                    report_sha256: evaluation.report_sha256.clone(),
                    unix_ms: unix_ms(),
                })?;
                if evaluation.outcome == Outcome::Pass {
                    promote_family(
                        &study,
                        &header.study_id,
                        &runs,
                        family,
                        pairs,
                        &evaluation.report_sha256,
                    )?;
                }
                println!(
                    "impact {} at {pairs} pairs: {:?}",
                    family.as_str(),
                    evaluation.outcome
                );
            }
            Action::Blocked(reason) => blocked(
                EXIT_BLOCKED_RESOURCE,
                &format!("BLOCKED_RESOURCE: {reason}"),
            ),
            Action::Done => {
                finish(root, &study, &header, &config, &companion.dir)?;
                println!("STUDY COMPLETE: {}", header.study_id);
                return Ok(());
            }
        }
    }
}

/// Completion marker, final report, and release of the runner authorization.
fn finish(
    root: &Path,
    study: &StudyDir,
    header: &StudyHeader,
    config: &StudyConfig,
    companion: &CampaignDir,
) -> anyhow::Result<()> {
    inspect::report(study, header, config)?;
    fs::write(
        study.0.join(inspect::COMPLETE_MARKER),
        json!({"completed_unix_ms": unix_ms()}).to_string(),
    )?;
    let loaded = validation_campaign::load_campaign(root, Some(companion.0.clone()))?;
    let mut state = loaded.state;
    state.mode = CampaignMode::Invalidated;
    save(&loaded.dir, &state)?;
    if let Some(message) = validation_campaign::kernels::cancel_runner(&state) {
        println!("{message}");
    }
    Ok(())
}
