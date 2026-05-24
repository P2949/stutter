use std::collections::BTreeMap;

use super::{RankingConfidence, TuneCandidateSummary, TuneProfileStats};

pub(super) fn select_best_profile(grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>) -> String {
    grouped
        .iter()
        .filter(|(_, runs)| runs.iter().any(|r| r.valid))
        .min_by_key(|(_, runs)| aggregate_profile_rank(runs))
        .map(|(name, _)| name.clone())
        .unwrap_or_default()
}

pub fn aggregate_profile_rank(runs: &[TuneCandidateSummary]) -> impl Ord {
    let invalid_run_count = runs.iter().filter(|r| !r.valid).count();
    let valid_runs: Vec<&TuneCandidateSummary> = runs.iter().filter(|r| r.valid).collect();

    let score_totals: Vec<u64> = valid_runs
        .iter()
        .map(|r| r.diagnostic_raw_score_total)
        .collect();
    let over_5ms: Vec<u64> = valid_runs.iter().map(|r| r.over_5ms).collect();
    let over_2ms: Vec<u64> = valid_runs.iter().map(|r| r.over_2ms).collect();
    let over_1ms: Vec<u64> = valid_runs.iter().map(|r| r.over_1ms).collect();
    let frame_over_50ms: Vec<u64> = valid_runs.iter().map(|r| r.frame_over_50ms).collect();
    let frame_over_33ms: Vec<u64> = valid_runs.iter().map(|r| r.frame_over_33ms).collect();
    let frame_over_16ms: Vec<u64> = valid_runs.iter().map(|r| r.frame_over_16ms).collect();
    let frame_p99s: Vec<u64> = valid_runs
        .iter()
        .map(|r| (r.frame_p99_ms * 1000.0) as u64)
        .collect();
    let frame_maxes: Vec<u64> = valid_runs
        .iter()
        .map(|r| (r.frame_max_ms * 1000.0) as u64)
        .collect();
    let max_latencies: Vec<u64> = valid_runs.iter().map(|r| r.max_latency_ns).collect();

    (
        invalid_run_count,
        median_u64(score_totals.clone()),
        median_u64(over_5ms),
        median_u64(over_2ms),
        median_u64(over_1ms),
        median_u64(frame_over_50ms),
        median_u64(frame_over_33ms),
        median_u64(frame_over_16ms),
        worst_u64(score_totals),
        worst_u64(frame_p99s),
        worst_u64(frame_maxes),
        worst_u64(max_latencies),
    )
}

pub fn percentile_nearest_rank_u64(values: &mut [u64], percentile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }

    values.sort_unstable();
    let percentile = percentile.clamp(0.0, 100.0);
    let rank = ((percentile / 100.0) * values.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(values.len() - 1);
    values[idx]
}

pub fn iqr_u64(values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }

    let mut q25_values = values.clone();
    let q25 = percentile_nearest_rank_u64(&mut q25_values, 25.0);
    let mut q75_values = values;
    let q75 = percentile_nearest_rank_u64(&mut q75_values, 75.0);
    q75.saturating_sub(q25)
}

pub fn profile_stats_from_grouped(
    grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
) -> Vec<TuneProfileStats> {
    grouped
        .iter()
        .map(|(profile, runs)| {
            let valid_runs = runs.iter().filter(|run| run.valid).collect::<Vec<_>>();
            let score_totals = valid_runs
                .iter()
                .map(|run| run.diagnostic_raw_score_total)
                .collect::<Vec<_>>();
            let over_5ms = valid_runs
                .iter()
                .map(|run| run.over_5ms)
                .collect::<Vec<_>>();
            let frame_p99_us = valid_runs
                .iter()
                .map(|run| (run.frame_p99_ms * 1000.0) as u64)
                .collect::<Vec<_>>();

            TuneProfileStats {
                profile: profile.clone(),
                valid_runs: valid_runs.len(),
                invalid_runs: runs.len().saturating_sub(valid_runs.len()),
                median_diagnostic_raw_score_total: median_u64(score_totals.clone()),
                iqr_diagnostic_raw_score_total: iqr_u64(score_totals.clone()),
                worst_diagnostic_raw_score_total: worst_u64(score_totals),
                median_over_5ms: median_u64(over_5ms.clone()),
                iqr_over_5ms: iqr_u64(over_5ms),
                median_frame_p99_us: median_u64(frame_p99_us.clone()),
                iqr_frame_p99_us: iqr_u64(frame_p99_us),
            }
        })
        .collect()
}

pub fn assess_ranking_confidence(
    profile_stats: &[TuneProfileStats],
    _grouped: &BTreeMap<String, Vec<TuneCandidateSummary>>,
    best_profile: &str,
    runs: u32,
) -> (RankingConfidence, Vec<String>) {
    let mut notes = Vec::new();
    let mut valid_stats = profile_stats
        .iter()
        .filter(|stat| stat.valid_runs > 0)
        .collect::<Vec<_>>();
    valid_stats.sort_by_key(|stat| stat.median_diagnostic_raw_score_total);

    if valid_stats.len() < 2 {
        notes.push("fewer than two profiles produced valid runs".to_owned());
        return (RankingConfidence::Unstable, notes);
    }

    let Some(best) = valid_stats
        .iter()
        .copied()
        .find(|stat| stat.profile == best_profile)
    else {
        notes.push("best profile did not produce valid runs".to_owned());
        return (RankingConfidence::Unstable, notes);
    };

    if runs >= 3 && best.valid_runs < 2 {
        notes.push("best profile has fewer than two valid runs".to_owned());
        return (RankingConfidence::Unstable, notes);
    }

    let Some(second) = valid_stats
        .iter()
        .copied()
        .find(|stat| stat.profile != best.profile)
    else {
        notes.push("no second valid profile is available for comparison".to_owned());
        return (RankingConfidence::Unstable, notes);
    };

    let diff = second
        .median_diagnostic_raw_score_total
        .abs_diff(best.median_diagnostic_raw_score_total);
    let max_iqr = best
        .iqr_diagnostic_raw_score_total
        .max(second.iqr_diagnostic_raw_score_total);
    if diff <= max_iqr && max_iqr > 0 {
        notes.push(format!(
            "best and second-best median scores are close relative to variance (diff={diff}, max_iqr={max_iqr})"
        ));
        return (RankingConfidence::Unstable, notes);
    }

    let five_percent_second = second.median_diagnostic_raw_score_total / 20;
    let close_to_second = diff <= five_percent_second;

    if runs < 3 {
        notes.push("--runs is less than 3; ranking confidence is limited".to_owned());
    }
    if best.invalid_runs > 0 {
        notes.push("best profile has invalid runs".to_owned());
    }
    if close_to_second {
        notes.push(format!(
            "best median score is within 5% of second-best (diff={diff})"
        ));
    }
    if best.iqr_diagnostic_raw_score_total > 0 {
        notes.push("best profile score IQR is non-zero".to_owned());
    }

    if runs < 3 || best.invalid_runs > 0 || close_to_second {
        (RankingConfidence::Low, notes)
    } else if best.iqr_diagnostic_raw_score_total > 0
        || second.iqr_diagnostic_raw_score_total > 0
        || !notes.is_empty()
    {
        (RankingConfidence::Medium, notes)
    } else {
        (RankingConfidence::High, notes)
    }
}

pub fn median_u64(mut values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

pub fn worst_u64(values: Vec<u64>) -> u64 {
    values.into_iter().max().unwrap_or(0)
}
