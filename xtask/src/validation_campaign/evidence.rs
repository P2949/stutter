//! Atomic evidence publication. A live-suite result is not task evidence until its portable
//! bundle verifies: raw attempt -> export into a partial directory -> `evidence-verify` ->
//! derived index entry -> fsync -> atomic rename into `bundles/` -> receipt. A failure keeps
//! the raw attempt plus a `failure.json`; nothing is deleted.
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::state::{CampaignDir, unix_ms, write_atomic};
use crate::evidence_bundle::{export_bundle, verified_index_entry};

/// Why publication stopped; `stage` names the pipeline step.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PipelineFailure {
    pub stage: String,
    pub summary: String,
}

impl PipelineFailure {
    fn at(stage: &str, error: impl std::fmt::Display) -> Self {
        Self {
            stage: stage.to_owned(),
            summary: format!("{error:#}"),
        }
    }
}

/// A verified, published bundle.
#[derive(Clone, Debug)]
pub struct Published {
    pub capture_id: String,
    pub bundle: PathBuf,
    pub receipt: PathBuf,
    pub index_entry: Value,
}

fn sync_tree(path: &Path) -> anyhow::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            sync_tree(&entry?.path())?;
        }
    }
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

/// Capture ids already published for any task in this campaign.
pub fn published_capture_ids(dir: &CampaignDir) -> anyhow::Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();
    let root = dir.bundles();
    if !root.is_dir() {
        return Ok(ids);
    }
    for task in fs::read_dir(&root)? {
        let task = task?.path();
        if !task.is_dir() {
            continue;
        }
        for bundle in fs::read_dir(&task)? {
            let name = bundle?.file_name().to_string_lossy().into_owned();
            // Partial directories are dot-prefixed and never count as published.
            if !name.starts_with('.') {
                ids.insert(name);
            }
        }
    }
    Ok(ids)
}

/// Which suite results may be published as evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Accept {
    PassOnly,
    /// Kernel compatibility is *observed*: a kernel the loader cannot use yields a FAIL result
    /// whose `compatibility` is `ObservedIncompatible`. That is a valid observation to keep.
    PassOrObservedIncompatible,
    /// A score series that ran fine but is not monotonic is `INCONCLUSIVE` at the `evaluation`
    /// stage. That is an empirical observation (the weights are retained and the limitation
    /// recorded), not a failed attempt.
    PassOrNonMonotonicScore,
}

impl Accept {
    pub fn allows(self, value: &Value) -> bool {
        value["status"] == "PASS"
            || (self == Self::PassOrObservedIncompatible
                && value["status"] == "FAIL"
                && value["compatibility"] == "ObservedIncompatible")
            || (self == Self::PassOrNonMonotonicScore
                && value["status"] == "INCONCLUSIVE"
                // Raw results carry the stage; index entries derived from them do not.
                && (value["stage"] == "evaluation" || value["stage"].is_null()))
    }
}

/// Runs the pipeline for a finished raw attempt. `expected_commit` is the commit the bundle
/// must certify; `allowed_kinds` limits acceptable evidence kinds.
/// Inputs of [`publish_bundle`].
pub struct PublishRequest<'a> {
    pub dir: &'a CampaignDir,
    pub repository: &'a Path,
    pub task: &'a str,
    pub attempt: u32,
    pub raw: &'a Path,
    pub expected_commit: &'a str,
    pub allowed_kinds: &'a [&'a str],
    pub accept: Accept,
}

