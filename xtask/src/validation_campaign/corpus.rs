//! The `corpus` task: runs predeclared scenarios until every family floor is met by eligible
//! generated fixtures, or the per-capture retry budget is spent. Progress is recomputed from
//! the fixtures on disk and an append-only rejection log, so a killed run resumes exactly where
//! it stopped and a rejected capture never counts.
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use super::{
    executor::{FingerprintExtras, RunContext, RunnerOutcome, TaskRunner},
    graph::TaskSpec,
    model::{BlockReason, FailureClass},
    repository::{file_sha256, git},
    scenarios::{
        CorpusPlan,
        exec::{CaptureHost, CaptureRequest, CaptureResult, run_capture},
        family_summary,
        model::ScenarioSet,
    },
    state::unix_ms,
};
use crate::diagnosis_calibration::load_corpus;

pub const SCENARIO_FILE: &str = "docs/validation/scenarios/core-validation.toml";

/// Builds the real (or, in tests, scripted) host for one corpus run. `Err` is an honest block:
/// a missing display, tool or privilege, never a retry loop.
pub trait HostFactory {
    fn host(
        &self,
        context: &RunContext<'_>,
        set: &ScenarioSet,
    ) -> Result<Box<dyn CaptureHost>, BlockReason>;

    /// The checkout is still the clean evidence base (re-checked after every recording).
    fn source_matches(&self, context: &RunContext<'_>) -> bool {
        git(context.repository, &["rev-parse", "HEAD"])
            .is_ok_and(|head| head.trim() == context.state.evidence_base)
            && git(context.repository, &["status", "--porcelain"])
                .is_ok_and(|status| status.trim().is_empty())
    }
}

pub struct CorpusRunner {
    pub factory: Box<dyn HostFactory>,
}

pub fn corpus_dir(context: &RunContext<'_>) -> PathBuf {
    context.dir.0.join("corpus")
}

/// Failed attempts per scenario, from the rejection log.
pub fn rejection_counts(corpus: &Path) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let text = fs::read_to_string(corpus.join("rejections.ndjson")).unwrap_or_default();
    for line in text.lines() {
        if let Some(id) = serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|v| v["scenario"].as_str().map(str::to_owned))
        {
            *counts.entry(id).or_default() += 1;
        }
    }
    counts
}

fn log_rejection(corpus: &Path, scenario: &str, stage: &str, reasons: &[String]) {
    let line =
        json!({"scenario": scenario, "unix_ms": unix_ms(), "stage": stage, "reasons": reasons});
    if let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(corpus.join("rejections.ndjson"))
    {
        let _ = writeln!(file, "{line}");
        let _ = file.sync_all();
    }
}

fn blocked(detail: String) -> RunnerOutcome {
    RunnerOutcome::Blocked(BlockReason {
        class: FailureClass::BlockedResource,
        detail,
    })
}

/// Plan from the fixtures present now.
pub fn plan(set: &ScenarioSet, corpus: &Path, context: &RunContext<'_>) -> (CorpusPlan, Value) {
    let fixtures = load_corpus(&corpus.join("fixtures"))
        .map(|(fixtures, _)| fixtures)
        .unwrap_or_default();
    let min = (
        context.manifest.corpus.min_positive,
        context.manifest.corpus.min_negative_control,
        context.manifest.corpus.min_mixed,
    );
    let plan = CorpusPlan::from_fixtures(
        set,
        &fixtures,
        &rejection_counts(corpus),
        context.manifest.corpus.failed_attempts_per_capture,
    );
    let summary = family_summary(&fixtures, min, &plan);
    (plan, summary)
}

impl TaskRunner for CorpusRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let scenario_path = context.repository.join(SCENARIO_FILE);
        let loaded = fs::read_to_string(&scenario_path)
            .map_err(|e| e.to_string())
            .and_then(|text| ScenarioSet::parse(&text).map_err(|e| e.to_string()))
            .and_then(|set| {
                set.validate_against(context.manifest)
                    .map(|()| set)
                    .map_err(|e| e.to_string())
            });
        let set = match loaded {
            Ok(set) => set,
            Err(error) => {
                return RunnerOutcome::Failed {
                    class: FailureClass::SoftwareDefect,
                    stage: "scenarios".to_owned(),
                    summary: format!("the scenario set is unusable: {error}"),
                    artifacts: None,
                };
            }
        };
        let corpus = corpus_dir(context);
        let fixtures_dir = corpus.join("fixtures");
        let raw_root = context.dir.0.join("raw-captures");
        if let Err(error) = fs::create_dir_all(&fixtures_dir) {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "storage".to_owned(),
                summary: error.to_string(),
                artifacts: None,
            };
        }
        let mut host = match self.factory.host(context, &set) {
            Ok(host) => host,
            Err(block) => return RunnerOutcome::Blocked(block),
        };
        let evidence_base = context.state.evidence_base.clone();
        let matches = || self.factory.source_matches(context);
        loop {
            let (current, _) = plan(&set, &corpus, context);
            if current.complete() {
                break;
            }
            let Some(next) = current.next_scenarios(1).into_iter().next() else {
                let why = current
                    .exhausted()
                    .unwrap_or_else(|| "no scenario can make progress".to_owned());
                return blocked(format!("corpus generation is exhausted: {why}"));
            };
            let progress = current.scenarios.iter().find(|s| s.id == next);
            let sequence = progress.map_or(0, |s| s.have + s.failed) + 1;
            let raw_capture_id = format!("{next}-{sequence}-{}", unix_ms());
            let raw_dir = raw_root.join(&raw_capture_id);
            let published = load_corpus(&fixtures_dir)
                .map(|(fixtures, _)| {
                    fixtures
                        .iter()
                        .filter_map(|f| f.meta.as_ref().map(|m| m.capture_id.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let Some(scenario) = set.get(&next) else {
                return blocked(format!("scenario {next} vanished from the set"));
            };
            let request = CaptureRequest {
                spec: scenario,
                campaign_id: &context.state.campaign_id,
                attempt: sequence as u32,
                raw_capture_id: &raw_capture_id,
                source_commit: &evidence_base,
                raw_dir: &raw_dir,
                fixtures_dir: &fixtures_dir,
                published_ids: &published,
                fingerprint_matches: &matches,
            };
            if let CaptureResult::Rejected { stage, reasons } = run_capture(host.as_mut(), &request)
            {
                log_rejection(&corpus, &next, &stage, &reasons);
            }
        }
        let (_, summary) = plan(&set, &corpus, context);
        let _ = fs::write(
            corpus.join("summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap_or_default(),
        );
        let Ok(scenario_sha256) = file_sha256(&scenario_path) else {
            return blocked("the scenario file disappeared".to_owned());
        };
        RunnerOutcome::Passed {
            artifacts: Some(context.dir.relative(&corpus)),
            receipt: None,
            limitations: Vec::new(),
            fingerprint: Box::new(FingerprintExtras {
                scenario_sha256: Some(scenario_sha256),
                ..FingerprintExtras::default()
            }),
            updates: Vec::new(),
        }
    }
}
