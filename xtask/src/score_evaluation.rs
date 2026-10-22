//! Empirical evaluation of `StutterScore` as a *ranking proxy within comparable
//! experiments* (never a human-perceived percentage). Evidence: score versus controlled
//! scheduler degradation level, and agreement with the direct runnable-latency metrics.
//! Weights are retained unless the data shows the proxy is not monotonic; semantic
//! objectives keep their own primaries regardless of any score change.
use anyhow::{Context, ensure};
use serde_json::{Value, json};
use stutter::api::{
    autotune::runnable_signals_for_target, recorder::IntervalRecord,
    scorer::score_from_interval_records,
};

#[derive(Clone, Debug, PartialEq)]
pub struct LevelObservation {
    pub level_ms: f64,
    pub score_total: f64,
    pub over_1ms: f64,
    pub over_2ms: f64,
    pub over_5ms: f64,
    pub runnable_p99_ns: Option<f64>,
    pub runnable_over_5ms_rate: Option<f64>,
}

/// Production score and runnable metrics for one controlled-degradation recording.
pub fn observe(
    level_ms: f64,
    records: &[IntervalRecord],
    pid: u32,
    ticks: u64,
) -> LevelObservation {
    let target = records
        .iter()
        .filter(|r| r.process_pid == Some(pid))
        .cloned()
        .collect::<Vec<_>>();
    let score = score_from_interval_records(&target);
    let signals = runnable_signals_for_target(records, pid, ticks);
    LevelObservation {
        level_ms,
        score_total: score.total as f64,
        over_1ms: score.over_1ms as f64,
        over_2ms: score.over_2ms as f64,
        over_5ms: score.over_5ms as f64,
        runnable_p99_ns: signals.runnable_latency_p99_ns.map(|v| v as f64),
        runnable_over_5ms_rate: signals
            .runnable_over_5ms
            .zip(signals.runnable_samples.filter(|s| *s > 0))
            .map(|(over, samples)| over as f64 / samples as f64),
    }
}

/// Average ranks (ties share the mean rank).
fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order = (0..values.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start;
        while end + 1 < order.len() && values[order[end + 1]] == values[order[start]] {
            end += 1;
        }
        let rank = (start + end) as f64 / 2.0 + 1.0;
        for index in &order[start..=end] {
            ranks[*index] = rank;
        }
        start = end + 1;
    }
    ranks
}

/// Spearman rank correlation; `None` when either series is constant or too short.
pub fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 3 {
        return None;
    }
    let (rx, ry) = (ranks(x), ranks(y));
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let (mx, my) = (mean(&rx), mean(&ry));
    let cov = rx
        .iter()
        .zip(&ry)
        .map(|(a, b)| (a - mx) * (b - my))
        .sum::<f64>();
    let var = |v: &[f64], m: f64| v.iter().map(|a| (a - m).powi(2)).sum::<f64>();
    let denom = (var(&rx, mx) * var(&ry, my)).sqrt();
    (denom > 0.0).then(|| cov / denom)
}

