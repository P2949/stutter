use std::{fs, path::Path};
use anyhow::Context;

use crate::{
    artifacts::ArtifactSelection,
    scorer, session_io,
    tune::{self, RankingConfidence, TuneSummary, recommendation::TuneRecommendationVerdict},
};

use super::model::BaselineTuneRecommendation;

pub fn build_baseline_tune_recommendation(
    baseline_run: &Path,
    tune_dir: &Path,
) -> anyhow::Result<BaselineTuneRecommendation> {
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

    let baseline = session_io::load_run_artifacts(baseline_run, ArtifactSelection::tune())
        .with_context(|| format!("failed to load baseline run {}", baseline_run.display()))?;
    let baseline_score =
        scorer::score_from_interval_records_and_frames(&baseline.intervals, &baseline.frame_events);
    let (_, baseline_scored_samples) = tune::tune_scored_record_counts(&baseline.intervals);

    let mut warnings = Vec::new();
    warnings.extend(baseline.validation.errors.iter().cloned());
    warnings.extend(baseline.validation.warnings.iter().cloned());
    warnings.extend(
        baseline
            .validation
            .missing_optional_files
            .iter()
            .map(|file| format!("baseline missing optional artifact: {file}")),
    );
    warnings.extend(summary.ranking_notes.iter().cloned());

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
        .map(|best| baseline_score.total as i64 - best as i64);
    let score_delta_percent = score_delta_abs.and_then(|delta| {
        (baseline_score.total > 0).then_some(delta as f64 / baseline_score.total as f64 * 100.0)
    });

    let baseline_over_5ms = baseline_score.over_5ms;
    let best_median_over_5ms = best_stat.map(|stat| stat.median_over_5ms);
    let over_5ms_delta_abs =
        best_median_over_5ms.map(|best| baseline_over_5ms as i64 - best as i64);

    let baseline_frame_p99_ms =
        (baseline_score.frame_p99_ms > 0.0).then_some(baseline_score.frame_p99_ms);

    let best_median_frame_p99_ms = best_stat.and_then(|stat| {
        (stat.median_frame_p99_us > 0).then_some(stat.median_frame_p99_us as f64 / 1000.0)
    });

    let frame_p99_delta_ms = match (baseline_frame_p99_ms, best_median_frame_p99_ms) {
        (Some(base), Some(best)) => Some(best - base),
        _ => None,
    };

    if baseline.intervals.is_empty() {
        warnings.push("baseline intervals are missing".to_owned());
    }
    if baseline_scored_samples < 50 {
        warnings.push(format!(
            "baseline scored sample count is low ({baseline_scored_samples})"
        ));
    }
    if let Some(best_profile) = &best_profile {
        let best_candidates = summary
            .candidates
            .iter()
            .filter(|candidate| candidate.profile == *best_profile)
            .collect::<Vec<_>>();
        if best_candidates.is_empty() {
            warnings.push(format!(
                "best profile '{best_profile}' has no candidate runs"
            ));
        } else if best_candidates.iter().any(|candidate| !candidate.valid) {
            warnings.push(format!(
                "best profile '{best_profile}' has invalid candidate runs"
            ));
        }
    }

    let baseline_quality_blocks_recommendation =
        baseline.intervals.is_empty() || baseline_scored_samples < 50;

    const LOW_BASELINE_SCORE_TOTAL: u64 = 100;

    let baseline_score_blocks_recommendation = if baseline_score.total == 0 {
        warnings.push("baseline score is zero; score improvement cannot be measured".to_owned());
        true
    } else if baseline_score.total < LOW_BASELINE_SCORE_TOTAL {
        warnings.push(format!(
            "baseline score is very low ({}); recommendation requires retest",
            baseline_score.total
        ));
        true
    } else {
        false
    };

    let verdict = if summary.ranking_confidence == RankingConfidence::Unstable {
        TuneRecommendationVerdict::NoRecommendation
    } else if let Some(best) = best_median_diagnostic_raw_score_total {
        let worse_threshold = baseline_score
            .total
            .saturating_add(baseline_score.total / 20);
        let improvement_threshold = baseline_score.total / 20;
        let improvement = baseline_score.total.saturating_sub(best);
        if best > worse_threshold {
            TuneRecommendationVerdict::NoRecommendation
        } else if baseline_quality_blocks_recommendation
            || baseline_score_blocks_recommendation
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
            "Best tuned profile '{}' improved the baseline by {} score point(s).",
            best_profile.as_deref().unwrap_or("unknown"),
            score_delta_abs.unwrap_or(0)
        ),
        TuneRecommendationVerdict::NeedsRetest => format!(
            "Best tuned profile '{}' needs another comparable run before it is trusted.",
            best_profile.as_deref().unwrap_or("unknown")
        ),
        TuneRecommendationVerdict::NoRecommendation => {
            if summary.ranking_confidence == RankingConfidence::Unstable {
                "No recommendation: tune ranking was unstable.".to_owned()
            } else if let Some(best) = best_median_diagnostic_raw_score_total {
                if best > baseline_score.total {
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
            next_steps
                .push("Rerun baseline and tune with the same route, scene, and load.".to_owned());
            next_steps.push("Use at least 5 tune runs for a stronger comparison.".to_owned());
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
        schema_version: 2,
        baseline_run: baseline.run_dir,
        tune_dir: tune_dir.to_path_buf(),
        best_profile,
        confidence: summary.ranking_confidence,
        verdict,
        summary: summary_text,
        diagnostic_baseline_raw_score_total: baseline_score.total,
        best_median_diagnostic_raw_score_total,
        score_delta_abs,
        score_delta_percent,
        baseline_over_5ms,
        best_median_over_5ms,
        over_5ms_delta_abs,
        baseline_frame_p99_ms,
        best_median_frame_p99_ms,
        frame_p99_delta_ms,
        warnings,
        next_steps,
    })
}
