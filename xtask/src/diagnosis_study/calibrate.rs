//! Recalibration against the study's own corpus, with an anti-overfitting split. Captures are
//! partitioned by a stable hash of their capture id (stratified by scenario, i.e. family and
//! role); the split is persisted so reruns can never reshuffle it. Thresholds are chosen on the
//! calibration set only, and replace the current default only if the untouched holdout also
//! agrees. Without that, the current threshold is retained.
use std::{fs, path::Path};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    inspect::COMPLETE_MARKER,
    model::StudyConfig,
    plan::family_passed,
    promote::link_tree,
    store::{StudyDir, StudyHeader},
};
use crate::{
    diagnosis_calibration::{FAMILIES, Fixture, load_corpus, report},
    evidence_bundle::sha256::sha256_hex,
    validation_campaign::scenarios::model::Family,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Calibration,
    Holdout,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SplitEntry {
    pub capture_id: String,
    pub fixture: String,
    pub scenario: String,
    pub part: Part,
}

/// Deterministic split: within each scenario, captures ordered by `sha256(capture_id)` go
/// calibration, calibration, holdout, calibration, calibration, holdout, ... (2:1), so both parts
/// always see both roles of every family and 15 matched pairs leave the calibration part with
/// the evidence floor of 10 positives and 10 controls.
pub fn build_split(fixtures: &[Fixture]) -> Vec<SplitEntry> {
    let mut by_scenario: std::collections::BTreeMap<String, Vec<(String, &Fixture)>> =
        Default::default();
    for fixture in fixtures {
        let Some(meta) = &fixture.meta else { continue };
        by_scenario
            .entry(fixture.scenario.clone())
            .or_default()
            .push((sha256_hex(meta.capture_id.as_bytes()), fixture));
    }
    let mut entries = Vec::new();
    for (scenario, mut group) in by_scenario {
        group.sort_by(|a, b| a.0.cmp(&b.0));
        for (index, (_, fixture)) in group.into_iter().enumerate() {
            entries.push(SplitEntry {
                capture_id: fixture
                    .meta
                    .as_ref()
                    .map(|m| m.capture_id.clone())
                    .unwrap_or_default(),
                fixture: fixture.name.clone(),
                scenario: scenario.clone(),
                part: if index % 3 == 2 {
                    Part::Holdout
                } else {
                    Part::Calibration
                },
            });
        }
    }
    entries
}

/// One threshold's counts on one dataset, as the calibration report states them.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub threshold: f64,
    pub tp: u64,
    pub fp: u64,
    pub fn_: u64,
    pub precision: Option<f64>,
    pub recall: Option<f64>,
    pub false_high_observation: u64,
    pub false_high_causal: u64,
}

impl Row {
    pub fn parse(value: &Value) -> Option<Self> {
        Some(Self {
            threshold: value["threshold"].as_f64()?,
            tp: value["tp"].as_u64()?,
            fp: value["fp"].as_u64()?,
            fn_: value["fn"].as_u64()?,
            precision: value["precision"].as_f64(),
            recall: value["recall"].as_f64(),
            false_high_observation: value["false_high_observation_confidence"].as_u64()?,
            false_high_causal: value["false_high_causal_confidence"].as_u64()?,
        })
    }

    fn precision_or_zero(&self) -> f64 {
        self.precision.unwrap_or(0.0)
    }

    fn recall_or_zero(&self) -> f64 {
        self.recall.unwrap_or(0.0)
    }
}

pub fn rows(report: &Value, key: &str) -> Vec<Row> {
    report[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Row::parse)
        .collect()
}

fn mixed_false_high(report: &Value, threshold: f64) -> u64 {
    report["mixed_sweep"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["threshold"].as_f64() == Some(threshold))
        .and_then(|row| row["mixed_false_high_causal_claims"].as_u64())
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub selected: f64,
    pub changed: bool,
    pub reasons: Vec<String>,
    /// No threshold of the sweep recovers a single positive: the gap is in the diagnosis logic.
    pub logic_gap: bool,
}

