//! The benefit stages: matched pairs through the real control path, thresholds chosen on the
//! calibration part and frozen, and one holdout evaluation. Pair data is append-only so every
//! decision can be recomputed from disk.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use stutter::api::autotune::{ObjectiveKind, RunnableThresholds};

use super::{
    benefit::{
        HoldoutVerdict, Order, PairResult, Part, RunnableThresholdsRecord, Scenario, Tally,
        evaluate_holdout, part_of, select_thresholds,
    },
    runner::{BenefitHarness, HarnessError, StageContext, StageOutcome},
};
use crate::{
    evidence_bundle::sha256::sha256_hex,
    validation_campaign::{
        model::{BlockReason, FailureClass},
        state::{unix_ms, write_atomic},
    },
};

pub fn benefit_dir(context: &StageContext<'_>) -> PathBuf {
    context.dir.0.join("benefit")
}

fn pairs_path(context: &StageContext<'_>) -> PathBuf {
    benefit_dir(context).join("pairs.jsonl")
}

fn calibration_path(context: &StageContext<'_>) -> PathBuf {
    benefit_dir(context).join("calibration.json")
}

fn holdout_path(context: &StageContext<'_>) -> PathBuf {
    benefit_dir(context).join("holdout.json")
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Calibration {
    pub thresholds: RunnableThresholdsRecord,
    pub calibration_tally: Tally,
    pub pairs_per_scenario: usize,
    pub pairs_sha256: String,
    pub calibration_pairs: usize,
    pub frozen_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HoldoutRecord {
    pub verdict: HoldoutVerdict,
    pub holdout_pairs: usize,
    pub pairs_sha256: String,
    pub evaluated_unix_ms: u64,
}

pub fn objective_kind(name: &str) -> Option<ObjectiveKind> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

pub fn read_pairs(path: &Path) -> anyhow::Result<Vec<PairResult>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("a pair record is corrupt"))
        .collect()
}

fn append_pair(path: &Path, pair: &PairResult) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", serde_json::to_string(pair)?)?;
    file.sync_all()?;
    Ok(())
}

pub fn pairs_digest(path: &Path) -> String {
    sha256_hex(&fs::read(path).unwrap_or_default())
}

fn order_for(pair: usize) -> Order {
    if pair.is_multiple_of(2) {
        Order::BaselineFirst
    } else {
        Order::CandidateFirst
    }
}

fn parts<'a>(
    pairs: &'a [PairResult],
    context: &StageContext<'_>,
    wanted: Part,
) -> Vec<&'a PairResult> {
    let split = &context.manifest.benefit;
    pairs
        .iter()
        .filter(|pair| {
            part_of(
                pair.scenario,
                pair.pair,
                split.calibration_parts,
                split.holdout_parts,
            ) == wanted
        })
        .collect()
}

/// Collects missing pairs up to `target` per scenario, in order, durably after each pair.
fn collect(
    context: &StageContext<'_>,
    harness: &dyn BenefitHarness,
    objective: ObjectiveKind,
    target: usize,
) -> Result<(), StageOutcome> {
    let path = pairs_path(context);
    let existing = read_pairs(&path).map_err(|error| {
        StageOutcome::failed(FailureClass::SoftwareDefect, "pairs", format!("{error:#}"))
    })?;
    for pair in 0..target {
        for scenario in Scenario::ALL {
            if existing
                .iter()
                .any(|known| known.scenario == scenario && known.pair == pair)
            {
                continue;
            }
            let measured = harness
                .measure(context, objective, scenario, pair, order_for(pair))
                .map_err(|error| match error {
                    HarnessError::Blocked(reason) => StageOutcome::Blocked(reason),
                    HarnessError::Retryable(summary) => {
                        StageOutcome::failed(FailureClass::Retryable, "measure", summary)
                    }
                    HarnessError::Defect(summary) => {
                        StageOutcome::failed(FailureClass::SoftwareDefect, "measure", summary)
                    }
                })?;
            append_pair(&path, &measured).map_err(|error| {
                StageOutcome::failed(FailureClass::Retryable, "persist", format!("{error:#}"))
            })?;
        }
    }
    Ok(())
}

fn pairs_per_scenario(pairs: &[PairResult]) -> usize {
    Scenario::ALL
        .into_iter()
        .map(|scenario| pairs.iter().filter(|p| p.scenario == scenario).count())
        .min()
        .unwrap_or(0)
}

