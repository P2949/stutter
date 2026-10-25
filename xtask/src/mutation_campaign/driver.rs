//! `xtask mutation-campaign`: init / status / continue / verify / report / cancel. The loop for
//! an agent is `status --json`, then `continue`, until the campaign is complete or genuinely
//! blocked. State is durable after every attempt; nothing depends on a conversation.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use stutter_validation_runner::client::SOCKET_ENV;

use super::{
    graph::Stage,
    inspect,
    manifest::{MANIFEST_FILE, MutationManifest},
    model::{MutationMode, MutationState, SCHEMA_VERSION},
    runner::{StageContext, StageOutcome, StageRunner},
    runner_auth, store,
};
use crate::validation_campaign::{
    self,
    bootstrap::{self, BootstrapOutcome, DEFAULT_SOCKET},
    model::{
        AttemptOutcome, BlockReason, Boundary, FailureClass, TaskAttempt, TaskFingerprint,
        TaskState, TaskStatus,
    },
    repository::{Binaries, cached_file_sha256, git},
    state::{CampaignDir, unix_ms},
};

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct MutationArgs {
    #[command(subcommand)]
    pub command: MutationCommand,
}

#[derive(clap::Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum MutationCommand {
    /// Create a campaign for one operation at the clean `main` HEAD.
    Init { operation: String },
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
    /// Run stages until the campaign is complete, blocked or `--max-stages` is reached.
    Continue {
        #[arg(long)]
        campaign: Option<PathBuf>,
        #[arg(long)]
        max_stages: Option<usize>,
        /// Re-entry after a controlled reboot: continue from the durable state.
        #[arg(long)]
        resume: bool,
    },
    Verify {
        #[arg(long)]
        campaign: Option<PathBuf>,
        #[arg(long)]
        strict: bool,
    },
    Report {
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
    Cancel {
        #[arg(long)]
        campaign: Option<PathBuf>,
    },
}

/// Process exit code per boundary, shared with the Core Validation campaign.
pub fn exit_code(boundary: Boundary) -> i32 {
    validation_campaign::exit_code(boundary)
}

pub fn run(root: &Path, args: MutationArgs) -> anyhow::Result<()> {
    match args.command {
        MutationCommand::Init { operation } => init(root, &operation),
        MutationCommand::Status { json, campaign } => {
            let dir = store::locate(root, campaign)?;
            let state = store::load(&dir)?;
            let status = inspect::status(&dir, &state)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!(
                    "mutation campaign {}: operation {} mode {:?} next {} blocked {}",
                    state.campaign_id,
                    state.operation,
                    state.mode,
                    status["next"],
                    status["blocked"]
                );
            }
            Ok(())
        }
        MutationCommand::Continue {
            campaign, resume, ..
        } if resume && campaign.is_none() && store::locate(root, None).is_err() => {
            bail!("nothing to resume: no mutation campaign exists")
        }
        MutationCommand::Continue {
            campaign,
            max_stages,
            ..
        } => {
            let boundary = continue_campaign(root, campaign, max_stages)?;
            println!("boundary={boundary:?}");
            let code = exit_code(boundary);
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        MutationCommand::Verify { campaign, strict } => {
            let dir = store::locate(root, campaign)?;
            let state = store::load(&dir)?;
            let problems = inspect::verify(root, &dir, &state);
            for problem in &problems {
                println!("VERIFY FAIL: {problem}");
            }
            if problems.is_empty() {
                println!("VERIFY PASS: campaign {} verifies", state.campaign_id);
            }
            ensure!(!strict || problems.is_empty(), "strict verification failed");
            Ok(())
        }
        MutationCommand::Report { campaign } => {
            let dir = store::locate(root, campaign)?;
            let state = store::load(&dir)?;
            let path = inspect::report(&dir, &state)?;
            println!("report_json={}", path.display());
            Ok(())
        }
        MutationCommand::Cancel { campaign } => {
            let dir = store::locate(root, campaign)?;
            let mut state = store::load(&dir)?;
            if state.mode == MutationMode::Active {
                state.mode = MutationMode::Cancelled;
                store::save(&dir, &state)?;
            }
            println!("cancelled={}", state.campaign_id);
            Ok(())
        }
    }
}