fn find(rows: &[Row], threshold: f64) -> Option<&Row> {
    rows.iter().find(|r| r.threshold == threshold)
}

/// The lexicographic choice on the calibration set, then the holdout gate. See the module docs.
pub fn decide(
    current: f64,
    calibration: &[Row],
    holdout: &[Row],
    calibration_report: &Value,
    holdout_report: &Value,
) -> Decision {
    let retain = |reasons: Vec<String>, logic_gap| Decision {
        selected: current,
        changed: false,
        reasons,
        logic_gap,
    };
    let Some(base) = find(calibration, current) else {
        return retain(
            vec!["the current threshold is not in the sweep".to_owned()],
            false,
        );
    };
    let logic_gap = !calibration.is_empty() && calibration.iter().all(|r| r.tp == 0);
    let mut candidates: Vec<&Row> = calibration
        .iter()
        .filter(|r| {
            r.false_high_causal <= base.false_high_causal
                && r.false_high_observation <= base.false_high_observation
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.precision_or_zero()
            .total_cmp(&a.precision_or_zero())
            .then(b.recall_or_zero().total_cmp(&a.recall_or_zero()))
            .then(
                (a.threshold - current)
                    .abs()
                    .total_cmp(&(b.threshold - current).abs()),
            )
    });
    let Some(best) = candidates.first().copied() else {
        return retain(
            vec!["no candidate keeps the false-high counts".to_owned()],
            logic_gap,
        );
    };
    if best.threshold == current {
        return retain(
            vec!["the calibration set prefers the current threshold".to_owned()],
            logic_gap,
        );
    }
    let (Some(hold_best), Some(hold_current)) =
        (find(holdout, best.threshold), find(holdout, current))
    else {
        return retain(
            vec!["the holdout set lacks the candidate".to_owned()],
            logic_gap,
        );
    };
    let mut failures = Vec::new();
    let mut check = |ok: bool, text: &str| {
        if !ok {
            failures.push(text.to_owned());
        }
    };
    check(
        hold_best.recall_or_zero() > hold_current.recall_or_zero(),
        "holdout recall does not improve",
    );
    check(
        hold_best.precision_or_zero() >= hold_current.precision_or_zero(),
        "holdout precision falls",
    );
    check(
        hold_best.fp <= hold_current.fp,
        "holdout false positives increase",
    );
    check(
        hold_best.false_high_causal <= hold_current.false_high_causal
            && hold_best.false_high_observation <= hold_current.false_high_observation,
        "holdout false-high confidence increases",
    );
    check(
        mixed_false_high(holdout_report, best.threshold)
            <= mixed_false_high(holdout_report, current),
        "mixed/confounded behavior worsens on the holdout",
    );
    let _ = calibration_report;
    if failures.is_empty() {
        Decision {
            selected: best.threshold,
            changed: true,
            reasons: vec![format!(
                "calibration prefers {} and the untouched holdout agrees",
                best.threshold
            )],
            logic_gap,
        }
    } else {
        Decision {
            selected: current,
            changed: false,
            reasons: std::iter::once(format!(
                "calibration prefers {} but the holdout does not generalize it",
                best.threshold
            ))
            .chain(failures)
            .collect(),
            logic_gap,
        }
    }
}

fn materialize(
    study: &StudyDir,
    split: &[SplitEntry],
    part: Part,
    name: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let dir = study.calibration().join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir)?;
    for entry in split.iter().filter(|e| e.part == part) {
        link_tree(
            &study.calibration().join("fixtures").join(&entry.fixture),
            &dir.join(&entry.fixture),
        )?;
    }
    Ok(dir)
}

/// Which parts a run may look at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// Sweeps the calibration part only; the holdout is never loaded. Used while designing a
    /// diagnosis change.
    CalibrationOnly,
    /// The single final evaluation: both parts, thresholds decided. Refused if one was recorded.
    Final,
}

