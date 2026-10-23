//! `xtask validation-campaign`: the deterministic, resumable Core Validation campaign. The
//! campaign (not the LLM) owns dependencies, provenance, pass/fail state, retries and source
//! invalidation; the LLM follows `status --json` and runs `continue`.
pub mod backend;
pub mod bootstrap;
pub mod calibration_task;
pub mod cli;
pub mod corpus;
pub mod evidence;
pub mod executor;
pub mod frame_score;
pub mod graph;
pub mod host_factory;
pub mod integration;
pub mod kernels;
pub mod manifest;
pub mod model;
pub mod overhead_real;
pub mod policy;
pub mod pr_task;
pub mod repo_pr;
pub mod repository;
pub mod runners;
pub mod scenarios;
pub mod shell;
pub mod state;
pub mod status;
pub mod system_host;
pub mod verify;
pub mod workloads;

#[cfg(test)]
mod tests;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail, ensure};
use cli::{CampaignArgs, CampaignCommand, WorkloadCommand};
use executor::Executor;
use graph::Graph;
use manifest::CampaignManifest;
use model::{Boundary, CampaignMode, CampaignState};
use repository::{Binaries, file_sha256, git};
use state::{CampaignDir, InitInputs, load, save, unix_ms, write_atomic};

const MANIFEST_PATH_KEY: &str = "manifest.path";

/// Process exit code per boundary, so scripts can branch without parsing.
pub fn exit_code(boundary: Boundary) -> i32 {
    match boundary {
        Boundary::Complete | Boundary::Idle => 0,
        Boundary::WaitingForReboot => 10,
        Boundary::BlockedResource => 11,
        Boundary::BlockedPrivilege => 12,
        Boundary::FailureRequiresSourceChange => 13,
    }
}

