use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::model::{
    BaselineTuneConfidenceMetadata, BaselineTuneRecommendation, BaselineTuneRecommendationOptions,
};
use crate::{
    artifacts::ArtifactSelection,
    scorer::{self, StutterScore},
    session_io,
    tune::{
        self, RankingConfidence, TuneCandidateSummary, TuneSummary,
        recommendation::TuneRecommendationVerdict,
        statistics::{self, FormalMetricComparison, MIN_FORMAL_SAMPLES_PER_SIDE},
    },
};

const BASELINE_MIN_SCORED_SAMPLES: u64 = 50;
const LOW_BASELINE_SCORE_TOTAL: u64 = 100;

struct BaselineSample {
    run_dir: PathBuf,
    scenario_name: Option<String>,
    scenario_hash: Option<String>,
    workload_label: Option<String>,
    route_label: Option<String>,
    score: StutterScore,
    scored_samples: u64,
    intervals_empty: bool,
    valid: bool,
}

#[cfg(test)]
pub fn build_baseline_tune_recommendation(
    baseline_run: &Path,
    tune_dir: &Path,
) -> anyhow::Result<BaselineTuneRecommendation> {
    build_baseline_tune_recommendation_for_baselines(&[baseline_run.to_path_buf()], tune_dir)
}

#[cfg(test)]
pub fn build_baseline_tune_recommendation_for_baselines(
    baseline_runs: &[PathBuf],
    tune_dir: &Path,
) -> anyhow::Result<BaselineTuneRecommendation> {
    build_baseline_tune_recommendation_for_baselines_with_options(
        baseline_runs,
        tune_dir,
        BaselineTuneRecommendationOptions::default(),
    )
}