/// Splits (once), sweeps the parts allowed by `mode` for every family whose impact study
/// passed and, in the final mode, decides.
pub fn calibrate(
    root: &Path,
    study: &StudyDir,
    header: &StudyHeader,
    config: &StudyConfig,
    mode: Mode,
) -> anyhow::Result<Value> {
    ensure!(
        study.0.join(COMPLETE_MARKER).is_file(),
        "calibration needs a completed study"
    );
    let decisions = study.decisions()?;
    let (fixtures, corpus_id) = load_corpus(&study.calibration().join("fixtures"))
        .context("the study has no promoted calibration fixtures")?;
    let final_path = study.calibration().join("decisions.json");
    ensure!(
        mode == Mode::CalibrationOnly || !final_path.exists(),
        "the final holdout evaluation was already run; it is never repeated"
    );
    let split_path = study.calibration().join("split.json");
    let split: Vec<SplitEntry> = match fs::read(&split_path) {
        Ok(bytes) => {
            let split: Vec<SplitEntry> = serde_json::from_slice(&bytes)?;
            let known: std::collections::BTreeSet<&str> =
                split.iter().map(|e| e.fixture.as_str()).collect();
            if fixtures.iter().any(|f| !known.contains(f.name.as_str())) {
                bail!("fixtures appeared after the split was fixed; it is never reshuffled");
            }
            split
        }
        Err(_) => {
            let split = build_split(&fixtures);
            fs::write(&split_path, serde_json::to_vec_pretty(&split)?)?;
            split
        }
    };
    let cal_dir = materialize(study, &split, Part::Calibration, "set-calibration")?;
    let hold_dir = (mode == Mode::Final)
        .then(|| materialize(study, &split, Part::Holdout, "set-holdout"))
        .transpose()?;
    let mut families = serde_json::Map::new();
    for spec in &FAMILIES {
        let family = Family::ALL
            .iter()
            .copied()
            .find(|f| f.as_str() == spec.name)
            .context("family")?;
        if !family_passed(&decisions, family) {
            families.insert(
                spec.name.to_owned(),
                json!({"status": "not_calibrated", "reason": "its target-impact study did not pass"}),
            );
            continue;
        }
        let cal = report(root, &cal_dir, spec.name)?;
        let Some(hold_dir) = &hold_dir else {
            families.insert(
                spec.name.to_owned(),
                json!({"status": "calibration_only", "calibration": cal}),
            );
            continue;
        };
        let hold = report(root, hold_dir, spec.name)?;
        let decision = decide(
            spec.current,
            &rows(&cal, "sweep"),
            &rows(&hold, "sweep"),
            &cal,
            &hold,
        );
        families.insert(
            spec.name.to_owned(),
            json!({
                "status": "calibrated",
                "current_threshold": spec.current,
                "unit": spec.unit,
                "selected_threshold": decision.selected,
                "changed": decision.changed,
                "diagnosis_logic_gap": decision.logic_gap,
                "reasons": decision.reasons,
                "calibration": cal,
                "holdout": hold,
            }),
        );
    }
    let result = json!({
        "schema_version": 1,
        "study_id": header.study_id,
        "corpus_id": corpus_id,
        "split_sha256": sha256_hex(&fs::read(&split_path)?),
        "captures": {"calibration": split.iter().filter(|e| e.part == Part::Calibration).count(),
                      "holdout": split.iter().filter(|e| e.part == Part::Holdout).count()},
        "selection_rule": "calibration: no new false-high claims, then precision, then recall, then smallest change; replaced only if the untouched holdout also improves recall without lower precision, more false positives, more false-high claims or worse mixed behavior",
        "families": families,
        "diagnosis_remains": "Experimental",
        "mixed_scenarios": config.mixed.iter().map(|m| m.id.clone()).collect::<Vec<_>>(),
    });
    let out = if mode == Mode::Final {
        final_path
    } else {
        study.calibration().join("calibration-only.json")
    };
    fs::write(out, serde_json::to_vec_pretty(&result)?)?;
    Ok(result)
}
