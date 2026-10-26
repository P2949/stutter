//! The `evidence-integration` task: after every collection task is done, independently re-verify
//! each published bundle, derive the index entries from the verified bundles, add the sanitized
//! corpus fixtures and a status-ledger entry, run the offline gates and land the result as one pull
//! request. Hand-authored digests or entries are impossible: they come out of `verify_bundle`.
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use super::{
    evidence::reverify_task,
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::TaskSpec,
    landing::Landed,
    landing_task::LandingTools,
    verify::produces_bundles,
};

pub const INDEX: &str = "docs/evidence/index.json";
pub const FIXTURES: &str = "stutter/tests/fixtures/runs";
pub const LEDGER: &str = "docs/CORE_VALIDATION_STATUS.md";

/// Verified entries of the campaign; injectable so tests need no real bundles.
pub type EntrySource = dyn Fn(&RunContext<'_>) -> Result<Vec<Value>, String>;

pub struct IntegrationRunner {
    pub tools: LandingTools,
    pub entries: Box<EntrySource>,
}

/// Every verified entry of every bundle-producing task, re-derived from disk.
pub fn verified_entries(context: &RunContext<'_>) -> Result<Vec<Value>, String> {
    let graph = super::graph::Graph::core().map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for spec in graph.specs() {
        if produces_bundles(spec.kind) && !spec.id.starts_with("rc-") {
            entries.extend(reverify_task(context.dir, spec.id).map_err(|e| format!("{e:#}"))?);
        }
    }
    Ok(entries)
}

/// The committed index plus `new`, deduplicated by capture id; new entries are appended in a
/// stable order. Returns the merged document and how many entries were added.
pub fn merge_index(existing: &Value, new: &[Value]) -> (Value, Vec<Value>) {
    let mut entries = existing["entries"].as_array().cloned().unwrap_or_default();
    let mut seen: std::collections::BTreeSet<String> = entries
        .iter()
        .filter_map(|e| e["capture_id"].as_str().map(str::to_owned))
        .collect();
    let mut added: Vec<Value> = new
        .iter()
        .filter(|e| {
            e["capture_id"]
                .as_str()
                .is_some_and(|id| seen.insert(id.to_owned()))
        })
        .cloned()
        .collect();
    added.sort_by_key(|e| {
        (
            e["suite"].as_str().unwrap_or("").to_owned(),
            e["capture_id"].as_str().unwrap_or("").to_owned(),
        )
    });

    entries.extend(added.iter().cloned());
    let mut merged = existing.clone();
    merged["entries"] = Value::Array(entries);
    (merged, added)
}

/// Copies accepted fixtures; a same-named directory with different content is refused.
pub fn copy_fixtures(from: &Path, to: &Path) -> Result<usize, String> {
    let Ok(read) = fs::read_dir(from) else {
        return Ok(0);
    };
    let mut copied = 0;
    let mut dirs: Vec<PathBuf> = read.filter_map(Result::ok).map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        if name.starts_with('.') || !dir.join("fixture.toml").is_file() {
            continue;
        }
        let target = to.join(&name);
        fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        for file in fs::read_dir(&dir)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
        {
            if !file.path().is_file() {
                continue;
            }
            let bytes = fs::read(file.path()).map_err(|e| e.to_string())?;
            let destination = target.join(file.file_name());
            match fs::read(&destination) {
                Ok(existing) if existing == bytes => {}
                Ok(_) => {
                    return Err(format!(
                        "fixture {name}/{} already exists with different content",
                        file.file_name().to_string_lossy()
                    ));
                }
                Err(_) => fs::write(&destination, bytes).map_err(|e| e.to_string())?,
            }
        }
        copied += 1;
    }
    Ok(copied)
}

/// Deterministic ledger text; identical inputs give identical bytes.
pub fn ledger_entry(
    campaign: &str,
    base: &str,
    added: &[Value],
    fixtures: usize,
    corpus: &Value,
) -> String {
    let mut per_suite: std::collections::BTreeMap<&str, usize> = Default::default();
    let mut kernels = std::collections::BTreeSet::new();
    for entry in added {
        *per_suite
            .entry(entry["suite"].as_str().unwrap_or("unknown"))
            .or_default() += 1;
        if let Some(kernel) = entry["kernel_release"].as_str() {
            kernels.insert(kernel.to_owned());
        }
    }
    let suites = per_suite
        .iter()
        .map(|(suite, n)| format!("{suite} x{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\n## Campaign {campaign}: evidence integration\n\nEvidence collected at `{base}` was re-verified from the retained bundles and indexed: {suites}. Distinct kernels observed: {}. Sanitized corpus fixtures added: {fixtures}. Corpus floors: {}.\nBundles stay with the operator; entries pin them by manifest digest.\n",
        if kernels.is_empty() {
            "none".to_owned()
        } else {
            kernels.into_iter().collect::<Vec<_>>().join(", ")
        },
        if corpus.is_null() {
            "not recorded".to_owned()
        } else {
            corpus.to_string()
        },
    )
}

fn apply(context: &RunContext<'_>, entries: &[Value], repository: &Path) -> Result<(), String> {
    let index_path = repository.join(INDEX);
    let existing: Value = fs::read(&index_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({"schema_version": 1, "entries": []}));
    let (merged, added) = merge_index(&existing, entries);
    if !added.is_empty() {
        let mut text = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?;
        text.push('\n');
        fs::write(&index_path, text).map_err(|e| e.to_string())?;
    }
    let fixtures = copy_fixtures(
        &context.dir.0.join("corpus/fixtures"),
        &repository.join(FIXTURES),
    )?;
    let ledger_path = repository.join(LEDGER);
    let ledger = fs::read_to_string(&ledger_path).unwrap_or_default();
    let heading = format!(
        "## Campaign {}: evidence integration",
        context.state.campaign_id
    );
    if !ledger.contains(&heading) && (!added.is_empty() || fixtures > 0) {
        let corpus = context
            .state
            .frozen
            .get("corpus.summary")
            .and_then(|t| serde_json::from_str(t).ok())
            .unwrap_or(Value::Null);
        let text = ledger_entry(
            &context.state.campaign_id,
            &context.state.evidence_base,
            &added,
            fixtures,
            &corpus,
        );
        fs::write(&ledger_path, format!("{}{text}", ledger.trim_end()))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl TaskRunner for IntegrationRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let entries = match (self.entries)(context) {
            Ok(entries) => entries,
            Err(error) => {
                return RunnerOutcome::Failed {
                    class: super::model::FailureClass::SoftwareDefect,
                    stage: "reverify".to_owned(),
                    summary: error,
                    artifacts: None,
                };
            }
        };
        let landed = self.tools.land(
            context,
            spec,
            attempt,
            &format!(
                "docs(evidence): integrate campaign {} evidence",
                context.state.campaign_id
            ),
            &|repository| apply(context, &entries, repository),
        );
        match landed {
            Ok(Landed::Merged { .. } | Landed::NoChange) => RunnerOutcome::passed(),
            Err(outcome) => outcome,
        }
    }
}
