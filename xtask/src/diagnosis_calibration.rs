//! Scheduler-delay diagnosis calibration over a frozen corpus. The sweep drives the
//! production report pipeline (`scheduler_calibration_outcome`); this module only labels,
//! scores and decides. It never edits thresholds: a change is a reviewed code change
//! backed by the retained result, and without enough evidence the decision is to retain.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, ensure};
use serde_json::{Value, json};
use stutter::api::report::scheduler_calibration_outcome;

use crate::evidence_bundle::sha256::sha256_hex;

pub const CURRENT_THRESHOLD_NS: u64 = 2_000_000;
pub const DEFAULT_SWEEP_MS: [f64; 7] = [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 4.0];
const SCHEDULER_FAMILY: [&str; 2] = ["CompositorSchedulerDelay", "GameThreadSchedulerDelay"];
/// Evidence floors below which the decision is always "retain".
const MIN_REAL_POSITIVES: usize = 10;
const MIN_REAL_NEGATIVES: usize = 10;
const MIN_MIXED: usize = 3;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Fixture {
    pub name: String,
    pub dir: PathBuf,
    pub real: bool,
    pub mixed: bool,
    pub expected_cause: String,
    /// `true` positive label, `false` negative control, `None` unlabeled (e.g. `Any`).
    pub positive: Option<bool>,
}

pub fn parse_fixture(dir: &Path, text: &str) -> Fixture {
    let mut fixture = Fixture {
        dir: dir.to_owned(),
        ..Fixture::default()
    };
    let (mut section, mut source) = (String::new(), String::new());
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_owned();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match (section.as_str(), key.trim()) {
            ("", "name") => fixture.name = value,
            ("", "source") => source = value,
            ("[expected]", "primary_cause") => fixture.expected_cause = value,
            ("[platform]", "scenario") => {
                fixture.mixed = value.contains("mixed") || value.contains("confound")
            }
            _ => {}
        }
    }
    fixture.real = fixture.name.starts_with("real_")
        || source == "sanitized-real-recording"
        || source == "validation-corpus";
    fixture.positive = match fixture.expected_cause.as_str() {
        "" | "Any" => None,
        cause => Some(SCHEDULER_FAMILY.contains(&cause)),
    };
    fixture
}

pub fn load_corpus(corpus: &Path) -> anyhow::Result<(Vec<Fixture>, String)> {
    let mut dirs = fs::read_dir(corpus)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("fixture.toml").is_file())
        .collect::<Vec<_>>();
    dirs.sort();
    let mut fixtures = Vec::new();
    let mut frozen = String::new();
    for dir in dirs {
        let text = fs::read_to_string(dir.join("fixture.toml"))?;
        frozen.push_str(&format!(
            "{}\n{text}\n",
            dir.file_name().and_then(|n| n.to_str()).unwrap_or("")
        ));
        fixtures.push(parse_fixture(&dir, &text));
    }
    Ok((fixtures, sha256_hex(frozen.as_bytes())))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counts {
    pub tp: usize,
    pub fp: usize,
    pub fn_: usize,
    pub tn: usize,
    pub false_high_confidence: usize,
}
impl Counts {
    pub fn precision(&self) -> Option<f64> {
        (self.tp + self.fp > 0).then(|| self.tp as f64 / (self.tp + self.fp) as f64)
    }
    pub fn recall(&self) -> Option<f64> {
        (self.tp + self.fn_ > 0).then(|| self.tp as f64 / (self.tp + self.fn_) as f64)
    }
}

/// `predicted` is the primary cause the pipeline produced (or `None`) with its confidence.
pub fn score(labels: &[(bool, Option<(String, String)>)]) -> Counts {
    let mut counts = Counts::default();
    for (positive, predicted) in labels {
        let scheduler = predicted
            .as_ref()
            .is_some_and(|(cause, _)| SCHEDULER_FAMILY.contains(&cause.as_str()));
        match (positive, scheduler) {
            (true, true) => counts.tp += 1,
            (true, false) => counts.fn_ += 1,
            (false, true) => {
                counts.fp += 1;
                if predicted
                    .as_ref()
                    .is_some_and(|(_, confidence)| confidence == "High")
                {
                    counts.false_high_confidence += 1;
                }
            }
            (false, false) => counts.tn += 1,
        }
    }
    counts
}

