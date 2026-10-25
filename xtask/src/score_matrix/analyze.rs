//! What the matrix answers: does one scalar score preserve the ordering of the direct evidence
//! across distinct workloads? Per profile the per-level medians are evaluated with the existing
//! evaluation (inversions, Spearman against level and against each direct metric); pooled
//! statistics run over every recording. Nothing here changes a weight.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::model::Profile;
use crate::score_evaluation::{LevelObservation, evaluate, spearman};

/// The retained measurements of one recording.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Measured {
    pub level_ms: f64,
    pub score_total: f64,
    pub runnable_score_total: f64,
    /// The same score when the explicitly named target's generic-class tasks count as the
    /// target (the rule the diagnosis uses); the recorded classes leave a non-game target's
    /// runnable terms empty.
    #[serde(default)]
    pub explicit_target_score_total: f64,
    #[serde(default)]
    pub explicit_target_runnable_score_total: f64,
    pub over_1ms: f64,
    pub over_2ms: f64,
    pub over_5ms: f64,
    pub runnable_p99_ns: Option<f64>,
    pub runnable_over_2ms_rate: Option<f64>,
    pub runnable_over_5ms_rate: Option<f64>,
    pub frame_p99_ms: Option<f64>,
    pub frame_p99_9_ms: Option<f64>,
    pub frame_over_16_6_rate: Option<f64>,
    pub frame_over_33_3_rate: Option<f64>,
    pub frame_samples: Option<usize>,
}

pub fn median(mut values: Vec<f64>) -> Option<f64> {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    })
}

fn med(runs: &[&Measured], f: impl Fn(&Measured) -> Option<f64>) -> Option<f64> {
    median(runs.iter().filter_map(|r| f(r)).collect())
}

/// The per-level median recording of one profile as an evaluable observation.
fn level_median(level: f64, runs: &[&Measured]) -> LevelObservation {
    let req = |f: fn(&Measured) -> f64| med(runs, |m| Some(f(m))).unwrap_or(f64::NAN);
    LevelObservation {
        level_ms: level,
        score_total: req(|m| m.score_total),
        runnable_score_total: req(|m| m.runnable_score_total),
        over_1ms: req(|m| m.over_1ms),
        over_2ms: req(|m| m.over_2ms),
        over_5ms: req(|m| m.over_5ms),
        runnable_p99_ns: med(runs, |m| m.runnable_p99_ns),
        runnable_over_5ms_rate: med(runs, |m| m.runnable_over_5ms_rate),
        frame_p99_ms: med(runs, |m| m.frame_p99_ms),
        frame_p99_9_ms: med(runs, |m| m.frame_p99_9_ms),
        frame_over_16_6_rate: med(runs, |m| m.frame_over_16_6_rate),
        frame_over_33_3_rate: med(runs, |m| m.frame_over_33_3_rate),
        frame_samples: med(runs, |m| m.frame_samples.map(|s| s as f64)).map(|s| s as usize),
        identity: None,
    }
}

fn rho(x: &[f64], y: &[f64]) -> Value {
    json!(spearman(x, y))
}

/// Pooled Spearman of the score and of the level against the direct metrics, over `runs`.
fn pooled(runs: &[&Measured]) -> Value {
    let col = |f: fn(&Measured) -> Option<f64>| -> Option<(Vec<f64>, Vec<f64>, Vec<f64>)> {
        let rows: Vec<_> = runs
            .iter()
            .filter_map(|m| Some((m.level_ms, m.score_total, f(m)?)))
            .collect();
        (rows.len() == runs.len()).then(|| {
            (
                rows.iter().map(|r| r.0).collect(),
                rows.iter().map(|r| r.1).collect(),
                rows.iter().map(|r| r.2).collect(),
            )
        })
    };
    let mut out = serde_json::Map::new();
    let level: Vec<f64> = runs.iter().map(|m| m.level_ms).collect();
    let score: Vec<f64> = runs.iter().map(|m| m.score_total).collect();
    out.insert("score_vs_level".into(), rho(&score, &level));
    for (name, select) in [
        (
            "runnable_p99",
            (|m: &Measured| m.runnable_p99_ns) as fn(&Measured) -> Option<f64>,
        ),
        ("frame_p99", |m| m.frame_p99_ms),
        ("frame_p99_9", |m| m.frame_p99_9_ms),
    ] {
        let value = col(select).map_or(Value::Null, |(lv, sc, direct)| {
            json!({"score_vs_direct": spearman(&sc, &direct), "direct_vs_level": spearman(&direct, &lv)})
        });
        out.insert(name.to_owned(), value);
    }
    Value::Object(out)
}

/// Both readings of the score, side by side: as recorded, and with the explicit target counted.
pub fn analyze(by_profile: &BTreeMap<Profile, Vec<Measured>>) -> Value {
    let explicit: BTreeMap<Profile, Vec<Measured>> = by_profile
        .iter()
        .map(|(profile, runs)| {
            let mapped = runs
                .iter()
                .map(|m| Measured {
                    score_total: m.explicit_target_score_total,
                    runnable_score_total: m.explicit_target_runnable_score_total,
                    ..m.clone()
                })
                .collect();
            (*profile, mapped)
        })
        .collect();
    json!({
        "as_recorded": analyze_variant(by_profile),
        "explicit_target": analyze_variant(&explicit),
        "note": "as_recorded is today's behavior (only game-class tasks contribute to the runnable terms); explicit_target counts the named process's generic-class tasks. Weights are identical in both.",
    })
}

