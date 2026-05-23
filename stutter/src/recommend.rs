use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    artifacts::ArtifactSelection,
    scorer, session_io,
    tune::{self, RankingConfidence, TuneSummary, recommendation::TuneRecommendationVerdict},
};

#[derive(Debug, Clone)]
pub struct RecommendCommandInput {
    pub baseline: PathBuf,
    pub tune: PathBuf,
    pub json: bool,
    pub markdown: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineTuneRecommendation {
    pub schema_version: u32,
    pub baseline_run: PathBuf,
    pub tune_dir: PathBuf,
    pub best_profile: Option<String>,
    pub confidence: RankingConfidence,
    pub verdict: TuneRecommendationVerdict,
    pub summary: String,
    pub diagnostic_baseline_raw_score_total: u64,
    #[serde(alias = "best_median_diagnostic_score_total")]
    pub best_median_diagnostic_raw_score_total: Option<u64>,
    pub score_delta_abs: Option<i64>,
    pub score_delta_percent: Option<f64>,

    pub baseline_over_5ms: u64,
    pub best_median_over_5ms: Option<u64>,
    pub over_5ms_delta_abs: Option<i64>,

    pub baseline_frame_p99_ms: Option<f64>,
    pub best_median_frame_p99_ms: Option<f64>,
    pub frame_p99_delta_ms: Option<f64>,

    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
}

pub fn recommend_command(input: RecommendCommandInput) -> anyhow::Result<()> {
    let rec = build_baseline_tune_recommendation(&input.baseline, &input.tune)?;
    if input.json {
        println!("{}", serde_json::to_string_pretty(&rec)?);
    } else if let Some(markdown_path) = input.markdown {
        let markdown = render_baseline_tune_recommendation_markdown(&rec);
        if let Some(parent) = markdown_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&markdown_path, markdown)
            .with_context(|| format!("failed to write {}", markdown_path.display()))?;
        println!("recommendation={}", markdown_path.display());
    } else {
        print!("{}", render_baseline_tune_recommendation_markdown(&rec));
    }
    Ok(())
}

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