/// The campaign directory for `--campaign`, or the single non-terminal campaign of `root`.
fn locate(root: &Path, explicit: Option<PathBuf>) -> anyhow::Result<CampaignDir> {
    if let Some(path) = explicit {
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        // Accept either the campaign directory or its campaign.json.
        let directory = if path.is_file() {
            path.parent()
                .map(Path::to_path_buf)
                .context("campaign path has no parent")?
        } else {
            path
        };
        return Ok(CampaignDir(directory));
    }
    let base = root.join(".llm-runs/validation");
    let mut candidates = Vec::new();
    if base.is_dir() {
        for entry in fs::read_dir(&base)? {
            let path = entry?.path();
            if path.join("campaign.json").is_file() {
                candidates.push(path);
            }
        }
    }
    candidates.sort();
    match candidates.len() {
        0 => bail!(
            "no campaign found under {}; run `validation-campaign init`",
            base.display()
        ),
        1 => Ok(CampaignDir(candidates.remove(0))),
        _ => {
            // Prefer the one that is still active.
            let active = candidates
                .iter()
                .filter(|path| {
                    load(&CampaignDir((*path).clone())).is_ok_and(|s| {
                        !matches!(s.mode, CampaignMode::Invalidated | CampaignMode::Complete)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            ensure!(
                active.len() == 1,
                "{} campaigns found; pass --campaign",
                candidates.len()
            );
            Ok(CampaignDir(active[0].clone()))
        }
    }
}

pub(super) struct Loaded {
    dir: CampaignDir,
    state: CampaignState,
    manifest: CampaignManifest,
    graph: Graph,
}

pub(super) fn load_campaign(root: &Path, explicit: Option<PathBuf>) -> anyhow::Result<Loaded> {
    let dir = locate(root, explicit)?;
    let state = load(&dir)?;
    let bytes = fs::read(dir.manifest_snapshot()).context("read manifest snapshot")?;
    ensure!(
        crate::evidence_bundle::sha256::sha256_hex(&bytes) == state.manifest_sha256,
        "manifest snapshot does not match the campaign's recorded digest"
    );
    let manifest = CampaignManifest::parse(&String::from_utf8(bytes)?)?;
    Ok(Loaded {
        dir,
        state,
        manifest,
        graph: Graph::core()?,
    })
}

fn build_executor(loaded: &Loaded, root: &Path) -> anyhow::Result<Executor> {
    let manifest_rel = loaded
        .state
        .frozen
        .get(MANIFEST_PATH_KEY)
        .cloned()
        .unwrap_or_else(|| "docs/validation/core-validation.toml".to_owned());
    let binaries = Binaries::for_repository(root)?;
    Ok(Executor {
        graph: Graph::core()?,
        manifest: loaded.manifest.clone(),
        runners: runners::default_runners(),
        repository: root.to_path_buf(),
        binaries: binaries.clone(),
        source_check: Box::new(|_, _| Ok(())),
    }
    .with_repository_check(root.join(manifest_rel)))
}

/// Runs a registered workload to completion; the privileged runner launches this by name.
fn run_workload(root: &Path, workload: &WorkloadCommand) -> anyhow::Result<()> {
    match workload {
        WorkloadCommand::FixedCompile { cycles, target_dir } => {
            workloads::fixed_compile::run_cycles(
                &workloads::SystemTools,
                root,
                target_dir,
                *cycles,
                &std::time::Instant::now,
            )
            .map(|_| ())
            .map_err(anyhow::Error::msg)
        }
    }
}

pub fn run(root: &Path, args: CampaignArgs) -> anyhow::Result<()> {
    match args.command {
        CampaignCommand::Init { manifest } => init(root, &manifest),
        CampaignCommand::Bootstrap { campaign, dry_run } => {
            bootstrap_command(root, campaign, dry_run)
        }
        CampaignCommand::Status { json, campaign } => {
            let loaded = load_campaign(root, campaign)?;
            let report = status::build(&loaded.state, &loaded.graph, &loaded.manifest);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", status::render_human(&report));
            }
            Ok(())
        }
        CampaignCommand::Next { campaign } => {
            let loaded = load_campaign(root, campaign)?;
            let next = loaded.state.ready(&loaded.graph);
            match next.first() {
                Some(task) => println!("{task}"),
                None => println!("none"),
            }
            Ok(())
        }
        CampaignCommand::Continue {
            campaign,
            max_tasks,
            resume,
        } => {
            let mut loaded = load_campaign(root, campaign)?;
            loaded.state.require_mode(&[
                CampaignMode::Collector,
                CampaignMode::ReleaseCandidate,
                CampaignMode::Development,
            ])?;
            if resume {
                println!("resuming campaign {}", loaded.state.campaign_id);
            }
            let executor = build_executor(&loaded, root)?;
            let report = executor.continue_campaign(&loaded.dir, &mut loaded.state, max_tasks)?;
            println!("ran: {}", report.ran.join(", "));
            println!("boundary: {:?}", report.boundary);
            let code = exit_code(report.boundary);
            if code != 0 {
                std::process::exit(code);
            }
            Ok(())
        }
        CampaignCommand::Verify { campaign, strict } => {
            let loaded = load_campaign(root, campaign)?;
            let problems =
                verify::verify_state(&loaded.dir, &loaded.state, &loaded.graph, &loaded.manifest);
            for problem in &problems {
                println!("VERIFY FAIL: {problem}");
            }
            if problems.is_empty() {
                println!(
                    "VERIFY PASS: campaign {} verifies",
                    loaded.state.campaign_id
                );
            }
            ensure!(!strict || problems.is_empty(), "strict verification failed");
            Ok(())
        }
        CampaignCommand::Workload { workload } => run_workload(root, &workload),
        CampaignCommand::Report { campaign } => report(root, campaign),
        CampaignCommand::Cancel { campaign } => {
            let mut loaded = load_campaign(root, campaign)?;
            loaded.state.mode = CampaignMode::Invalidated;
            loaded.state.decisions.push(model::CampaignDecision {
                task: "campaign".to_owned(),
                decision: "cancelled by the operator; evidence retained".to_owned(),
                limitations: Vec::new(),
                unix_ms: unix_ms(),
            });
            save(&loaded.dir, &loaded.state)?;
            if let Some(message) = kernels::cancel_runner(&loaded.state) {
                println!("{message}");
            }
            println!(
                "campaign {} cancelled; no evidence was deleted",
                loaded.state.campaign_id
            );
            Ok(())
        }
    }
}

pub(super) fn init(root: &Path, manifest_path: &Path) -> anyhow::Result<()> {
    init_with(root, manifest_path, &|binaries, commit| {
        repository::check_binary_revision(&binaries.stutter, commit)
    })
}

/// `init` with the binary-revision check injected so tests can exercise the success path.
pub(super) fn init_with(
    root: &Path,
    manifest_path: &Path,
    binary_check: &dyn Fn(&Binaries, &str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let manifest_path = if manifest_path.is_absolute() {
        manifest_path.to_path_buf()
    } else {
        root.join(manifest_path)
    };
    let (manifest, manifest_sha) = CampaignManifest::load(&manifest_path)?;
    let manifest_rel = manifest_path
        .strip_prefix(root)
        .context("the manifest must live inside the repository")?
        .to_string_lossy()
        .into_owned();
    ensure!(
        git(root, &["status", "--porcelain"])?.is_empty(),
        "refusing to initialize: the working tree is dirty"
    );
    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    ensure!(
        branch == "main",
        "refusing to initialize from {branch:?}: a campaign starts from a clean `main`"
    );
    let commit = git(root, &["rev-parse", "HEAD"])?;
    let binaries = Binaries::for_repository(root)?;
    let graph = Graph::core()?;
    graph.check_skip_policy(&manifest.policy.skip_tasks)?;
    ensure!(
        binaries.stutter.is_file(),
        "{} is missing; build with `cargo build -p stutter -p xtask` first",
        binaries.stutter.display()
    );
    binary_check(&binaries, &commit)?;
    // One active collector per repository.
    let base = root.join(".llm-runs/validation");
    if base.is_dir() {
        for entry in fs::read_dir(&base)? {
            let dir = CampaignDir(entry?.path());
            if let Ok(existing) = load(&dir)
                && matches!(
                    existing.mode,
                    CampaignMode::Development
                        | CampaignMode::Collector
                        | CampaignMode::ReleaseCandidate
                )
            {
                bail!(
                    "campaign {} is still active for this repository; cancel or complete it first",
                    existing.campaign_id
                );
            }
        }
    }
    let short = &commit[..commit.len().min(7)];
    let campaign_id = format!("core-{short}-{}", unix_ms());
    let dir = CampaignDir::under(root, &campaign_id);
    let (uid, gid) = process_ids();
    let mut state = CampaignState::init(
        InitInputs {
            campaign_id: campaign_id.clone(),
            repository: fs::canonicalize(root)?,
            evidence_base: commit,
            manifest_sha256: manifest_sha.clone(),
            cargo_lock_sha256: file_sha256(&root.join("Cargo.lock"))?,
            xtask_sha256: repository::cached_file_sha256(&binaries.xtask)?,
            stutter_sha256: repository::cached_file_sha256(&binaries.stutter)?,
            uid,
            gid,
        },
        &graph,
        &manifest,
    );
    state
        .frozen
        .insert(MANIFEST_PATH_KEY.to_owned(), manifest_rel);
    write_atomic(&dir.manifest_snapshot(), &fs::read(&manifest_path)?)?;
    save(&dir, &state)?;
    println!("campaign_id={campaign_id}");
    println!("state={}", dir.state_path().display());
    Ok(())
}

/// uid/gid of the invoking process: the owner of /proc/self (no unsafe code needed).
fn process_ids() -> (u32, u32) {
    use std::os::unix::fs::MetadataExt;
    fs::metadata("/proc/self").map_or((0, 0), |meta| (meta.uid(), meta.gid()))
}

/// Installs (or confirms) the privileged runner; `BLOCKED_PRIVILEGE_BOOTSTRAP` exits with the
/// privilege boundary code and is a report, not a failure to hide.
fn bootstrap_command(root: &Path, campaign: Option<PathBuf>, dry_run: bool) -> anyhow::Result<()> {
    let loaded = load_campaign(root, campaign)?;
    let socket = std::env::var_os(stutter_validation_runner::client::SOCKET_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(|| PathBuf::from(bootstrap::DEFAULT_SOCKET), PathBuf::from);
    let outcome = bootstrap::bootstrap(
        &loaded.dir,
        &loaded.state,
        &loaded.manifest,
        &socket,
        dry_run,
    )?;
    if outcome == bootstrap::BootstrapOutcome::BlockedPrivilege {
        std::process::exit(exit_code(Boundary::BlockedPrivilege));
    }
    Ok(())
}

fn report(root: &Path, campaign: Option<PathBuf>) -> anyhow::Result<()> {
    let loaded = load_campaign(root, campaign)?;
    let status = status::build(&loaded.state, &loaded.graph, &loaded.manifest);
    let json_path = loaded.dir.0.join("report.json");
    write_atomic(&json_path, &serde_json::to_vec_pretty(&status)?)?;
    let mut markdown = format!(
        "# Campaign {}\n\nmode: {:?}\noverall: {}\nevidence base: `{}`\n\n| task | status | attempts |\n| --- | --- | --- |\n",
        status.campaign_id, status.mode, status.overall, status.evidence_base
    );
    for id in loaded.graph.topological() {
        if let (Some(spec), Some(task)) = (loaded.graph.get(id), loaded.state.tasks.get(id)) {
            markdown.push_str(&format!(
                "| {id} | {:?} | {} |\n",
                loaded.state.effective_status(spec),
                task.attempts.len()
            ));
        }
    }
    let markdown_path = loaded.dir.0.join("report.md");
    write_atomic(&markdown_path, markdown.as_bytes())?;
    println!("report_json={}", json_path.display());
    println!("report_markdown={}", markdown_path.display());
    Ok(())
}