fn init(root: &Path, operation: &str) -> anyhow::Result<()> {
    let (manifest, manifest_sha256) = MutationManifest::load(&root.join(MANIFEST_FILE))?;
    let section = manifest.operation(operation)?;
    let stages = section.stage_list()?;
    let evidence_base = git(root, &["rev-parse", "HEAD"])?;
    ensure!(
        git(root, &["status", "--porcelain"])?.is_empty(),
        "refusing to start a campaign on a dirty working tree"
    );
    let binaries = Binaries::for_repository(root)?;
    validation_campaign::repository::check_binary_revision(&binaries.stutter, &evidence_base)?;
    let campaign_id = format!(
        "mut-{}-{}-{}",
        operation.replace('_', "-"),
        &evidence_base[..7],
        unix_ms()
    );
    let dir = store::dir_for(root, &campaign_id);
    fs::create_dir_all(&dir.0)?;
    fs::copy(root.join(MANIFEST_FILE), store::manifest_snapshot(&dir))?;
    let state = MutationState {
        schema_version: SCHEMA_VERSION,
        campaign_id: campaign_id.clone(),
        generation: 1,
        mode: MutationMode::Active,
        repository: root.to_path_buf(),
        evidence_base,
        manifest_sha256,
        cargo_lock_sha256: cached_file_sha256(&root.join("Cargo.lock"))?,
        xtask_sha256: cached_file_sha256(&binaries.xtask)?,
        stutter_sha256: cached_file_sha256(&binaries.stutter)?,
        uid: nix_uid(),
        gid: nix_gid(),
        created_unix_ms: unix_ms(),
        operation: operation.to_owned(),
        objective: section.objective.clone(),
        tasks: stages
            .iter()
            .map(|stage| (stage.as_str().to_owned(), TaskState::new(stage.as_str())))
            .collect(),
        skipped: section
            .skip
            .iter()
            .map(|entry| (entry.stage.clone(), entry.reason.clone()))
            .collect(),
        decision: None,
        frozen: Default::default(),
    };
    store::save(&dir, &state)?;
    println!("campaign_id={campaign_id}");
    println!("campaign={}", dir.0.display());
    Ok(())
}

fn nix_uid() -> u32 {
    std::fs::metadata("/proc/self").map_or(0, |m| std::os::unix::fs::MetadataExt::uid(&m))
}

fn nix_gid() -> u32 {
    std::fs::metadata("/proc/self").map_or(0, |m| std::os::unix::fs::MetadataExt::gid(&m))
}

/// The ordered stages of this campaign.
pub fn ordered_stages(state: &MutationState) -> Vec<Stage> {
    Stage::ALL
        .into_iter()
        .filter(|stage| state.tasks.contains_key(stage.as_str()))
        .collect()
}

/// A failure that ends the experiment honestly (not a defect, not an environment problem).
fn is_validation_failure(task: &TaskState) -> bool {
    task.status == TaskStatus::Failed
        && task.attempts.last().is_some_and(|attempt| {
            matches!(
                attempt.outcome,
                AttemptOutcome::Failed {
                    class: FailureClass::ValidationFailure,
                    ..
                }
            )
        })
}

/// What to do next: a stage, nothing (complete/blocked), and why.
#[derive(Debug, Eq, PartialEq)]
pub enum Next {
    Stage(Stage),
    Complete,
    Blocked(BlockReason),
}

pub fn next_stage(state: &MutationState, default_attempts: u32) -> Next {
    let stages = ordered_stages(state);
    for stage in stages.iter().filter(|stage| **stage != Stage::TierDecision) {
        let task = &state.tasks[stage.as_str()];
        match task.status {
            TaskStatus::Passed => {}
            // The experiment ended honestly: the tier decision records the deferral.
            _ if is_validation_failure(task) => break,
            TaskStatus::Failed => {
                let retryable = task.failed_attempts() < default_attempts as usize
                    && task.attempts.last().is_some_and(|attempt| {
                        matches!(
                            attempt.outcome,
                            AttemptOutcome::Failed {
                                class: FailureClass::Retryable,
                                ..
                            }
                        )
                    });
                return if retryable {
                    Next::Stage(*stage)
                } else {
                    Next::Blocked(BlockReason {
                        class: FailureClass::SoftwareDefect,
                        detail: format!("stage {} failed", stage.as_str()),
                    })
                };
            }
            // A block is re-checked on every `continue`: the resource may exist now.
            _ => return Next::Stage(*stage),
        }
    }
    match state.tasks.get(Stage::TierDecision.as_str()) {
        Some(task) if task.status == TaskStatus::Passed => Next::Complete,
        Some(_) => Next::Stage(Stage::TierDecision),
        None => Next::Blocked(BlockReason {
            class: FailureClass::SoftwareDefect,
            detail: "the campaign has no tier_decision stage".to_owned(),
        }),
    }
}