pub fn run(
    root: &Path,
    corpus: &Path,
    out: Option<&Path>,
    thresholds_ms: &[f64],
) -> anyhow::Result<()> {
    let (fixtures, frozen_id) = load_corpus(&root.join(corpus))?;
    let labeled = fixtures
        .iter()
        .filter(|f| f.positive.is_some())
        .collect::<Vec<_>>();
    ensure!(!labeled.is_empty(), "corpus has no labeled fixtures");
    let mut results = Vec::new();
    // Agreement is measured against the current threshold, computed up front so sweep
    // order cannot matter.
    let mut baseline: BTreeMap<String, String> = BTreeMap::new();
    for fixture in &labeled {
        let outcome = scheduler_calibration_outcome(&fixture.dir, CURRENT_THRESHOLD_NS)
            .with_context(|| format!("diagnose {}", fixture.name))?;
        baseline.insert(
            fixture.name.clone(),
            outcome.map(|o| o.cause).unwrap_or_default(),
        );
    }
    for threshold_ms in thresholds_ms {
        let threshold_ns = (threshold_ms * 1_000_000.0).round() as u64;
        let mut labels = Vec::new();
        let mut confidences: BTreeMap<String, usize> = BTreeMap::new();
        let mut stable = 0;
        for fixture in &labeled {
            let outcome = scheduler_calibration_outcome(&fixture.dir, threshold_ns)
                .with_context(|| format!("diagnose {}", fixture.name))?;
            let cause = outcome
                .as_ref()
                .map(|o| o.cause.clone())
                .unwrap_or_default();
            if baseline.get(&fixture.name).is_some_and(|b| *b == cause) {
                stable += 1;
            }
            if SCHEDULER_FAMILY.contains(&cause.as_str()) {
                *confidences
                    .entry(
                        outcome
                            .as_ref()
                            .map_or(String::new(), |o| o.confidence.clone()),
                    )
                    .or_default() += 1;
            }
            labels.push((
                fixture.positive == Some(true),
                outcome.map(|o| (o.cause, o.confidence)),
            ));
        }
        let counts = score(&labels);
        results.push(json!({"threshold_ms":threshold_ms,"tp":counts.tp,"fp":counts.fp,"fn":counts.fn_,"tn":counts.tn,
            "precision":counts.precision(),"recall":counts.recall(),"false_high_confidence":counts.false_high_confidence,
            "primary_cause_agreement_with_current":stable as f64 / labeled.len() as f64,
            "scheduler_confidence_distribution":confidences}));
    }
    let count = |real: bool, positive: bool| {
        labeled
            .iter()
            .filter(|f| f.real == real && f.positive == Some(positive))
            .count()
    };
    let mixed = fixtures.iter().filter(|f| f.mixed).count();
    let (real_pos, real_neg) = (count(true, true), count(true, false));
    let sufficient =
        real_pos >= MIN_REAL_POSITIVES && real_neg >= MIN_REAL_NEGATIVES && mixed >= MIN_MIXED;
    let decision = if sufficient {
        "evidence_sufficient_for_review"
    } else {
        "retain_current_threshold_evidence_insufficient"
    };
    let report = json!({"schema_version":1,"family":"scheduler_delay","corpus_id":frozen_id,"current_threshold_ms":2.0,
        "corpus":{"labeled":labeled.len(),"real_positive":real_pos,"real_negative_controls":real_neg,"mixed_confounded":mixed,
            "unlabeled":fixtures.len() - labeled.len()},
        "evidence_floors":{"real_positive":MIN_REAL_POSITIVES,"real_negative":MIN_REAL_NEGATIVES,"mixed":MIN_MIXED},
        "sweep":results,"decision":decision,
        "limitations":["family-level labels (scheduler vs not); specific-cause agreement is not scored",
            "mixed/confounded cases are only counted when fixtures are tagged mixed; none are tagged today",
            "diagnosis existence confidence and causal confidence are one value in the pipeline and are NOT separately calibrated",
            "numerical optimum is never chosen mechanically; false high-confidence claims weigh more than inconclusive ones"]});
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(out) = out {
        fs::write(root.join(out), serde_json::to_vec_pretty(&report)?)?;
    }
    let _: Value = report;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_expected_cause_and_skip_wildcards() {
        let text = "name = \"real_x\"\nsource = \"sanitized-real-recording\"\n[expected]\nprimary_cause = \"GameThreadSchedulerDelay\"\n";
        let fixture = parse_fixture(Path::new("d"), text);
        assert!(fixture.real && fixture.positive == Some(true));
        let control = parse_fixture(
            Path::new("d"),
            "name = \"c\"\n[expected]\nprimary_cause = \"IrqDelayCandidate\"\n[platform]\nscenario = \"mixed-io\"\n",
        );
        assert_eq!(
            (control.positive, control.mixed, control.real),
            (Some(false), true, false)
        );
        assert_eq!(
            parse_fixture(
                Path::new("d"),
                "name = \"a\"\n[expected]\nprimary_cause = \"Any\"\n"
            )
            .positive,
            None
        );
    }

    #[test]
    fn scoring_counts_false_high_confidence_separately() {
        let hit = |c: &str| Some(("GameThreadSchedulerDelay".to_owned(), c.to_owned()));
        let counts = score(&[
            (true, hit("High")),
            (true, None),
            (false, hit("High")),
            (false, hit("Low")),
            (false, Some(("IrqDelayCandidate".into(), "High".into()))),
        ]);
        assert_eq!(
            (
                counts.tp,
                counts.fn_,
                counts.fp,
                counts.tn,
                counts.false_high_confidence
            ),
            (1, 1, 2, 1, 1)
        );
        assert_eq!(counts.precision(), Some(1.0 / 3.0));
        assert_eq!(counts.recall(), Some(0.5));
        assert_eq!(Counts::default().precision(), None);
    }

    #[test]
    fn real_corpus_sweep_retains_the_threshold_because_mixed_evidence_is_missing() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let (fixtures, id) = load_corpus(&root.join("stutter/tests/fixtures/runs")).unwrap();
        assert_eq!(id.len(), 64);
        assert!(fixtures.iter().filter(|f| f.positive == Some(true)).count() >= 4);
        assert_eq!(fixtures.iter().filter(|f| f.mixed).count(), 0);
        let default = scheduler_calibration_outcome(
            &fixtures
                .iter()
                .find(|f| f.name == "game_thread_scheduler_delay")
                .unwrap()
                .dir,
            CURRENT_THRESHOLD_NS,
        )
        .unwrap()
        .unwrap();
        assert_eq!(default.cause, "GameThreadSchedulerDelay");
    }
}
