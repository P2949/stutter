use super::*;

#[cfg(test)]
mod keep_best_policy_tests {
    use crate::{
        actions::cpu_affinity::CpuAffinityProfileAction,
        daemon_policy::{ActionSource, DaemonMode, PolicyIntent, PolicyRejection},
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
    };

    #[test]
    fn tune_keep_best_uses_policy_and_rejects_medium_risk_without_medium_mode() {
        let profile = Profile {
            name: "medium-priority".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: Some(10),
                ionice: None,
                match_class: vec![TaskClass::Indexer],
                match_comm: Vec::new(),
            }],
        };
        let action = CpuAffinityProfileAction {
            tree_pid: 1234,
            profile,
            force_restore_overwrite: false,
        };
        let descriptor = action.descriptor_with_persistent_effect(true);

        let low_policy = crate::watch::profile_apply_policy(false, false, true, ActionSource::Tune);
        assert!(matches!(
            low_policy.check_action(PolicyIntent::Apply, &descriptor),
            Err(PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                ..
            })
        ));

        let medium_policy =
            crate::watch::profile_apply_policy(false, true, true, ActionSource::Tune);
        assert!(
            medium_policy
                .check_action(PolicyIntent::Apply, &descriptor)
                .is_ok()
        );
    }
}

#[cfg(test)]
mod ranking_tests {
    use std::path::{Path, PathBuf};

    use super::{
        ranking::{iqr_u64, percentile_nearest_rank_u64},
        *,
    };

    fn tune_candidate(
        profile: &str,
        iteration: u32,
        diagnostic_raw_score_total: u64,
        valid: bool,
    ) -> TuneCandidateSummary {
        TuneCandidateSummary {
            profile: profile.to_owned(),
            iteration,
            run_dir: PathBuf::from(format!("/tmp/{profile}-{iteration}")),
            applied_tasks: 1,
            warmup_seconds: 1,
            measure_seconds: 1,
            interval_count: 2,
            samples: 100,
            scored_samples: 100,
            diagnostic_raw_score_total,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            max_latency_ns: 0,
            frame_count: 0,
            frame_max_ms: 0.0,
            frame_p99_ms: 0.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
            coverage: TuneCoverageMetrics::default(),
            valid,
        }
    }

    fn grouped_candidates(
        candidates: Vec<TuneCandidateSummary>,
    ) -> BTreeMap<String, Vec<TuneCandidateSummary>> {
        let mut grouped = BTreeMap::new();
        for candidate in candidates {
            grouped
                .entry(candidate.profile.clone())
                .or_insert_with(Vec::new)
                .push(candidate);
        }
        grouped
    }

    #[test]
    fn candidate_order_counterbalances_iterations() {
        assert_eq!(candidate_order_for_iteration(3, 1), vec![0, 1, 2]);
        assert_eq!(candidate_order_for_iteration(3, 2), vec![0, 2, 1]);
        assert_eq!(candidate_order_for_iteration(3, 3), vec![2, 0, 1]);
        assert_eq!(candidate_order_for_iteration(1, 4), vec![0]);
    }

    #[test]
    fn percentile_nearest_rank_and_iqr_work_on_u64_values() {
        let mut values = vec![40, 10, 30, 20];
        assert_eq!(percentile_nearest_rank_u64(&mut values, 25.0), 10);
        let mut values = vec![40, 10, 30, 20];
        assert_eq!(percentile_nearest_rank_u64(&mut values, 50.0), 20);
        let mut values = vec![40, 10, 30, 20];
        assert_eq!(percentile_nearest_rank_u64(&mut values, 100.0), 40);
        assert_eq!(iqr_u64(vec![10, 20, 30, 40]), 20);
    }

    #[test]
    fn ranking_confidence_is_unstable_for_close_results() {
        let grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 100, true),
            tune_candidate("a", 2, 100, true),
            tune_candidate("a", 3, 120, true),
            tune_candidate("b", 1, 110, true),
            tune_candidate("b", 2, 110, true),
            tune_candidate("b", 3, 110, true),
        ]);
        let stats = profile_stats_from_grouped(&grouped);
        let (confidence, notes) = assess_ranking_confidence(&stats, &grouped, "a", 3);

        assert_eq!(confidence, RankingConfidence::Unstable);
        assert!(notes.iter().any(|note| note.contains("variance")));
    }

    #[test]
    fn ranking_confidence_distinguishes_high_medium_and_low() {
        let high_grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 90, true),
            tune_candidate("a", 2, 90, true),
            tune_candidate("a", 3, 90, true),
            tune_candidate("b", 1, 120, true),
            tune_candidate("b", 2, 120, true),
            tune_candidate("b", 3, 120, true),
        ]);
        let high_stats = profile_stats_from_grouped(&high_grouped);
        let (confidence, _) = assess_ranking_confidence(&high_stats, &high_grouped, "a", 3);
        assert_eq!(confidence, RankingConfidence::High);

        let medium_grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 90, true),
            tune_candidate("a", 2, 90, true),
            tune_candidate("a", 3, 100, true),
            tune_candidate("b", 1, 150, true),
            tune_candidate("b", 2, 150, true),
            tune_candidate("b", 3, 150, true),
        ]);
        let medium_stats = profile_stats_from_grouped(&medium_grouped);
        let (confidence, _) = assess_ranking_confidence(&medium_stats, &medium_grouped, "a", 3);
        assert_eq!(confidence, RankingConfidence::Medium);

        let low_grouped = grouped_candidates(vec![
            tune_candidate("a", 1, 90, true),
            tune_candidate("a", 2, 90, true),
            tune_candidate("b", 1, 120, true),
            tune_candidate("b", 2, 120, true),
        ]);
        let low_stats = profile_stats_from_grouped(&low_grouped);
        let (confidence, _) = assess_ranking_confidence(&low_stats, &low_grouped, "a", 2);
        assert_eq!(confidence, RankingConfidence::Low);
    }

    #[test]
    fn test_retain_after_warmup() {
        struct TestRecord {
            elapsed_ms: u64,
        }
        let mut records = vec![
            TestRecord { elapsed_ms: 0 },
            TestRecord { elapsed_ms: 500 },
            TestRecord { elapsed_ms: 1000 },
            TestRecord { elapsed_ms: 2000 },
        ];

        retain_after_warmup(&mut records, 1, |r| r.elapsed_ms);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].elapsed_ms, 1000);
        assert_eq!(records[1].elapsed_ms, 2000);
    }

    #[test]
    fn test_tune_run_dir_iteration() {
        let base = Path::new("/tmp/tune");
        assert_ne!(tune_run_dir(base, "kcd", 1), tune_run_dir(base, "kcd", 2));
        assert_eq!(tune_run_dir(base, "kcd", 1), base.join("iter-001-kcd"));
    }

    #[test]
    fn test_sanitize_profile_name() {
        let base = Path::new("/tmp/tune");
        assert_eq!(
            tune_run_dir(base, "my profile/name", 1),
            base.join("iter-001-my_profile_name")
        );
        assert_eq!(
            tune_run_dir(base, "hot-path#123", 1),
            base.join("iter-001-hot-path_123")
        );
        assert_eq!(
            tune_run_dir(base, "../traversal", 1),
            base.join("iter-001-___traversal")
        );
    }
}
