//! Durable service-campaign controller. Every attempt is retained with exact source/binary pins.
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use anyhow::{Context, ensure};
use stutter_validation_runner::model::{BoundedServiceAction, Request};

use super::{
    manifest::{self, Manifest},
    model::{Mode, Stage, State},
    runner, store,
};
use crate::validation_campaign::{
    bootstrap::{self, BootstrapOutcome},
    model::{AttemptOutcome, FailureClass, TaskAttempt, TaskFingerprint, TaskState, TaskStatus},
    repository::{Binaries, check_binary_revision, file_sha256, git},
    state::{CampaignDir, append_task_attempt, unix_ms, write_atomic},
};

#[derive(clap::Args, Clone, Debug, Eq, PartialEq)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(clap::Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Init,
    Status {
        #[arg(long)]
        campaign: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Continue {
        #[arg(long)]
        campaign: Option<PathBuf>,
        #[arg(long)]
        max_stages: Option<usize>,
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

pub fn init(root: &Path) -> anyhow::Result<()> {
    ensure!(
        git(root, &["status", "--porcelain"])?.is_empty(),
        "service campaign requires clean committed source"
    );
    let source = git(root, &["rev-parse", "HEAD"])?;
    let binaries = Binaries::for_repository(root)?;
    check_binary_revision(&binaries.stutter, &source)?;
    let manifest = Manifest::load(&root.join(manifest::FILE))?;
    // Only a person measuring the schedule sets this; the campaign it makes can never be evidence.
    let rehearsal = std::env::var_os("STUTTER_SERVICE_REHEARSAL").is_some();
    let identity = fs::metadata("/proc/self")?;
    ensure!(
        identity.uid() != 0,
        "service campaign controller runs as the monitored desktop user"
    );
    let campaign_id = format!("service-{}-{}", &source[..7], unix_ms());
    let dir = CampaignDir(root.join(store::ROOT).join(&campaign_id));
    fs::create_dir_all(&dir.0)?;
    fs::copy(root.join(manifest::FILE), dir.manifest_snapshot())?;
    let state = State {
        schema_version: 1,
        campaign_id,
        mode: Mode::Active,
        repository: root.to_owned(),
        source_commit: source,
        manifest_sha256: file_sha256(&root.join(manifest::FILE))?,
        cargo_lock_sha256: file_sha256(&root.join("Cargo.lock"))?,
        stutter_sha256: file_sha256(&binaries.stutter)?,
        xtask_sha256: file_sha256(&binaries.xtask)?,
        uid: identity.uid(),
        gid: identity.gid(),
        created_unix_ms: unix_ms(),
        kernel_release: fs::read_to_string("/proc/sys/kernel/osrelease")?
            .trim()
            .to_owned(),
        host_system: runner::system_closure("/run/current-system")?,
        rehearsal,
        required_reboots: if rehearsal {
            0
        } else {
            manifest.coverage.minimum_real_reboots
        },
        boot_ids: vec![stutter_core::boot::current_boot_id()?],
        reboot_pending: false,
        tasks: Stage::ALL
            .into_iter()
            .map(|stage| (stage.id().to_owned(), TaskState::new(stage.id())))
            .collect(),
    };
    ensure!(
        runner::target(&state).max_reboots == manifest.coverage.minimum_real_reboots,
        "root reboot budget and manifest disagree"
    );
    snapshot_binaries(&state, &binaries)?;
    store::save(&dir, &state)?;
    println!(
        "campaign={}\ncampaign_id={}",
        dir.0.display(),
        state.campaign_id
    );
    Ok(())
}

pub fn fingerprint(state: &State, stage: Stage) -> TaskFingerprint {
    TaskFingerprint {
        task: stage.id().to_owned(),
        evidence_commit: state.source_commit.clone(),
        manifest_sha256: state.manifest_sha256.clone(),
        stutter_sha256: Some(state.stutter_sha256.clone()),
        xtask_sha256: Some(state.xtask_sha256.clone()),
        workload_sha256: None,
        scenario_sha256: None,
        kernel_release: Some(state.kernel_release.clone()),
    }
}

pub trait StageRunner {
    fn execute(
        &self,
        stage: Stage,
        dir: &CampaignDir,
        state: &mut State,
        attempt: u32,
    ) -> anyhow::Result<AttemptOutcome>;
    /// Runs after the attempt and the campaign state are durably persisted and only when the
    /// attempt is waiting for a reboot. This is where the reboot itself is requested: a reboot
    /// inside `execute` would erase the machine before anything was recorded.
    fn after_persist(
        &self,
        stage: Stage,
        state: &State,
        outcome: &AttemptOutcome,
    ) -> anyhow::Result<()> {
        let _ = (stage, state, outcome);
        Ok(())
    }
}

pub fn drive(
    root: &Path,
    dir: &CampaignDir,
    stages: &dyn StageRunner,
    max_stages: Option<usize>,
) -> anyhow::Result<()> {
    drive_with(dir, stages, max_stages, &|state| {
        runner::integrity(root, state)
    })
}

pub fn drive_with(
    dir: &CampaignDir,
    stages: &dyn StageRunner,
    max_stages: Option<usize>,
    integrity: &dyn Fn(&State) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut state = store::load(dir)?;
    ensure!(
        state.mode == Mode::Active,
        "service campaign is {:?}",
        state.mode
    );
    if let Err(error) = integrity(&state) {
        state.mode = Mode::Invalidated;
        store::save(dir, &state)?;
        return Err(error);
    }
    for _ in 0..max_stages.unwrap_or(Stage::ALL.len()) {
        let Some(stage) = Stage::ALL
            .into_iter()
            .find(|stage| state.tasks[stage.id()].status != TaskStatus::Passed)
        else {
            state.mode = Mode::Qualified;
            store::save(dir, &state)?;
            return Ok(());
        };
        let previous = state.tasks[stage.id()].attempts.last();
        if let Some(previous) = previous {
            if matches!(
                previous.outcome,
                AttemptOutcome::Failed {
                    class: FailureClass::SoftwareDefect | FailureClass::ValidationFailure,
                    ..
                }
            ) {
                anyhow::bail!(
                    "service stage {} needs a source fix or truthful disposition; its failure cannot be retried into a pass",
                    stage.id()
                );
            }
            ensure!(
                state.tasks[stage.id()].failed_attempts() < 3,
                "service stage retry budget exhausted"
            );
        }
        if state.tasks[stage.id()].status == TaskStatus::Running {
            let number = state.tasks[stage.id()].attempts.len() as u32 + 1;
            let interrupted = TaskAttempt { number, started_unix_ms: unix_ms(), finished_unix_ms: unix_ms(),
                outcome: AttemptOutcome::Failed { class: FailureClass::Retryable, stage: stage.id().to_owned(), summary: "controller interrupted before the attempt completed; partial artifacts retained".to_owned() },
                fingerprint: fingerprint(&state, stage), artifacts: Some(dir.relative(&dir.attempt_dir(stage.id(), number))), receipt: None,
                limitations: vec!["timestamps record interruption recovery, not the unknown interrupted start".to_owned()] };
            append_task_attempt(
                dir,
                state.tasks.get_mut(stage.id()).context("stage missing")?,
                interrupted,
            )?;
            store::save(dir, &state)?;
        }
        let number = state.tasks[stage.id()].attempts.len() as u32 + 1;
        state
            .tasks
            .get_mut(stage.id())
            .context("stage missing")?
            .status = TaskStatus::Running;
        store::save(dir, &state)?;
        let started = unix_ms();
        let outcome = match stages.execute(stage, dir, &mut state, number) {
            Ok(outcome) => outcome,
            Err(error) => AttemptOutcome::Failed {
                class: error
                    .downcast_ref::<StageFailure>()
                    .map_or(FailureClass::SoftwareDefect, |failure| failure.class),
                stage: stage.id().to_owned(),
                summary: format!("{error:#}"),
            },
        };
        let attempt = TaskAttempt {
            number,
            started_unix_ms: started,
            finished_unix_ms: unix_ms(),
            outcome: outcome.clone(),
            fingerprint: fingerprint(&state, stage),
            artifacts: Some(dir.relative(&dir.attempt_dir(stage.id(), number))),
            receipt: None,
            limitations: vec!["systemd-only; OpenRC remains unqualified".to_owned()],
        };
        append_task_attempt(
            dir,
            state.tasks.get_mut(stage.id()).context("stage missing")?,
            attempt,
        )?;
        store::save(dir, &state)?;
        if outcome == AttemptOutcome::WaitingForReboot {
            stages.after_persist(stage, &state, &outcome)?;
            return Ok(());
        }
        if outcome != AttemptOutcome::Passed {
            return Ok(());
        }
        if let Err(error) = integrity(&state) {
            state.mode = Mode::Invalidated;
            store::save(dir, &state)?;
            return Err(error);
        }
    }
    if Stage::ALL
        .into_iter()
        .all(|stage| state.tasks[stage.id()].status == TaskStatus::Passed)
    {
        state.mode = Mode::Qualified;
        store::save(dir, &state)?;
    }
    Ok(())
}

pub fn verify(dir: &CampaignDir, state: &State, strict: bool) -> anyhow::Result<serde_json::Value> {
    let mut failures = Vec::new();
    for stage in Stage::ALL {
        let task = state
            .tasks
            .get(stage.id())
            .with_context(|| format!("missing service stage {}", stage.id()))?;
        if task.status != TaskStatus::Passed
            || task
                .attempts
                .last()
                .is_none_or(|attempt| attempt.outcome != AttemptOutcome::Passed)
        {
            failures.push(format!("{} has no PASS evidence", stage.id()));
        }
        for (index, attempt) in task.attempts.iter().enumerate() {
            ensure!(
                attempt.number as usize == index + 1,
                "service attempt sequence is corrupt"
            );
            let retained: TaskAttempt = serde_json::from_slice(&fs::read(
                dir.attempt_dir(stage.id(), attempt.number)
                    .join("attempt.json"),
            )?)?;
            ensure!(
                retained == *attempt && attempt.fingerprint == fingerprint(state, stage),
                "service retained attempt/source mismatch"
            );
        }
    }
    let report = serde_json::json!({"campaign": state.campaign_id, "source_commit": state.source_commit, "passed": failures.is_empty(), "failures": failures, "mode": state.mode});
    if strict {
        ensure!(
            report["passed"] == true && state.mode == Mode::Qualified,
            "service campaign is not qualified: {report}"
        );
    }
    Ok(report)
}

pub fn run(root: &Path, args: Args) -> anyhow::Result<()> {
    match args.command {
        Command::Init => init(root),
        Command::Status { campaign, json: _ } | Command::Report { campaign } => {
            let dir = store::locate(root, campaign)?;
            let state = store::load(&dir)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&verify(&dir, &state, false)?)?
            );
            Ok(())
        }
        Command::Verify { campaign, strict } => {
            let dir = store::locate(root, campaign)?;
            println!("{}", verify(&dir, &store::load(&dir)?, strict)?);
            Ok(())
        }
        Command::Cancel { campaign } => {
            let dir = store::locate(root, campaign)?;
            let mut state = store::load(&dir)?;
            cancel(&mut state);
            store::save(&dir, &state)
        }
        Command::Continue {
            campaign,
            max_stages,
            resume: _,
        } => {
            let dir = store::locate(root, campaign)?;
            let mut state = store::load(&dir)?;
            if let Err(error) = runner::integrity(root, &state) {
                state.mode = Mode::Invalidated;
                store::save(&dir, &state)?;
                return Err(error);
            }
            // Every stage runs from the campaign's own xtask, so the binary that qualifies is the one
            // that was pinned and not whatever Cargo last left in `target/debug`.
            let pinned = state.pinned_xtask();
            if std::env::current_exe()?.canonicalize()? != pinned.canonicalize()? {
                use std::os::unix::process::CommandExt;
                let error = std::process::Command::new(&pinned)
                    .args(std::env::args_os().skip(1))
                    .exec();
                anyhow::bail!(
                    "could not hand over to the pinned xtask {}: {error}",
                    pinned.display()
                );
            }
            let snapshot = fs::read(dir.manifest_snapshot())?;
            ensure!(
                stutter_core::hash::sha256_hex(&snapshot) == state.manifest_sha256,
                "retained manifest changed"
            );
            if bootstrap::bootstrap_target(&dir, &runner::target(&state), &runner::socket(), false)?
                == BootstrapOutcome::BlockedPrivilege
            {
                anyhow::bail!("service campaign privilege bootstrap is unavailable");
            }
            drive(root, &dir, &super::harness::Production, max_stages)
        }
    }
}

