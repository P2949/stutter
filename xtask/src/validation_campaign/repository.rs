//! Source integrity and repository actions. Reuses the live-acceptance provenance rules in
//! spirit (clean tree, binary revision == source, linked harness == binary); the harness
//! itself still enforces them again inside every suite.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, ensure};

use super::{
    graph::{Graph, SourceBinding, TaskSpec, issue_gates},
    model::{CampaignMode, CampaignState, RepositoryAction},
};
use crate::evidence_bundle::sha256::sha256_hex;

pub fn git(repository: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository)
        .output()
        .with_context(|| format!("run git {args:?}"))?;
    ensure!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub fn file_sha256(path: &Path) -> anyhow::Result<String> {
    Ok(sha256_hex(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    ))
}

/// Binaries whose identity a campaign pins.
#[derive(Clone, Debug)]
pub struct Binaries {
    pub stutter: PathBuf,
    pub xtask: PathBuf,
}

impl Binaries {
    pub fn for_repository(repository: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            stutter: repository.join("target/debug/stutter"),
            xtask: std::env::current_exe().context("locate the running xtask")?,
        })
    }
}

/// `stutter --version` must report the source commit, and the xtask's linked library the same
/// build as the CLI, exactly as every live suite requires.
pub fn check_binary_revision(stutter: &Path, commit: &str) -> anyhow::Result<()> {
    let output = Command::new(stutter).arg("--version").output()?;
    ensure!(output.status.success(), "stutter --version failed");
    let version = String::from_utf8(output.stdout)?.trim().to_owned();
    let short = &commit[..commit.len().min(7)];
    ensure!(
        version.contains(&format!("git {short})")) || version.contains(&format!("git {commit})")),
        "stutter binary reports {version:?} but the campaign source is {commit}"
    );
    ensure!(
        version == format!("stutter {}", stutter::api::build_version()),
        "xtask is linked against a different stutter build than the stutter binary"
    );
    Ok(())
}

/// The commit a task must run at, or `None` when the task is not tied to a frozen commit.
fn required_commit<'a>(state: &'a CampaignState, spec: &TaskSpec) -> Option<&'a str> {
    match spec.binding {
        SourceBinding::None => None,
        SourceBinding::EvidenceBase => Some(&state.evidence_base),
        SourceBinding::ReleaseCandidate => state.release_candidate.as_deref(),
    }
}

/// Source-integrity check before a source-bound task. `Err` carries the reason; the caller
/// invalidates the campaign. Unbound tasks (integration, policy, ...) are allowed to run on a
/// moved `main`.
pub fn check_source(
    state: &CampaignState,
    spec: &TaskSpec,
    manifest_path: &Path,
    binaries: &Binaries,
) -> Result<(), String> {
    let Some(commit) = required_commit(state, spec) else {
        return if spec.binding == SourceBinding::ReleaseCandidate {
            Err("no release candidate has been frozen".to_owned())
        } else {
            Ok(())
        };
    };
    let check = || -> anyhow::Result<()> {
        let head = git(&state.repository, &["rev-parse", "HEAD"])?;
        ensure!(head == commit, "HEAD is {head}, the task requires {commit}");
        ensure!(
            git(&state.repository, &["status", "--porcelain"])?.is_empty(),
            "working tree is dirty"
        );
        ensure!(
            file_sha256(manifest_path)? == state.manifest_sha256,
            "campaign manifest changed"
        );
        ensure!(
            file_sha256(&state.repository.join("Cargo.lock"))? == state.cargo_lock_sha256,
            "Cargo.lock changed"
        );
        ensure!(
            cached_file_sha256(&binaries.stutter)? == state.stutter_sha256,
            "stutter binary changed"
        );
        ensure!(
            cached_file_sha256(&binaries.xtask)? == state.xtask_sha256,
            "xtask binary changed"
        );
        check_binary_revision(&binaries.stutter, commit)
    };
    check().map_err(|error| format!("{error:#}"))
}

/// Repository actions the agent should perform with its own authenticated `gh`: no API
/// credentials live in the campaign. An issue is ready only when every gate task truly passed.
pub fn repository_actions(state: &CampaignState, graph: &Graph) -> Vec<RepositoryAction> {
    let passed = |id: &str| {
        graph
            .get(id)
            .is_some_and(|spec| state.effective_status(spec).satisfies_dependents())
    };
    let mut actions = issue_gates()
        .into_iter()
        .map(|gate| {
            let missing = gate
                .tasks
                .iter()
                .filter(|task| !passed(task))
                .cloned()
                .collect::<Vec<_>>();
            let ready = missing.is_empty();
            RepositoryAction {
                kind: "close_issue".to_owned(),
                issue: Some(gate.issue),
                ready,
                evidence_summary: if ready {
                    format!(
                        "{}: all gate tasks passed ({})",
                        gate.title,
                        gate.tasks.join(", ")
                    )
                } else {
                    format!("{}: waiting on {}", gate.title, missing.join(", "))
                },
            }
        })
        .collect::<Vec<_>>();
    actions.push(RepositoryAction {
        kind: "push_tag".to_owned(),
        issue: None,
        ready: state.mode == CampaignMode::ReleaseCandidate && passed("tag"),
        evidence_summary: "the tag task passed; push the annotated tag at the release candidate"
            .to_owned(),
    });
    actions
}

/// Identity of a file as the filesystem reports it; any rewrite changes at least one field.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FileStamp {
    len: u64,
    modified_ns: i128,
    inode: u64,
}

fn stamp(path: &Path) -> anyhow::Result<FileStamp> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    Ok(FileStamp {
        len: meta.len(),
        modified_ns: i128::from(meta.mtime()) * 1_000_000_000 + i128::from(meta.mtime_nsec()),
        inode: meta.ino(),
    })
}

type HashCache = std::sync::Mutex<std::collections::HashMap<PathBuf, (FileStamp, String)>>;

/// SHA-256 of a large binary, recomputed only when its size, mtime or inode changes. The
/// campaign re-checks the same 300 MB binaries before every source-bound task.
pub fn cached_file_sha256(path: &Path) -> anyhow::Result<String> {
    static CACHE: std::sync::OnceLock<HashCache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let current = stamp(path)?;
    if let Ok(guard) = cache.lock()
        && let Some((known, digest)) = guard.get(path)
        && *known == current
    {
        return Ok(digest.clone());
    }
    let digest = file_sha256(path)?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(path.to_path_buf(), (current, digest.clone()));
    }
    Ok(digest)
}