pub fn evaluate(mut observations: Vec<LevelObservation>) -> anyhow::Result<Value> {
    ensure!(
        observations.len() >= 4,
        "need at least four degradation levels, have {}",
        observations.len()
    );
    observations.sort_by(|a, b| a.level_ms.total_cmp(&b.level_ms));
    ensure!(
        observations[0].level_ms == 0.0,
        "the zero-degradation control level is required"
    );
    let levels = observations.iter().map(|o| o.level_ms).collect::<Vec<_>>();
    let scores = observations
        .iter()
        .map(|o| o.score_total)
        .collect::<Vec<_>>();
    let inversions = scores.windows(2).filter(|w| w[1] < w[0]).count();
    let column = |select: fn(&LevelObservation) -> Option<f64>| -> Option<Vec<f64>> {
        observations.iter().map(select).collect()
    };
    let against = |name: &str, series: Option<Vec<f64>>| {
        (name.to_owned(), series.and_then(|s| spearman(&scores, &s)))
    };
    let correlations = [
        against("runnable_p99", column(|o| o.runnable_p99_ns)),
        against(
            "runnable_over_5ms_rate",
            column(|o| o.runnable_over_5ms_rate),
        ),
        against(
            "sample_over_1ms",
            Some(observations.iter().map(|o| o.over_1ms).collect()),
        ),
    ];
    let level_rho = spearman(&levels, &scores);
    let monotonic = inversions == 0 && level_rho.is_some_and(|rho| rho >= 0.9);
    Ok(json!({
        "schema_version": 1,
        "interpretation": "ranking proxy within comparable experiments; not a human-perceived percentage",
        "levels_ms": levels,
        "score_total": scores,
        "adjacent_inversions": inversions,
        "spearman_score_vs_level": level_rho,
        "spearman_score_vs": correlations.iter().map(|(n, v)| (n.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
        "monotonic_in_degradation": monotonic,
        "weights": "retained (100 x >5 ms, 20 x >2 ms, 1 x >1 ms; frame terms unchanged)",
        "weights_decision": if monotonic { "retain: no empirical case for change" } else { "review: score is not monotonic in controlled degradation" },
        "primaries_unchanged": ["GameRunnableLatency: runnable latency is primary", "CompileThroughputWithForegroundProtection: fixed-work elapsed time is primary"],
        "not_available": ["frame p99, p99.9, >16.6 ms and >33 ms rates: the synthetic workload emits no frames"],
    }))
}

pub fn run_from_recordings(levels: &[(f64, std::path::PathBuf)]) -> anyhow::Result<Value> {
    let mut observations = Vec::new();
    for (level, dir) in levels {
        let records = std::fs::read_to_string(dir.join("interval.json"))?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<IntervalRecord>)
            .collect::<Result<Vec<_>, _>>()?;
        let (pid, ticks) = target_identity(dir, &records)?;
        observations.push(observe(*level, &records, pid, ticks));
    }
    evaluate(observations)
}

/// The game process with the most samples, with its start time from the session.
fn target_identity(
    dir: &std::path::Path,
    records: &[IntervalRecord],
) -> anyhow::Result<(u32, u64)> {
    let mut samples = std::collections::BTreeMap::<u32, u64>::new();
    for record in records {
        if let Some(pid) = record.process_pid {
            *samples.entry(pid).or_default() += record.samples;
        }
    }
    let pid = samples
        .into_iter()
        .max_by_key(|(_, s)| *s)
        .map(|(pid, _)| pid)
        .context("no sampled process")?;
    let session: stutter::api::recorder::SessionFile =
        serde_json::from_slice(&std::fs::read(dir.join("session.json"))?)?;
    let ticks = session
        .tasks
        .iter()
        .find(|task| task.process_pid == Some(pid))
        .and_then(|task| task.process_starttime_ticks)
        .context("target start time missing from session")?;
    Ok((pid, ticks))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(level: f64, score: f64, p99: f64) -> LevelObservation {
        LevelObservation {
            level_ms: level,
            score_total: score,
            over_1ms: score,
            over_2ms: 0.0,
            over_5ms: 0.0,
            runnable_p99_ns: Some(p99),
            runnable_over_5ms_rate: Some(score / 1e4),
        }
    }

    #[test]
    fn spearman_handles_ties_constants_and_direction() {
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]), Some(1.0));
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]), Some(-1.0));
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[5.0, 5.0, 5.0]), None);
        assert!(spearman(&[1.0, 2.0], &[1.0, 2.0]).is_none());
        let tied = spearman(&[1.0, 2.0, 3.0, 4.0], &[1.0, 1.0, 2.0, 3.0]).unwrap();
        assert!(tied > 0.9 && tied < 1.0);
    }

    #[test]
    fn monotonic_scores_retain_weights_and_inversions_force_review() {
        let good = (0..6)
            .map(|i| {
                obs(
                    [0.0, 1.0, 2.0, 4.0, 8.0, 16.0][i],
                    i as f64 * 10.0,
                    1e6 * (i + 1) as f64,
                )
            })
            .collect();
        let report = evaluate(good).unwrap();
        assert_eq!(report["monotonic_in_degradation"], true);
        assert!(
            report["weights_decision"]
                .as_str()
                .unwrap()
                .starts_with("retain")
        );
        let bad = [0.0, 1.0, 2.0, 4.0, 8.0, 16.0]
            .iter()
            .zip([0.0, 30.0, 20.0, 40.0, 50.0, 60.0])
            .map(|(l, s)| obs(*l, s, 1e6))
            .collect();
        let report = evaluate(bad).unwrap();
        assert_eq!(report["monotonic_in_degradation"], false);
        assert_eq!(report["adjacent_inversions"], 1);
        assert!(
            report["weights_decision"]
                .as_str()
                .unwrap()
                .starts_with("review")
        );
    }

    #[test]
    fn requires_four_levels_including_the_zero_control() {
        assert!(evaluate(vec![obs(0.0, 0.0, 1.0), obs(1.0, 1.0, 1.0)]).is_err());
        assert!(evaluate((1..=5).map(|i| obs(i as f64, i as f64, 1.0)).collect()).is_err());
    }

    #[test]
    fn semantic_objective_primaries_are_not_the_generic_score() {
        use stutter::api::autotune::ObjectiveKind;
        let runnable = ObjectiveKind::GameRunnableLatency
            .descriptor()
            .primary_metric;
        assert!(runnable.to_lowercase().contains("runnable"), "{runnable}");
        let compile = ObjectiveKind::CompileThroughputWithForegroundProtection
            .descriptor()
            .primary_metric;
        assert!(
            compile.to_lowercase().contains("elapsed") || compile.to_lowercase().contains("fixed"),
            "{compile}"
        );
    }
}