pub fn calibrate(
    context: &StageContext<'_>,
    _attempt: u32,
    harness: &dyn BenefitHarness,
) -> StageOutcome {
    if calibration_path(context).is_file() {
        return StageOutcome::passed(
            Some(context.dir.relative(&benefit_dir(context))),
            Some(context.dir.relative(&calibration_path(context))),
        );
    }
    let Some(objective) = objective_kind(&context.state.objective) else {
        return StageOutcome::failed(
            FailureClass::SoftwareDefect,
            "configuration",
            format!("unknown objective {}", context.state.objective),
        );
    };
    let benefit = &context.manifest.benefit;
    let mut target = benefit.initial_pairs;
    loop {
        if let Err(outcome) = collect(context, harness, objective, target) {
            return outcome;
        }
        let pairs = match read_pairs(&pairs_path(context)) {
            Ok(pairs) => pairs,
            Err(error) => {
                return StageOutcome::failed(
                    FailureClass::SoftwareDefect,
                    "pairs",
                    format!("{error:#}"),
                );
            }
        };
        let calibration = parts(&pairs, context, Part::Calibration);
        let selected = select_thresholds(&calibration, &context.operation.thresholds)
            .filter(|(_, tally)| tally.beneficial_keep_rate() >= benefit.min_beneficial_keep_rate);
        if let Some((thresholds, tally)) = selected {
            let record = Calibration {
                thresholds: thresholds.into(),
                calibration_tally: tally,
                pairs_per_scenario: pairs_per_scenario(&pairs),
                pairs_sha256: pairs_digest(&pairs_path(context)),
                calibration_pairs: calibration.len(),
                frozen_unix_ms: unix_ms(),
            };
            let body = serde_json::to_vec_pretty(&record).unwrap_or_default();
            return match write_atomic(&calibration_path(context), &body) {
                Ok(()) => StageOutcome::passed(
                    Some(context.dir.relative(&benefit_dir(context))),
                    Some(context.dir.relative(&calibration_path(context))),
                ),
                Err(error) => {
                    StageOutcome::failed(FailureClass::Retryable, "persist", error.to_string())
                }
            };
        }
        if target >= benefit.max_pairs {
            return StageOutcome::Failed {
                class: FailureClass::ValidationFailure,
                stage: "calibration".to_owned(),
                summary: format!(
                    "no threshold set in the grid keeps the beneficial pairs without a false keep after {target} pairs per scenario"
                ),
                artifacts: Some(context.dir.relative(&benefit_dir(context))),
            };
        }
        target = (target + benefit.step).min(benefit.max_pairs);
    }
}

pub fn holdout(context: &StageContext<'_>, _attempt: u32) -> StageOutcome {
    let artifacts = Some(context.dir.relative(&benefit_dir(context)));
    // The holdout is evaluated once: a stored verdict is final.
    if let Ok(bytes) = fs::read(holdout_path(context))
        && let Ok(record) = serde_json::from_slice::<HoldoutRecord>(&bytes)
    {
        return holdout_outcome(context, &record, artifacts);
    }
    let calibration: Calibration = match fs::read(calibration_path(context))
        .context("the thresholds were not frozen")
        .and_then(|bytes| Ok(serde_json::from_slice(&bytes)?))
    {
        Ok(calibration) => calibration,
        Err(error) => {
            return StageOutcome::Blocked(BlockReason {
                class: FailureClass::BlockedResource,
                detail: format!("{error:#}"),
            });
        }
    };
    let pairs = match read_pairs(&pairs_path(context)) {
        Ok(pairs) => pairs,
        Err(error) => {
            return StageOutcome::failed(
                FailureClass::SoftwareDefect,
                "pairs",
                format!("{error:#}"),
            );
        }
    };
    let holdout_pairs = parts(&pairs, context, Part::Holdout);
    let frozen = RunnableThresholds {
        min_improvement_percent: calibration.thresholds.min_improvement_percent,
        max_regression_percent: calibration.thresholds.max_regression_percent,
        min_samples: calibration.thresholds.min_samples,
    };
    let record = HoldoutRecord {
        verdict: evaluate_holdout(
            &holdout_pairs,
            frozen,
            context.manifest.benefit.min_beneficial_keep_rate,
        ),
        holdout_pairs: holdout_pairs.len(),
        pairs_sha256: pairs_digest(&pairs_path(context)),
        evaluated_unix_ms: unix_ms(),
    };
    if let Err(error) = write_atomic(
        &holdout_path(context),
        &serde_json::to_vec_pretty(&record).unwrap_or_default(),
    ) {
        return StageOutcome::failed(FailureClass::Retryable, "persist", error.to_string());
    }
    holdout_outcome(context, &record, artifacts)
}

fn holdout_outcome(
    context: &StageContext<'_>,
    record: &HoldoutRecord,
    artifacts: Option<String>,
) -> StageOutcome {
    if record.verdict.passed {
        StageOutcome::passed(
            artifacts,
            Some(context.dir.relative(&holdout_path(context))),
        )
    } else {
        StageOutcome::Failed {
            class: FailureClass::ValidationFailure,
            stage: "holdout".to_owned(),
            summary: format!(
                "holdout: false keeps {}, beneficial keep rate {:.2} (required {:.2})",
                record.verdict.tally.false_keeps,
                record.verdict.beneficial_keep_rate,
                context.manifest.benefit.min_beneficial_keep_rate
            ),
            artifacts,
        }
    }
}