/// Source and binaries are exactly what the campaign was initialized with.
pub fn integrity(root: &Path, dir: &CampaignDir, state: &MutationState) -> Result<(), String> {
    let check = || -> anyhow::Result<()> {
        let head = git(root, &["rev-parse", "HEAD"])?;
        ensure!(
            head == state.evidence_base,
            "HEAD is {head}, the campaign requires {}",
            state.evidence_base
        );
        ensure!(
            git(root, &["status", "--porcelain"])?.is_empty(),
            "working tree is dirty"
        );
        ensure!(
            cached_file_sha256(&root.join(MANIFEST_FILE))? == state.manifest_sha256,
            "the mutation manifest changed"
        );
        ensure!(
            cached_file_sha256(&store::manifest_snapshot(dir))? == state.manifest_sha256,
            "the retained manifest snapshot changed"
        );
        ensure!(
            cached_file_sha256(&root.join("Cargo.lock"))? == state.cargo_lock_sha256,
            "Cargo.lock changed"
        );
        let binaries = Binaries::for_repository(root)?;
        ensure!(
            cached_file_sha256(&binaries.stutter)? == state.stutter_sha256,
            "the stutter binary changed since init"
        );
        ensure!(
            cached_file_sha256(&binaries.xtask)? == state.xtask_sha256,
            "the xtask binary changed since init"
        );
        Ok(())
    };
    check().map_err(|error| format!("{error:#}"))
}

pub fn fingerprint(state: &MutationState, stage: Stage) -> TaskFingerprint {
    TaskFingerprint {
        task: stage.as_str().to_owned(),
        evidence_commit: state.evidence_base.clone(),
        manifest_sha256: state.manifest_sha256.clone(),
        stutter_sha256: Some(state.stutter_sha256.clone()),
        xtask_sha256: Some(state.xtask_sha256.clone()),
        ..TaskFingerprint::default()
    }
}

fn boundary_for(block: &BlockReason) -> Boundary {
    match block.class {
        FailureClass::BlockedPrivilege => Boundary::BlockedPrivilege,
        FailureClass::BlockedResource => Boundary::BlockedResource,
        _ => Boundary::FailureRequiresSourceChange,
    }
}

fn continue_campaign(
    root: &Path,
    explicit: Option<PathBuf>,
    max_stages: Option<usize>,
) -> anyhow::Result<Boundary> {
    let dir = store::locate(root, explicit)?;
    let (manifest, _) = MutationManifest::load(&store::manifest_snapshot(&dir))
        .context("read the retained manifest snapshot")?;
    let state = store::load(&dir)?;
    let operation = manifest.operation(&state.operation)?;
    let socket = std::env::var_os(SOCKET_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_SOCKET), PathBuf::from);
    if state.mode == MutationMode::Active
        && bootstrap::bootstrap_target(
            &dir,
            &runner_auth::target(&state, operation, root),
            &socket,
            false,
        )? == BootstrapOutcome::BlockedPrivilege
    {
        println!("BLOCKED_PRIVILEGE_BOOTSTRAP: not root and passwordless sudo is unavailable");
        return Ok(Boundary::BlockedPrivilege);
    }
    let runner = production_runner(root)?;
    drive(root, &dir, &manifest, &runner, max_stages)
}

fn production_runner(_root: &Path) -> anyhow::Result<super::stages::ProductionRunner> {
    Ok(super::stages::ProductionRunner {
        suites: validation_campaign::backend::backend_from_env(),
        suspend: Box::new(validation_campaign::backend::DirectBackend),
        harness: Box::new(super::runner::UnavailableHarness),
    })
}