/// One reading: per profile and pooled.
fn analyze_variant(by_profile: &BTreeMap<Profile, Vec<Measured>>) -> Value {
    let mut profiles = serde_json::Map::new();
    let mut every: Vec<&Measured> = Vec::new();
    let mut inversions = 0;
    let mut all_monotonic = true;
    for (profile, runs) in by_profile {
        every.extend(runs.iter());
        let mut levels: BTreeMap<u64, Vec<&Measured>> = BTreeMap::new();
        for run in runs {
            levels.entry(run.level_ms as u64).or_default().push(run);
        }
        let medians: Vec<LevelObservation> = levels
            .iter()
            .map(|(level, group)| level_median(*level as f64, group))
            .collect();
        let evaluation =
            evaluate(medians, true).unwrap_or_else(|e| json!({"error": format!("{e:#}")}));
        inversions += evaluation["adjacent_inversions"].as_u64().unwrap_or(0);
        all_monotonic &= evaluation["monotonic_in_degradation"] == true;
        profiles.insert(
            profile.id().to_owned(),
            json!({
                "recordings": runs.len(),
                "per_level": levels.iter().map(|(level, group)| json!({
                    "level_ms": level,
                    "recordings": group.len(),
                    "score_total_median": med(group, |m| Some(m.score_total)),
                    "score_total_values": group.iter().map(|m| m.score_total).collect::<Vec<_>>(),
                    "runnable_p99_ns_median": med(group, |m| m.runnable_p99_ns),
                    "runnable_over_2ms_rate_median": med(group, |m| m.runnable_over_2ms_rate),
                    "runnable_over_5ms_rate_median": med(group, |m| m.runnable_over_5ms_rate),
                    "frame_p99_ms_median": med(group, |m| m.frame_p99_ms),
                    "frame_p99_9_ms_median": med(group, |m| m.frame_p99_9_ms),
                    "frame_over_16_6_rate_median": med(group, |m| m.frame_over_16_6_rate),
                    "frame_over_33_3_rate_median": med(group, |m| m.frame_over_33_3_rate),
                })).collect::<Vec<_>>(),
                "evaluation": evaluation,
                "within_profile_recordings": pooled(&runs.iter().collect::<Vec<_>>()),
            }),
        );
    }
    json!({
        "schema_version": 1,
        "question": "Does one scalar score reliably preserve the ordering of the direct evidence across distinct workloads?",
        "profiles": profiles,
        "pooled": pooled(&every),
        "total_adjacent_inversions": inversions,
        "every_profile_monotonic": all_monotonic,
        "weights": "unchanged; this study decides nothing about weights",
        "decision": if all_monotonic {
            "the score preserved the ordering in every profile; weights stay and the evidence is broader than v0.1"
        } else {
            "the score does not reliably preserve the ordering across these workloads: weights stay, tune_evaluation stays Experimental, and a score redesign or objective-specific scores is a future design issue"
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(level: f64, score: f64, p99: f64) -> Measured {
        Measured {
            level_ms: level,
            score_total: score,
            runnable_score_total: score,
            runnable_p99_ns: Some(p99),
            frame_p99_ms: Some(p99 / 1e6),
            frame_p99_9_ms: Some(p99 / 1e6),
            frame_over_16_6_rate: Some(score / 1e4),
            frame_over_33_3_rate: Some(score / 1e5),
            runnable_over_5ms_rate: Some(score / 1e4),
            frame_samples: Some(100),
            ..Measured::default()
        }
    }

    #[test]
    fn a_monotonic_profile_is_reported_as_preserving_the_ordering() {
        let mut data = BTreeMap::new();
        data.insert(
            Profile::VkmarkLight,
            [0.0, 1.0, 2.0, 4.0, 8.0, 16.0]
                .iter()
                .flat_map(|l| (0..3).map(move |r| run(*l, l * 10.0 + f64::from(r), l * 1e6 + 1.0)))
                .collect::<Vec<_>>(),
        );
        let result = analyze_variant(&data);
        assert_eq!(result["every_profile_monotonic"], true);
        assert_eq!(result["total_adjacent_inversions"], 0);
        assert!(result["pooled"]["score_vs_level"].as_f64().unwrap() > 0.9);
    }

    #[test]
    fn an_inversion_in_any_profile_is_reported_and_leaves_the_weights_alone() {
        let mut data = BTreeMap::new();
        data.insert(
            Profile::VkcubeLight,
            [
                (0.0, 5.0),
                (1.0, 10.0),
                (2.0, 4.0),
                (4.0, 40.0),
                (8.0, 80.0),
                (16.0, 160.0),
            ]
            .iter()
            .map(|(l, s)| run(*l, *s, l * 1e6 + 1.0))
            .collect::<Vec<_>>(),
        );
        let result = analyze_variant(&data);
        assert_eq!(result["every_profile_monotonic"], false);
        assert!(result["total_adjacent_inversions"].as_u64().unwrap() >= 1);
        assert!(
            result["decision"]
                .as_str()
                .unwrap()
                .contains("weights stay")
        );
    }

    #[test]
    fn the_median_ignores_non_finite_values() {
        assert_eq!(median(vec![3.0, f64::NAN, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(vec![]), None);
    }
}