pub fn publish_bundle(request: &PublishRequest<'_>) -> Result<Published, PipelineFailure> {
    let PublishRequest {
        dir,
        repository,
        task,
        attempt,
        raw,
        expected_commit,
        allowed_kinds,
        accept,
    } = *request;
    let result: Value = fs::read(raw.join("result.json"))
        .map_err(|error| PipelineFailure::at("suite_result", error))
        .and_then(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|error| PipelineFailure::at("suite_result", error))
        })?;
    if !accept.allows(&result) {
        return Err(PipelineFailure::at(
            "suite_result",
            format!("suite reported {}", result["status"]),
        ));
    }
    let task_dir = dir.bundles().join(task);
    let partial = task_dir.join(format!(".partial-{attempt:04}"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&task_dir).map_err(|error| PipelineFailure::at("export", error))?;
    export_bundle(repository, raw, &partial)
        .map_err(|error| PipelineFailure::at("export", error))?;
    let entry = verified_index_entry(&partial)
        .map_err(|error| PipelineFailure::at("evidence_verify", error))?;
    if !accept.allows(&entry) {
        return Err(PipelineFailure::at(
            "evidence_verify",
            "bundle receipt is not an acceptable result",
        ));
    }
    if entry["source_commit"] != expected_commit {
        return Err(PipelineFailure::at(
            "source_commit",
            format!(
                "bundle certifies {}, campaign requires {expected_commit}",
                entry["source_commit"]
            ),
        ));
    }
    let kind = entry["evidence_kind"].as_str().unwrap_or_default();
    if !allowed_kinds.contains(&kind) {
        return Err(PipelineFailure::at(
            "evidence_kind",
            format!("evidence kind {kind:?} is not one of {allowed_kinds:?}"),
        ));
    }
    let capture_id = entry["capture_id"]
        .as_str()
        .ok_or_else(|| PipelineFailure::at("index", "bundle has no capture id"))?
        .to_owned();
    let published =
        published_capture_ids(dir).map_err(|error| PipelineFailure::at("duplicate", error))?;
    if published.contains(&capture_id) {
        return Err(PipelineFailure::at(
            "duplicate",
            format!("capture {capture_id} was already published"),
        ));
    }
    sync_tree(&partial).map_err(|error| PipelineFailure::at("fsync", error))?;
    let bundle = task_dir.join(&capture_id);
    fs::rename(&partial, &bundle).map_err(|error| PipelineFailure::at("publish", error))?;
    fs::File::open(&task_dir)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| PipelineFailure::at("publish", error))?;
    let receipt = dir.receipts().join(task).join(format!("{capture_id}.json"));
    let receipt_body = json!({
        "schema_version": 1,
        "task": task,
        "attempt": attempt,
        "capture_id": capture_id,
        "bundle": dir.relative(&bundle),
        "suite": entry["suite"],
        "evidence_kind": entry["evidence_kind"],
        "source_commit": entry["source_commit"],
        "bundle_manifest_sha256": entry["manifest_sha256"],
        "verified_unix_ms": unix_ms(),
    });
    let index_path = dir.indexes().join(task).join(format!("{capture_id}.json"));
    write_atomic(
        &index_path,
        &serde_json::to_vec_pretty(&entry).unwrap_or_default(),
    )
    .and_then(|()| {
        write_atomic(
            &receipt,
            &serde_json::to_vec_pretty(&receipt_body).unwrap_or_default(),
        )
    })
    .map_err(|error| PipelineFailure::at("receipt", error))?;
    Ok(Published {
        capture_id,
        bundle,
        receipt,
        index_entry: entry,
    })
}

/// Keeps a failed attempt inspectable without making the LLM parse raw command output.
pub fn record_failure(
    dir: &CampaignDir,
    task: &str,
    attempt: u32,
    failure: &PipelineFailure,
) -> anyhow::Result<PathBuf> {
    let path = dir.attempt_dir(task, attempt).join("failure.json");
    write_atomic(
        &path,
        &serde_json::to_vec_pretty(&json!({
            "task": task,
            "attempt": attempt,
            "stage": failure.stage,
            "error_summary": failure.summary,
            "artifacts": dir.relative(&dir.attempt_dir(task, attempt)),
        }))?,
    )?;
    Ok(path)
}

/// Re-verifies every published bundle of `task` from disk; the receipts are not trusted.
pub fn reverify_task(dir: &CampaignDir, task: &str) -> anyhow::Result<Vec<Value>> {
    let task_dir = dir.bundles().join(task);
    let mut entries = Vec::new();
    if !task_dir.is_dir() {
        return Ok(entries);
    }
    let mut names = fs::read_dir(&task_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect::<Vec<_>>();
    names.sort();
    for name in names {
        entries.push(
            verified_index_entry(&task_dir.join(&name))
                .with_context(|| format!("published bundle {task}/{name} no longer verifies"))?,
        );
    }
    Ok(entries)
}

/// `indexes/combined.json`: the committed historical index plus the campaign's verified
/// entries. Progress reads this; the committed index is only updated after collection freezes.
pub fn rebuild_combined_index(dir: &CampaignDir, repository: &Path) -> anyhow::Result<PathBuf> {
    let committed_path = repository.join("docs/evidence/index.json");
    let mut entries: Vec<Value> = if committed_path.is_file() {
        let committed: Value = serde_json::from_slice(&fs::read(&committed_path)?)?;
        committed["entries"].as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut seen: BTreeSet<String> = entries
        .iter()
        .filter_map(|entry| entry["capture_id"].as_str().map(str::to_owned))
        .collect();
    let root = dir.indexes();
    if root.is_dir() {
        let mut files = Vec::new();
        for task in fs::read_dir(&root)? {
            let task = task?.path();
            if task.is_dir() {
                for file in fs::read_dir(&task)? {
                    files.push(file?.path());
                }
            }
        }
        files.sort();
        for file in files {
            let entry: Value = serde_json::from_slice(&fs::read(&file)?)?;
            if let Some(id) = entry["capture_id"].as_str()
                && seen.insert(id.to_owned())
            {
                entries.push(entry);
            }
        }
    }
    let path = dir.indexes().join("combined.json");
    write_atomic(
        &path,
        &serde_json::to_vec_pretty(&json!({"schema_version": 1, "entries": entries}))?,
    )?;
    Ok(path)
}