pub fn render_baseline_tune_recommendation_markdown(rec: &BaselineTuneRecommendation) -> String {
    let mut out = String::new();
    pushln(&mut out, "# stutter baseline/tune recommendation");
    pushln(&mut out, "");
    pushln(&mut out, format!("Verdict: {:?}", rec.verdict));
    pushln(
        &mut out,
        format!(
            "Best profile: {}",
            rec.best_profile.as_deref().unwrap_or("none")
        ),
    );
    pushln(&mut out, format!("Confidence: {:?}", rec.confidence));
    pushln(&mut out, "");
    pushln(&mut out, "## Summary");
    pushln(&mut out, "");
    pushln(&mut out, &rec.summary);
    pushln(&mut out, "");
    pushln(&mut out, "## Scores");
    pushln(&mut out, "");
    pushln(
        &mut out,
        format!(
            "- Baseline score: {}",
            rec.diagnostic_baseline_raw_score_total
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Best median score: {}",
            rec.best_median_diagnostic_raw_score_total
                .map(|score| score.to_string())
                .unwrap_or_else(|| "none".to_owned())
        ),
    );
    if let Some(delta) = rec.score_delta_abs {
        pushln(
            &mut out,
            format!(
                "- Score delta: {} ({})",
                delta,
                rec.score_delta_percent
                    .map(|pct| format!("{pct:.1}%"))
                    .unwrap_or_else(|| "n/a".to_owned())
            ),
        );
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Latency details");
    pushln(&mut out, "");
    pushln(
        &mut out,
        format!("- Baseline over 5ms: {}", rec.baseline_over_5ms),
    );
    pushln(
        &mut out,
        format!(
            "- Best median over 5ms: {}",
            rec.best_median_over_5ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Over 5ms delta: {}",
            rec.over_5ms_delta_abs
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Baseline frame p99: {}",
            rec.baseline_frame_p99_ms
                .map(|v| format!("{v:.3}ms"))
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Best median frame p99: {}",
            rec.best_median_frame_p99_ms
                .map(|v| format!("{v:.3}ms"))
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Frame p99 delta: {}",
            rec.frame_p99_delta_ms
                .map(|v| format!("{v:+.3}ms"))
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(&mut out, "");
    pushln(&mut out, "## Warnings");
    pushln(&mut out, "");
    if rec.warnings.is_empty() {
        pushln(&mut out, "- none");
    } else {
        for warning in &rec.warnings {
            pushln(&mut out, format!("- {warning}"));
        }
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Next steps");
    pushln(&mut out, "");
    for step in &rec.next_steps {
        pushln(&mut out, format!("- {step}"));
    }
    out
}

#[cfg(test)]
fn render_tune_recommendation_for_summary(summary: &TuneSummary) -> String {
    tune::recommendation::render_tune_recommendation_markdown(
        &tune::recommendation::build_tune_recommendation(summary, None),
    )
}

fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{
        metrics::IntervalRecord,
        process_tree::TaskClass,
        recorder::{RecordedConfig, RecordedTime, SessionFile},
        tune::{TuneCandidateSummary, TuneCoverageMetrics, TuneIterationOrder, TuneProfileStats},
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-recommend-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn minimal_session(interval_count: u64) -> SessionFile {
        SessionFile {
            core: crate::recorder::SessionMetadataCore {
                schema_version: crate::recorder::SESSION_SCHEMA_VERSION,
                run_name: Some("baseline".to_owned()),
                started_at: RecordedTime::default(),
                ended_at: RecordedTime::default(),
                duration_ms: 1000,
                interval_record_count: interval_count,
                active_target_pids_count: 1,
                ..Default::default()
            },
            stop_reason: "test".to_owned(),
            config: RecordedConfig {
                summary_period_ms: 1000,
                spike_threshold_ns: 1_000_000,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn interval(over_5ms: u64, samples: u64) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms: 1000,
            task: 1,
            active: true,
            class: TaskClass::Game,
            comm: "game".to_owned(),
            process_pid: Some(1),
            process_comm: "game".into(),
            samples,
            stored_samples: samples,
            over_5ms,
            percentile_scope: "all".to_owned(),
            ..Default::default()
        }
    }

    fn write_baseline(dir: &Path, score_over_5ms: u64, samples: u64) {
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec_pretty(&minimal_session(1)).unwrap(),
        )
        .unwrap();
        fs::write(
            dir.join("interval.json"),
            format!(
                "{}\n",
                serde_json::to_string(&interval(score_over_5ms, samples)).unwrap()
            ),
        )
        .unwrap();
    }

    fn stat(profile: &str, score: u64, confidence_runs: usize) -> TuneProfileStats {
        TuneProfileStats {
            profile: profile.to_owned(),
            valid_runs: confidence_runs,
            invalid_runs: 0,
            median_diagnostic_raw_score_total: score,
            iqr_diagnostic_raw_score_total: 0,
            worst_diagnostic_raw_score_total: score,
            median_over_5ms: score / 100,
            iqr_over_5ms: 0,
            median_frame_p99_us: 0,
            iqr_frame_p99_us: 0,
        }
    }

    fn candidate(profile: &str, score: u64) -> TuneCandidateSummary {
        TuneCandidateSummary {
            profile: profile.to_owned(),
            iteration: 1,
            run_dir: PathBuf::from(format!("/tmp/{profile}")),
            applied_tasks: 1,
            warmup_seconds: 0,
            measure_seconds: 1,
            interval_count: 1,
            samples: 100,
            scored_samples: 100,
            diagnostic_raw_score_total: score,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: score / 100,
            max_latency_ns: 0,
            frame_count: 0,
            frame_max_ms: 0.0,
            frame_p99_ms: 0.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
            coverage: TuneCoverageMetrics::default(),
            valid: true,
        }
    }

    fn write_tune(dir: &Path, confidence: RankingConfidence, best_score: u64) {
        fs::create_dir_all(dir).unwrap();
        let summary = TuneSummary {
            schema_version: 1,
            tree_pid: 42,
            profiles_path: PathBuf::from("profiles.toml"),
            runs: 3,
            epoch_seconds: 60,
            warmup_seconds: 10,
            restore_policy: "restore-after-each".to_owned(),
            best_profile: if confidence == RankingConfidence::Unstable {
                String::new()
            } else {
                "best".to_owned()
            },
            candidate_order: vec![TuneIterationOrder {
                iteration: 1,
                profiles: vec!["best".to_owned(), "other".to_owned()],
            }],
            profile_stats: vec![
                stat("best", best_score, 3),
                stat("other", best_score + 100, 3),
            ],
            ranking_confidence: confidence,
            ranking_notes: Vec::new(),
            comparability_warnings: Vec::new(),
            candidates: vec![candidate("best", best_score)],
        };
        fs::write(
            dir.join("tuning_summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn missing_tuning_summary_fails_clearly() {
        let baseline = temp_dir("missing-summary-baseline");
        write_baseline(&baseline, 2, 100);
        let tune = temp_dir("missing-summary-tune");

        let err = build_baseline_tune_recommendation(&baseline, &tune).unwrap_err();

        assert!(err.to_string().contains("missing tuning summary"));
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn unstable_tune_summary_gives_no_recommendation() {
        let baseline = temp_dir("unstable-baseline");
        write_baseline(&baseline, 2, 100);
        let tune = temp_dir("unstable-tune");
        write_tune(&tune, RankingConfidence::Unstable, 100);

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

        assert_eq!(rec.verdict, TuneRecommendationVerdict::NoRecommendation);
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn best_worse_than_baseline_gives_no_recommendation() {
        let baseline = temp_dir("worse-baseline");
        write_baseline(&baseline, 1, 100);
        let tune = temp_dir("worse-tune");
        write_tune(&tune, RankingConfidence::High, 200);

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

        assert_eq!(rec.verdict, TuneRecommendationVerdict::NoRecommendation);
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn best_more_than_five_percent_better_gives_recommended() {
        let baseline = temp_dir("better-baseline");
        write_baseline(&baseline, 2, 100);
        let tune = temp_dir("better-tune");
        write_tune(&tune, RankingConfidence::High, 100);

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

        assert_eq!(rec.verdict, TuneRecommendationVerdict::Recommended);
        assert_eq!(rec.score_delta_abs, Some(100));
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn low_confidence_gives_needs_retest() {
        let baseline = temp_dir("low-baseline");
        write_baseline(&baseline, 2, 100);
        let tune = temp_dir("low-tune");
        write_tune(&tune, RankingConfidence::Low, 100);

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

        assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn json_output_serializes_expected_fields() {
        let baseline = temp_dir("json-baseline");
        write_baseline(&baseline, 2, 100);
        let tune = temp_dir("json-tune");
        write_tune(&tune, RankingConfidence::High, 100);

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();
        let json = serde_json::to_value(&rec).unwrap();

        assert_eq!(json["schema_version"], 2);
        assert_eq!(json["best_profile"], "best");
        assert_eq!(json["diagnostic_baseline_raw_score_total"], 200);
        assert_eq!(json["baseline_over_5ms"], 2);
        assert!(json["best_median_over_5ms"].is_number());
        assert!(json["over_5ms_delta_abs"].is_number());
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn zero_baseline_score_never_recommends() {
        let baseline = temp_dir("zero-baseline-score");
        write_baseline(&baseline, 0, 100); // score 0
        let tune = temp_dir("zero-tune-score");
        write_tune(&tune, RankingConfidence::High, 0); // score 0

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

        assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
        assert!(
            rec.warnings
                .iter()
                .any(|w| w.contains("baseline score is zero"))
        );
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn very_low_baseline_score_needs_retest() {
        let baseline = temp_dir("low-baseline-score");
        // Score will be 50 * 100 = 5000ns? No, interval(over_5ms, samples) returns score_over_5ms * 100 in stat()?
        // Wait, scorer::score_from_interval_records_and_frames calculates score.
        // Let's look at interval():
        // over_5ms is passed as a field.
        // stat() uses score / 100 for median_over_5ms.
        // baseline_score.total is calculated from intervals.
        // scorer::score_from_interval_records_and_frames(intervals, frames)
        // total = intervals.iter().map(|r| r.over_5ms as u64 * 100 + r.over_1ms as u64 * 10).sum()
        // If I pass 0 to write_baseline, total is 0.
        // If I pass 1, total is 100.
        // LOW_BASELINE_SCORE_TOTAL is 100.

        write_baseline(&baseline, 0, 100);
        // Change it to exactly 50 total score.
        fs::write(
            baseline.join("interval.json"),
            format!(
                "{}\n",
                serde_json::to_string(&IntervalRecord {
                    elapsed_ms: 1000,
                    task: 1,
                    active: true,
                    class: TaskClass::Game,
                    comm: "game".to_owned(),
                    samples: 100,
                    stored_samples: 100,
                    over_1ms: 50, // 50 score
                    ..Default::default()
                })
                .unwrap()
            ),
        )
        .unwrap();

        let tune = temp_dir("low-tune-score");
        write_tune(&tune, RankingConfidence::High, 10); // score 10

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

        assert_eq!(rec.diagnostic_baseline_raw_score_total, 50);
        assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
        assert!(
            rec.warnings
                .iter()
                .any(|w| w.contains("baseline score is very low (50)"))
        );
        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }

    #[test]
    fn frame_p99_metrics_are_captured_structured() {
        let baseline = temp_dir("frame-baseline");
        fs::write(
            baseline.join("session.json"),
            serde_json::to_vec_pretty(&minimal_session(1)).unwrap(),
        )
        .unwrap();
        fs::write(
            baseline.join("frame_correlation.json"),
            r#"{"elapsed_ms":100,"frametime_ms":16.0}
{"elapsed_ms":200,"frametime_ms":17.0}
"#,
        )
        .unwrap();

        let tune = temp_dir("frame-tune");
        let summary = TuneSummary {
            schema_version: 1,
            tree_pid: 42,
            profiles_path: PathBuf::from("profiles.toml"),
            runs: 3,
            epoch_seconds: 60,
            warmup_seconds: 10,
            restore_policy: "restore-after-each".to_owned(),
            best_profile: "best".to_owned(),
            candidate_order: vec![],
            profile_stats: vec![TuneProfileStats {
                profile: "best".to_owned(),
                valid_runs: 3,
                invalid_runs: 0,
                median_diagnostic_raw_score_total: 100,
                iqr_diagnostic_raw_score_total: 0,
                worst_diagnostic_raw_score_total: 100,
                median_over_5ms: 1,
                iqr_over_5ms: 0,
                median_frame_p99_us: 15000, // 15ms
                iqr_frame_p99_us: 0,
            }],
            ranking_confidence: RankingConfidence::High,
            ranking_notes: Vec::new(),
            comparability_warnings: Vec::new(),
            candidates: vec![candidate("best", 100)],
        };
        fs::write(
            tune.join("tuning_summary.json"),
            serde_json::to_vec_pretty(&summary).unwrap(),
        )
        .unwrap();

        let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();
        let tune_only_markdown = render_tune_recommendation_for_summary(&summary);

        assert!(tune_only_markdown.contains("# stutter tuning recommendation"));
        assert!(tune_only_markdown.contains("Best profile: best"));
        assert_eq!(rec.baseline_frame_p99_ms, Some(17.0));
        assert_eq!(rec.best_median_frame_p99_ms, Some(15.0));
        assert_eq!(rec.frame_p99_delta_ms, Some(-2.0));

        fs::remove_dir_all(baseline).ok();
        fs::remove_dir_all(tune).ok();
    }
}
