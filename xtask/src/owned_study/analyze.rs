//! Deterministic analysis of a finished (or partial) study: paired block effects, a seeded
//! bootstrap interval, a verdict by the frozen practical threshold and a held-out ranking that can
//! only ever contain the manifest's legal candidates.
use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use stutter::api::search::{
    ConfigSpace, Dimension, ensemble::held_out_ranking, learning::import_measurements,
};

use super::{manifest::Manifest, run::Record};

const RESAMPLES: usize = 10_000;

/// A training or holdout split of the legal candidates' improvements, and how many trials fell outside the space.
type Split = (BTreeMap<Vec<String>, Vec<f64>>, usize);

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// 95% percentile-bootstrap interval of the mean over blocks, seeded from the manifest digest.
pub fn bootstrap(values: &[f64], seed: u64) -> (f64, f64) {
    let mut state = seed;
    let mut means: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            let sum: f64 = (0..values.len())
                .map(|_| values[(splitmix(&mut state) % values.len() as u64) as usize])
                .sum();
            sum / values.len() as f64
        })
        .collect();
    means.sort_by(f64::total_cmp);
    (
        means[RESAMPLES * 25 / 1000],
        means[RESAMPLES * 975 / 1000 - 1],
    )
}

pub fn verdict(low: f64, high: f64, threshold: f64) -> &'static str {
    if low > threshold {
        "Beneficial"
    } else if high < -threshold {
        "Harmful"
    } else if low > -threshold && high < threshold {
        "Equivalent"
    } else {
        "Inconclusive"
    }
}

/// Improvement in percent per candidate and block: `(t_baseline - t_candidate) / t_baseline`.
/// A block counts only when the baseline and the candidate are both valid.
pub fn improvements(
    manifest: &Manifest,
    records: &[Record],
) -> BTreeMap<String, BTreeMap<u32, f64>> {
    let time = |block: u32, arm: &str| {
        records
            .iter()
            .find(|r| r.block == block && r.arm == arm && r.valid)
            .and_then(|r| r.elapsed_ns)
    };
    let mut out: BTreeMap<String, BTreeMap<u32, f64>> = BTreeMap::new();
    for candidate in &manifest.candidates {
        for block in 1..=manifest.blocks {
            if let (Some(base), Some(value)) =
                (time(block, "baseline"), time(block, &candidate.name))
            {
                out.entry(candidate.name.clone())
                    .or_default()
                    .insert(block, 100.0 * (base as f64 - value as f64) / base as f64);
            }
        }
    }
    out
}

fn export(
    source: &str,
    rows: &BTreeMap<String, BTreeMap<u32, f64>>,
    keep: impl Fn(u32) -> bool,
) -> Vec<u8> {
    let trials: Vec<_> = rows
        .iter()
        .flat_map(|(name, blocks)| {
            blocks.iter().filter(|(block, _)| keep(**block)).map(move |(_, value)| {
                serde_json::json!({"candidate": [name], "improvement": value, "correct": true})
            })
        })
        .collect();
    serde_json::to_vec(&serde_json::json!({"source": source, "trials": trials})).unwrap_or_default()
}

pub fn analyze(manifest: &Manifest, results: &Path) -> anyhow::Result<serde_json::Value> {
    let records: Vec<Record> = std::fs::read_to_string(results)
        .context("read the results")?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .context("a result record is malformed")?;
    let digest = manifest.digest();
    anyhow::ensure!(
        records
            .iter()
            .all(|r| r.manifest_sha256 == digest && r.study_id == manifest.study_id),
        "a record belongs to another manifest"
    );
    let rows = improvements(manifest, &records);
    let seed = u64::from_str_radix(&digest[..16], 16).unwrap_or(1);
    let threshold = manifest.practical_percent;
    let mut effects = serde_json::Map::new();
    for (name, blocks) in &rows {
        let values: Vec<f64> = blocks.values().copied().collect();
        let (low, high) = bootstrap(&values, seed);
        effects.insert(
            name.clone(),
            serde_json::json!({
                "blocks": values.len(), "mean_percent": mean(&values), "ci95_percent": [low, high],
                "verdict": verdict(low, high, threshold),
            }),
        );
    }
    // The ranker sees only the manifest's legal candidates; any other name is dropped on import.
    let space = ConfigSpace {
        dimensions: vec![Dimension {
            name: "placement".into(),
            values: manifest
                .candidates
                .iter()
                .map(|arm| arm.name.clone())
                .collect(),
        }],
        forbidden: vec![],
    };
    let split = |keep: &dyn Fn(u32) -> bool| -> anyhow::Result<Split> {
        if !rows.values().any(|blocks| blocks.keys().any(|b| keep(*b))) {
            return Ok((BTreeMap::new(), 0));
        }
        let imported = import_measurements(&export(&manifest.study_id, &rows, keep), &space)?;
        Ok((imported.dataset, imported.outside_space))
    };
    let (train_set, train_outside) = split(&|b| b <= manifest.training_blocks)?;
    let (holdout_set, holdout_outside) = split(&|b| b > manifest.training_blocks)?;
    let mean_of = |dataset: &BTreeMap<Vec<String>, Vec<f64>>, name: &str| {
        dataset.get(&vec![name.to_owned()]).map(|v| mean(v))
    };
    let favourite = manifest
        .candidates
        .iter()
        .filter_map(|arm| Some((arm.name.clone(), mean_of(&train_set, &arm.name)?)))
        .max_by(|a, b| a.1.total_cmp(&b.1));
    let ranking = held_out_ranking(&train_set, &holdout_set);
    Ok(serde_json::json!({
        "schema_version": 1, "study_id": manifest.study_id, "manifest_sha256": digest,
        "practical_percent": threshold,
        "valid_slots": records.iter().filter(|r| r.valid).count(),
        "invalid_slots": records.iter().filter(|r| !r.valid).count(),
        "effects": effects,
        "ranking": ranking.as_ref().map(|r| serde_json::json!({
            "candidates": r.candidates, "top1_agrees": r.top1_agrees,
            "holdout_regret": r.holdout_regret, "spearman": r.spearman,
        })),
        "ranking_note": if ranking.is_none() { "InsufficientLegalCandidateEvidence: fewer than three legal candidates with blocks in both splits" } else { "ranked among legal manifest candidates only" },
        "training_favourite": favourite.as_ref().map(|(name, value)| serde_json::json!({
            "candidate": name, "training_mean_percent": value,
            "holdout_mean_percent": mean_of(&holdout_set, name),
        })),
        "outside_space_dropped": train_outside + holdout_outside,
        "authorizes_mutation": false,
    }))
}