pub fn build_baseline_tune_recommendation_for_baselines_with_options(
    baseline_runs: &[PathBuf],
    tune_dir: &Path,
    options: BaselineTuneRecommendationOptions,
) -> anyhow::Result<BaselineTuneRecommendation> {
    if baseline_runs.is_empty() {
        anyhow::bail!("at least one --baseline run is required");
    }

    let summary_path = tune_dir.join("tuning_summary.json");
    if !summary_path.exists() {
        anyhow::bail!(
            "missing tuning summary: expected {}",
            summary_path.display()
        );
    }
    let summary: TuneSummary = serde_json::from_slice(
        &fs::read(&summary_path)
            .with_context(|| format!("failed to read {}", summary_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", summary_path.display()))?;

    let mut warnings = Vec::new();
    warnings.extend(summary.ranking_notes.iter().cloned());
    warnings.extend(
        summary
            .comparability_warnings
            .iter()
            .map(|warning| match &warning.profile {
                Some(profile) => format!(
                    "comparability {:?} profile={} kind={}: {}",
                    warning.severity, profile, warning.kind, warning.message
                ),
                None => format!(
                    "comparability {:?} kind={}: {}",
                    warning.severity, warning.kind, warning.message
                ),
            }),
    );

    let baseline_samples = load_baseline_samples(baseline_runs, &mut warnings)?;
    push_scenario_identity_warnings(&baseline_samples, &summary, &mut warnings, &options);
    let valid_baselines = baseline_samples
        .iter()
        .filter(|sample| sample.valid)
        .collect::<Vec<_>>();
    let baseline_valid_runs = valid_baselines.len();
    let baseline_invalid_runs = baseline_samples.len().saturating_sub(baseline_valid_runs);

    if baseline_valid_runs == 0 {
        warnings.push("no valid baseline run was available for comparison".to_owned());
    }
    if baseline_invalid_runs > 0 {
        warnings.push(format!(
            "{} baseline run(s) were invalid and excluded from formal statistics",
            baseline_invalid_runs
        ));
    }

    let baseline_score_total =
        median_u64_from_f64(&baseline_metric_values(&valid_baselines, |sample| {
            sample.score.total as f64
        }));
    let baseline_over_5ms =
        median_u64_from_f64(&baseline_metric_values(&valid_baselines, |sample| {
            sample.score.over_5ms as f64
        }));
    let baseline_frame_p99_ms =
        optional_median_f64(&baseline_metric_values(&valid_baselines, |sample| {
            sample.score.frame_p99_ms
        }));

    let best_profile = if summary.best_profile.is_empty() {
        None
    } else {
        Some(summary.best_profile.clone())
    };
    let best_stat = best_profile.as_deref().and_then(|profile| {
        summary
            .profile_stats
            .iter()
            .find(|stat| stat.profile == profile && stat.valid_runs > 0)
    });
    let best_median_diagnostic_raw_score_total =
        best_stat.map(|stat| stat.median_diagnostic_raw_score_total);
    let score_delta_abs = best_median_diagnostic_raw_score_total
        .map(|best| baseline_score_total as i64 - best as i64);
    let score_delta_percent = score_delta_abs.and_then(|delta| {
        (baseline_score_total > 0).then_some(delta as f64 / baseline_score_total as f64 * 100.0)
    });

    let best_median_over_5ms = best_stat.map(|stat| stat.median_over_5ms);
    let over_5ms_delta_abs =
        best_median_over_5ms.map(|best| baseline_over_5ms as i64 - best as i64);

    let best_median_frame_p99_ms = best_stat.and_then(|stat| {
        (stat.median_frame_p99_us > 0).then_some(stat.median_frame_p99_us as f64 / 1000.0)
    });

    let frame_p99_delta_ms = match (baseline_frame_p99_ms, best_median_frame_p99_ms) {
        (Some(base), Some(best)) => Some(best - base),
        _ => None,
    };

    let best_candidates = best_profile
        .as_deref()
        .map(|profile| valid_candidates_for_profile(&summary, profile))
        .unwrap_or_default();

    if let Some(best_profile) = &best_profile {
        let all_best_candidates = summary
            .candidates
            .iter()
            .filter(|candidate| candidate.profile == *best_profile)
            .collect::<Vec<_>>();
        if all_best_candidates.is_empty() {
            warnings.push(format!(
                "best profile '{best_profile}' has no candidate runs"
            ));
        } else if all_best_candidates.iter().any(|candidate| !candidate.valid) {
            warnings.push(format!(
                "best profile '{best_profile}' has invalid candidate runs"
            ));
        }
    }

    let formal_metrics = formal_metric_comparisons(&valid_baselines, &best_candidates);
    extend_formal_metric_warnings(&mut warnings, &formal_metrics);
    let formal_score_metric = formal_metrics
        .iter()
        .find(|metric| metric.metric == "diagnostic_raw_score_total");
    let formal_score_blocks_recommendation = formal_score_metric.is_none_or(|metric| {
        !metric.enough_samples
            || !metric.statistically_significant
            || metric.improvement_delta <= 0.0
    });

    let score_effect_size =
        formal_metric_effect_size(&formal_metrics, "diagnostic_raw_score_total");
    let score_noise_ratio =
        metric_noise_ratio(&tuned_metric_values(&best_candidates, |candidate| {
            candidate.diagnostic_raw_score_total as f64
        }));
    let over_5ms_effect_size = formal_metric_effect_size(&formal_metrics, "over_5ms");
    let over_5ms_noise_ratio =
        metric_noise_ratio(&tuned_metric_values(&best_candidates, |candidate| {
            candidate.over_5ms as f64
        }));
    let frame_p99_effect_size = formal_metric_effect_size(&formal_metrics, "frame_p99_ms");
    let frame_p99_noise_ratio = metric_noise_ratio(
        &tuned_metric_values(&best_candidates, |candidate| candidate.frame_p99_ms)
            .into_iter()
            .filter(|value| *value > 0.0)
            .collect::<Vec<_>>(),
    );

    let confidence_metadata = BaselineTuneConfidenceMetadata {
        ranking_confidence: summary.ranking_confidence,
        ranking_notes: summary.ranking_notes.clone(),
        tune_runs: summary.runs,
        tune_epoch_seconds: summary.epoch_seconds,
        tune_warmup_seconds: summary.warmup_seconds,
        best_profile_valid_runs: best_stat.map(|stat| stat.valid_runs),
        best_profile_invalid_runs: best_stat.map(|stat| stat.invalid_runs),
        baseline_valid_runs,
        baseline_invalid_runs,
    };

    let baseline_quality_blocks_recommendation =
        valid_baselines.iter().any(|sample| sample.intervals_empty)
            || valid_baselines
                .iter()
                .any(|sample| sample.scored_samples < BASELINE_MIN_SCORED_SAMPLES)
            || baseline_valid_runs == 0;

    let baseline_score_blocks_recommendation = if baseline_score_total == 0 {
        warnings.push("baseline score is zero; score improvement cannot be measured".to_owned());
        true
    } else if baseline_score_total < LOW_BASELINE_SCORE_TOTAL {
        warnings.push(format!(
            "baseline score is very low ({}); recommendation requires retest",
            baseline_score_total
        ));
        true
    } else {
        false
    };

    let verdict = if summary.ranking_confidence == RankingConfidence::Unstable {
        TuneRecommendationVerdict::NoRecommendation
    } else if let Some(best) = best_median_diagnostic_raw_score_total {
        let worse_threshold = baseline_score_total.saturating_add(baseline_score_total / 20);
        let improvement_threshold = baseline_score_total / 20;
        let improvement = baseline_score_total.saturating_sub(best);
        if best > worse_threshold {
            TuneRecommendationVerdict::NoRecommendation
        } else if baseline_quality_blocks_recommendation
            || baseline_score_blocks_recommendation
            || formal_score_blocks_recommendation
            || improvement < improvement_threshold
        {
            TuneRecommendationVerdict::NeedsRetest
        } else if matches!(
            summary.ranking_confidence,
            RankingConfidence::Medium | RankingConfidence::High
        ) {
            TuneRecommendationVerdict::Recommended
        } else {
            TuneRecommendationVerdict::NeedsRetest
        }
    } else {
        TuneRecommendationVerdict::NoRecommendation
    };

    let summary_text = match verdict {
        TuneRecommendationVerdict::Recommended => format!(
            "Best tuned profile '{}' improved the repeated baseline by {} score point(s) with a bootstrap CI that excludes zero.",
            best_profile.as_deref().unwrap_or("unknown"),
            score_delta_abs.unwrap_or(0)
        ),
        TuneRecommendationVerdict::NeedsRetest => format!(
            "Best tuned profile '{}' needs more comparable baseline/tune samples before it is trusted.",
            best_profile.as_deref().unwrap_or("unknown")
        ),
        TuneRecommendationVerdict::NoRecommendation => {
            if summary.ranking_confidence == RankingConfidence::Unstable {
                "No recommendation: tune ranking was unstable.".to_owned()
            } else if let Some(best) = best_median_diagnostic_raw_score_total {
                if best > baseline_score_total {
                    "No recommendation: the best tuned candidate failed to beat baseline."
                        .to_owned()
                } else {
                    "No recommendation: no trustworthy tuned winner was available.".to_owned()
                }
            } else {
                "No recommendation: tuning summary had no valid best profile.".to_owned()
            }
        }
    };

    let mut next_steps = Vec::new();
    match verdict {
        TuneRecommendationVerdict::Recommended => {
            next_steps
                .push("Review tuning_recommendation.md before applying the profile.".to_owned());
            next_steps.push("Apply manually and keep stutter restore available.".to_owned());
        }
        TuneRecommendationVerdict::NeedsRetest => {
            next_steps.push(format!(
                "Collect at least {MIN_FORMAL_SAMPLES_PER_SIDE} baseline runs and {MIN_FORMAL_SAMPLES_PER_SIDE} tune runs on the same route, scene, and load."
            ));
            next_steps.push(
                "Treat the current result as directional only if any formal A/B metric says enough_samples=false or its CI crosses zero."
                    .to_owned(),
            );
        }
        TuneRecommendationVerdict::NoRecommendation => {
            next_steps.push("Do not apply a tuned profile from this result.".to_owned());
            next_steps.push(
                "Collect another baseline and tuning run if the workload was not stable."
                    .to_owned(),
            );
        }
    }

    Ok(BaselineTuneRecommendation {
        schema_version: 5,
        baseline_run: baseline_samples[0].run_dir.clone(),
        baseline_runs: baseline_samples
            .iter()
            .map(|sample| sample.run_dir.clone())
            .collect(),
        tune_dir: tune_dir.to_path_buf(),
        scenario_name: summary.scenario_name.clone(),
        scenario_hash: summary.scenario_hash.clone(),
        workload_label: summary.workload_label.clone(),
        route_label: summary.route_label.clone(),
        best_profile,
        confidence: summary.ranking_confidence,
        confidence_metadata: Some(confidence_metadata),
        verdict,
        summary: summary_text,
        baseline_valid_runs,
        baseline_invalid_runs,
        diagnostic_baseline_raw_score_total: baseline_score_total,
        best_median_diagnostic_raw_score_total,
        score_delta_abs,
        score_delta_percent,
        score_effect_size,
        score_noise_ratio,
        baseline_over_5ms,
        best_median_over_5ms,
        over_5ms_delta_abs,
        over_5ms_effect_size,
        over_5ms_noise_ratio,
        baseline_frame_p99_ms,
        best_median_frame_p99_ms,
        frame_p99_delta_ms,
        frame_p99_effect_size,
        frame_p99_noise_ratio,
        formal_metrics,
        warnings,
        next_steps,
    })
}

fn load_baseline_samples(
    baseline_runs: &[PathBuf],
    warnings: &mut Vec<String>,
) -> anyhow::Result<Vec<BaselineSample>> {
    let mut samples = Vec::with_capacity(baseline_runs.len());
    for run_dir in baseline_runs {
        let baseline = session_io::load_run_artifacts(run_dir, ArtifactSelection::tune())
            .with_context(|| format!("failed to load baseline run {}", run_dir.display()))?;
        warnings.extend(
            baseline
                .validation
                .errors
                .iter()
                .map(|warning| format!("baseline {}: {warning}", run_dir.display())),
        );
        warnings.extend(
            baseline
                .validation
                .warnings
                .iter()
                .map(|warning| format!("baseline {}: {warning}", run_dir.display())),
        );
        warnings.extend(
            baseline
                .validation
                .missing_optional_files
                .iter()
                .map(|file| {
                    format!(
                        "baseline {} missing optional artifact: {file}",
                        run_dir.display()
                    )
                }),
        );

        let score = scorer::score_from_interval_records_and_frames(
            &baseline.intervals,
            &baseline.frame_events,
        );
        let (_, scored_samples) = tune::tune_scored_record_counts(&baseline.intervals);
        let intervals_empty = baseline.intervals.is_empty();
        let valid = !intervals_empty && scored_samples >= BASELINE_MIN_SCORED_SAMPLES;
        if intervals_empty {
            warnings.push(format!(
                "baseline {} intervals are missing",
                run_dir.display()
            ));
        }
        if scored_samples < BASELINE_MIN_SCORED_SAMPLES {
            warnings.push(format!(
                "baseline {} scored sample count is low ({scored_samples})",
                run_dir.display()
            ));
        }

        samples.push(BaselineSample {
            run_dir: baseline.run_dir,
            scenario_name: baseline.session.core.scenario_name.clone(),
            scenario_hash: baseline.session.core.scenario_hash.clone(),
            workload_label: baseline.session.core.workload_label.clone(),
            route_label: baseline.session.core.route_label.clone(),
            score,
            scored_samples,
            intervals_empty,
            valid,
        });
    }
    Ok(samples)
}

fn push_scenario_identity_warnings(
    baselines: &[BaselineSample],
    summary: &TuneSummary,
    warnings: &mut Vec<String>,
    options: &BaselineTuneRecommendationOptions,
) {
    if options.allow_scenario_mismatch {
        return;
    }

    for baseline in baselines {
        push_optional_identity_warning(
            warnings,
            &baseline.run_dir,
            "scenario_name",
            baseline.scenario_name.as_deref(),
            summary.scenario_name.as_deref(),
        );
        push_optional_identity_warning(
            warnings,
            &baseline.run_dir,
            "scenario_hash",
            baseline.scenario_hash.as_deref(),
            summary.scenario_hash.as_deref(),
        );
        push_optional_identity_warning(
            warnings,
            &baseline.run_dir,
            "workload_label",
            baseline.workload_label.as_deref(),
            summary.workload_label.as_deref(),
        );
        push_optional_identity_warning(
            warnings,
            &baseline.run_dir,
            "route_label",
            baseline.route_label.as_deref(),
            summary.route_label.as_deref(),
        );
    }
}

fn push_optional_identity_warning(
    warnings: &mut Vec<String>,
    run_dir: &Path,
    field: &str,
    baseline: Option<&str>,
    tune: Option<&str>,
) {
    if baseline == tune {
        return;
    }

    warnings.push(format!(
        "scenario-mismatch baseline={} field={} baseline={} tune={}",
        run_dir.display(),
        field,
        baseline.unwrap_or("<missing>"),
        tune.unwrap_or("<missing>")
    ));
}

fn valid_candidates_for_profile<'a>(
    summary: &'a TuneSummary,
    profile: &str,
) -> Vec<&'a TuneCandidateSummary> {
    summary
        .candidates
        .iter()
        .filter(|candidate| candidate.profile == profile && candidate.valid)
        .collect()
}

fn formal_metric_comparisons(
    baselines: &[&BaselineSample],
    tuned: &[&TuneCandidateSummary],
) -> Vec<FormalMetricComparison> {
    vec![
        statistics::compare_lower_is_better_metric(
            "diagnostic_raw_score_total",
            "score_points",
            &baseline_metric_values(baselines, |sample| sample.score.total as f64),
            &tuned_metric_values(tuned, |candidate| {
                candidate.diagnostic_raw_score_total as f64
            }),
        ),
        statistics::compare_lower_is_better_metric(
            "over_5ms",
            "samples",
            &baseline_metric_values(baselines, |sample| sample.score.over_5ms as f64),
            &tuned_metric_values(tuned, |candidate| candidate.over_5ms as f64),
        ),
        statistics::compare_lower_is_better_metric(
            "frame_p99_ms",
            "ms",
            &baseline_metric_values(baselines, |sample| sample.score.frame_p99_ms)
                .into_iter()
                .filter(|value| *value > 0.0)
                .collect::<Vec<_>>(),
            &tuned_metric_values(tuned, |candidate| candidate.frame_p99_ms)
                .into_iter()
                .filter(|value| *value > 0.0)
                .collect::<Vec<_>>(),
        ),
        statistics::compare_lower_is_better_metric(
            "frame_over_16ms",
            "frames",
            &baseline_metric_values(baselines, |sample| sample.score.frame_over_16ms as f64),
            &tuned_metric_values(tuned, |candidate| candidate.frame_over_16ms as f64),
        ),
        statistics::compare_lower_is_better_metric(
            "frame_over_33ms",
            "frames",
            &baseline_metric_values(baselines, |sample| sample.score.frame_over_33ms as f64),
            &tuned_metric_values(tuned, |candidate| candidate.frame_over_33ms as f64),
        ),
        statistics::compare_lower_is_better_metric(
            "frame_over_50ms",
            "frames",
            &baseline_metric_values(baselines, |sample| sample.score.frame_over_50ms as f64),
            &tuned_metric_values(tuned, |candidate| candidate.frame_over_50ms as f64),
        ),
        statistics::compare_lower_is_better_metric(
            "max_latency_ns",
            "ns",
            &baseline_metric_values(baselines, |sample| sample.score.max_latency_ns as f64),
            &tuned_metric_values(tuned, |candidate| candidate.max_latency_ns as f64),
        ),
    ]
}

fn extend_formal_metric_warnings(warnings: &mut Vec<String>, metrics: &[FormalMetricComparison]) {
    for metric in metrics {
        for warning in &metric.uncertainty_warnings {
            push_unique(warnings, format!("{}: {}", metric.metric, warning));
        }
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn formal_metric_effect_size(metrics: &[FormalMetricComparison], metric_name: &str) -> Option<f64> {
    metrics
        .iter()
        .find(|metric| metric.metric == metric_name)
        .and_then(|metric| metric.effect_size)
        .map(|value| value.abs())
}

fn metric_noise_ratio(values: &[f64]) -> Option<f64> {
    let values = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let median = statistics::median_f64(&values);
    if median <= f64::EPSILON {
        return None;
    }

    Some(iqr_f64(&values) / median.abs())
}

fn iqr_f64(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut values = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if values.is_empty() {
        return 0.0;
    }
    let mut q25_values = values.clone();
    let q25 = percentile_nearest_rank_f64(&mut q25_values, 25.0);
    let q75 = percentile_nearest_rank_f64(&mut values, 75.0);
    (q75 - q25).max(0.0)
}

fn percentile_nearest_rank_f64(values: &mut [f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(f64::total_cmp);
    let percentile = percentile.clamp(0.0, 100.0);
    let rank = ((percentile / 100.0) * values.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(values.len() - 1);
    values[idx]
}

fn baseline_metric_values(
    samples: &[&BaselineSample],
    value: impl Fn(&BaselineSample) -> f64,
) -> Vec<f64> {
    samples
        .iter()
        .map(|sample| value(sample))
        .filter(|value| value.is_finite())
        .collect()
}

fn tuned_metric_values(
    samples: &[&TuneCandidateSummary],
    value: impl Fn(&TuneCandidateSummary) -> f64,
) -> Vec<f64> {
    samples
        .iter()
        .map(|sample| value(sample))
        .filter(|value| value.is_finite())
        .collect()
}

fn median_u64_from_f64(values: &[f64]) -> u64 {
    statistics::median_f64(values).round().max(0.0) as u64
}

fn optional_median_f64(values: &[f64]) -> Option<f64> {
    let values = values
        .iter()
        .copied()
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| statistics::median_f64(&values))
}