pub fn write_report(
    dir: &CampaignDir,
    stage: Stage,
    attempt: u32,
    report: &serde_json::Value,
) -> anyhow::Result<()> {
    write_atomic(
        &dir.attempt_dir(stage.id(), attempt).join("result.json"),
        &serde_json::to_vec_pretty(report)?,
    )
}

/// A live stage error that knows what kind of failure it is. Any other error is an invariant
/// break in the product or harness and so a software defect.
#[derive(Debug)]
pub(crate) struct StageFailure {
    pub class: FailureClass,
    pub detail: String,
}

impl std::fmt::Display for StageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for StageFailure {}

impl StageFailure {
    pub fn error(class: FailureClass, detail: impl Into<String>) -> anyhow::Error {
        anyhow::Error::new(Self {
            class,
            detail: detail.into(),
        })
    }
}

/// Copies the exact validated executables into the campaign, atomically, read-only and fsynced,
/// then proves the copies hash to the recorded digests.
pub(super) fn snapshot_binaries(state: &State, binaries: &Binaries) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = state.pinned_dir();
    fs::create_dir_all(&dir)?;
    for (source, target, expected) in [
        (
            &binaries.stutter,
            state.pinned_stutter(),
            &state.stutter_sha256,
        ),
        (&binaries.xtask, state.pinned_xtask(), &state.xtask_sha256),
    ] {
        let temporary = target.with_extension("partial");
        let _ = fs::remove_file(&temporary);
        fs::copy(source, &temporary)
            .with_context(|| format!("snapshot {} into the campaign", source.display()))?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o555))?;
        fs::File::open(&temporary)?.sync_all()?;
        fs::rename(&temporary, &target)?;
        ensure!(
            file_sha256(&target)? == *expected,
            "the campaign snapshot {} does not hash to the validated binary",
            target.display()
        );
    }
    fs::File::open(&dir)?.sync_all()?;
    Ok(())
}

