use super::*;

#[test]
fn best_baseline_compares_against_best_non_baseline_candidate() {
    let mut summary = summary(RankingConfidence::High);
    summary.best_profile = "baseline".to_owned();
    summary.profile_stats = vec![
        stat("baseline", 80),
        stat("tuned", 120),
        stat("worse-tuned", 200),
    ];

    for candidate in &mut summary.candidates {
        if candidate.profile == "best" {
            candidate.profile = "baseline".to_owned();
            candidate.diagnostic_raw_score_total = 80;
        } else if candidate.profile == "baseline" {
            candidate.profile = "tuned".to_owned();
            candidate.diagnostic_raw_score_total = 120;
        } else if candidate.profile == "second" {
            candidate.profile = "worse-tuned".to_owned();
            candidate.diagnostic_raw_score_total = 200;
        }
    }

    let rec = build_tune_recommendation(&summary, Some("baseline"));

    assert_eq!(rec.best_profile.as_deref(), Some("baseline"));
    assert_eq!(rec.baseline_profile.as_deref(), Some("baseline"));
    assert_eq!(rec.compared_against.as_deref(), Some("best-non-baseline"));
    assert_eq!(
        rec.comparison_metrics.as_ref().unwrap().other_profile,
        "tuned"
    );
    assert_ne!(
        rec.best_profile.as_deref(),
        Some(
            rec.comparison_metrics
                .as_ref()
                .unwrap()
                .other_profile
                .as_str()
        )
    );
    assert!(
        rec.summary
            .contains("Baseline profile 'baseline' remained best")
    );
    assert!(
        rec.warnings
            .iter()
            .any(|warning| warning.contains("best profile is the baseline profile"))
    );
}

#[test]
fn best_tuned_profile_still_compares_against_baseline() {
    let rec = build_tune_recommendation(&summary(RankingConfidence::High), Some("baseline"));

    assert_eq!(rec.best_profile.as_deref(), Some("best"));
    assert_eq!(rec.compared_against.as_deref(), Some("baseline"));
    assert_eq!(
        rec.comparison_metrics.as_ref().unwrap().other_profile,
        "baseline"
    );
}

#[test]
fn best_baseline_without_other_valid_candidate_has_no_comparison_target() {
    let mut summary = summary(RankingConfidence::High);
    summary.best_profile = "baseline".to_owned();
    summary.profile_stats = vec![stat("baseline", 80)];
    summary
        .candidates
        .retain(|candidate| candidate.profile == "baseline");
    for candidate in &mut summary.candidates {
        candidate.profile = "baseline".to_owned();
    }

    let rec = build_tune_recommendation(&summary, Some("baseline"));

    assert_eq!(rec.best_profile.as_deref(), Some("baseline"));
    assert_eq!(rec.compared_against, None);
    assert!(rec.comparison_metrics.is_none());
    assert!(
        rec.warnings
            .iter()
            .any(|warning| warning.contains("no valid comparison target"))
    );
}

#[test]
fn recommendation_never_compares_best_profile_against_itself() {
    let mut baseline_best = summary(RankingConfidence::High);
    baseline_best.best_profile = "baseline".to_owned();
    baseline_best.profile_stats = vec![stat("baseline", 80), stat("tuned", 120)];
    for candidate in &mut baseline_best.candidates {
        if candidate.profile == "best" {
            candidate.profile = "baseline".to_owned();
            candidate.diagnostic_raw_score_total = 80;
        } else {
            candidate.profile = "tuned".to_owned();
            candidate.diagnostic_raw_score_total = 120;
        }
    }

    let cases = [
        (summary(RankingConfidence::High), Some("baseline")),
        (summary(RankingConfidence::High), None),
        (baseline_best, Some("baseline")),
    ];

    for (summary, baseline) in cases {
        let rec = build_tune_recommendation(&summary, baseline);

        if let (Some(best), Some(comparison)) =
            (rec.best_profile.as_deref(), rec.comparison_metrics.as_ref())
        {
            assert_ne!(best, comparison.other_profile);
        }
    }
}