pub fn drive(
    root: &Path,
    dir: &CampaignDir,
    manifest: &MutationManifest,
    runner: &dyn StageRunner,
    max_stages: Option<usize>,
) -> anyhow::Result<Boundary> {
    drive_with(root, dir, manifest, runner, max_stages, &|state| {
        integrity(root, dir, state)
    })
}

/// The state machine, with the source-integrity check injected so tests drive it with scripted
/// runners and no repository.
pub fn drive_with(
    root: &Path,
    dir: &CampaignDir,
    manifest: &MutationManifest,
    runner: &dyn StageRunner,
    max_stages: Option<usize>,
    check_integrity: &dyn Fn(&MutationState) -> Result<(), String>,
) -> anyhow::Result<Boundary> {
    let mut ran = 0usize;
    loop {
        let mut state = store::load(dir)?;
        ensure!(
            state.mode == MutationMode::Active || state.mode == MutationMode::Complete,
            "the campaign is {:?}; nothing to continue",
            state.mode
        );
        let operation = manifest.operation(&state.operation)?;
        if state.mode == MutationMode::Complete {
            return Ok(Boundary::Complete);
        }
        if let Err(reason) = check_integrity(&state) {
            state.mode = MutationMode::Invalidated;
            store::save(dir, &state)?;
            println!("INVALIDATED: {reason}");
            return Ok(Boundary::FailureRequiresSourceChange);
        }
        let stage = match next_stage(&state, manifest.retry.default_attempts) {
            Next::Complete => {
                state.mode = MutationMode::Complete;
                store::save(dir, &state)?;
                return Ok(Boundary::Complete);
            }
            Next::Blocked(reason) => {
                println!("BLOCKED: {}", reason.detail);
                return Ok(boundary_for(&reason));
            }
            Next::Stage(stage) => stage,
        };
        if max_stages.is_some_and(|limit| ran >= limit) {
            return Ok(Boundary::Idle);
        }
        ran += 1;
        let binaries = Binaries::for_repository(root)?;
        let attempt = state.tasks[stage.as_str()].attempts.len() as u32 + 1;
        let started = unix_ms();
        let outcome = {
            let context = StageContext {
                repository: root,
                dir,
                state: &state,
                manifest,
                operation,
                binaries: &binaries,
            };
            runner.run(&context, stage, attempt)
        };
        let finished = unix_ms();
        let fingerprint = fingerprint(&state, stage);
        let mut boundary = None;
        let task = state
            .tasks
            .get_mut(stage.as_str())
            .context("stage state vanished")?;
        let (attempt_outcome, artifacts, receipt, limitations, status, block) = match outcome {
            StageOutcome::Passed {
                artifacts,
                receipt,
                limitations,
            } => (
                AttemptOutcome::Passed,
                artifacts,
                receipt,
                limitations,
                TaskStatus::Passed,
                None,
            ),
            StageOutcome::Failed {
                class,
                stage: failed_stage,
                summary,
                artifacts,
            } => (
                AttemptOutcome::Failed {
                    class,
                    stage: failed_stage,
                    summary,
                },
                artifacts,
                None,
                Vec::new(),
                TaskStatus::Failed,
                None,
            ),
            StageOutcome::Blocked(reason) => {
                boundary = Some(boundary_for(&reason));
                (
                    AttemptOutcome::Blocked(reason.clone()),
                    None,
                    None,
                    Vec::new(),
                    match reason.class {
                        FailureClass::BlockedPrivilege => TaskStatus::BlockedPrivilege,
                        _ => TaskStatus::BlockedResource,
                    },
                    Some(reason),
                )
            }
        };
        task.attempts.push(TaskAttempt {
            number: attempt,
            started_unix_ms: started,
            finished_unix_ms: finished,
            outcome: attempt_outcome,
            fingerprint: fingerprint.clone(),
            artifacts,
            receipt,
            limitations,
        });
        task.status = status;
        task.fingerprint = (status == TaskStatus::Passed).then_some(fingerprint);
        task.block = block;
        if stage == Stage::TierDecision && status == TaskStatus::Passed {
            state.decision = Some(inspect::read_decision(dir, &state)?);
            state.mode = MutationMode::Complete;
        }
        store::save(dir, &state)?;
        if let Some(boundary) = boundary {
            return Ok(boundary);
        }
        if state.mode == MutationMode::Complete {
            return Ok(Boundary::Complete);
        }
    }
}