/// Removes the campaign's service artifacts (the runner refuses everything but status once it is
/// cancelled, so this comes first), then disarms the root runner so no reboot can resume it.
/// A cleanup that fails, such as volatile units a reboot already removed, is reported but can never
/// keep the resume armed.
fn cancel(state: &mut State) {
    if state.tasks[Stage::Install.id()].status == TaskStatus::Passed {
        let uninstall = runner::request(
            state,
            Stage::Install,
            Request::BoundedService {
                action: BoundedServiceAction::UninstallPinned,
            },
        );
        if let Err(error) = uninstall {
            eprintln!("service artifacts left for inspection: {error:#}");
        }
    }
    match runner::request_campaign(state, Request::CancelCampaign) {
        Ok(_) => eprintln!("root runner: campaign cancelled, resume disarmed"),
        Err(error) => eprintln!("root runner cancel not applied: {error:#}"),
    }
    state.mode = Mode::Cancelled;
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn a_typed_stage_failure_keeps_its_class_through_anyhow() {
        let error = StageFailure::error(FailureClass::Retryable, "foreign build");
        let class = error
            .downcast_ref::<StageFailure>()
            .map_or(FailureClass::SoftwareDefect, |failure| failure.class);
        assert_eq!(class, FailureClass::Retryable);
        let plain = anyhow::anyhow!("invariant broke");
        assert!(plain.downcast_ref::<StageFailure>().is_none());
    }
}
