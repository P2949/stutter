    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::model::*;
    use crate::tune::model::TuneCandidateSummary;
    use crate::process_tree::TaskClass;

    fn mock_candidate(name: &str, coverage: TuneCoverageMetrics) -> TuneCandidateSummary {
        TuneCandidateSummary {
            profile: name.to_string(),
            iteration: 1,
            run_dir: PathBuf::from("."),
            applied_tasks: 0,
            warmup_seconds: 0,
            measure_seconds: 0,
            interval_count: 0,
            samples: 0,
            scored_samples: 0,
            diagnostic_raw_score_total: 0,
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
            coverage,
            valid: true,
        }
    }

    fn warning_candidate(
        name: &str,
        scored_samples: u64,
        frame_count: usize,
        drop_counter_total: u64,
        applied_tasks: usize,
        valid: bool,
    ) -> TuneCandidateSummary {
        TuneCandidateSummary {
            profile: name.to_string(),
            iteration: 1,
            run_dir: PathBuf::from("."),
            applied_tasks,
            warmup_seconds: 0,
            measure_seconds: 0,
            interval_count: 2,
            samples: scored_samples,
            scored_samples,
            diagnostic_raw_score_total: 0,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            max_latency_ns: 0,
            frame_count,
            frame_max_ms: 0.0,
            frame_p99_ms: 0.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
            coverage: TuneCoverageMetrics {
                drop_counter_total,
                ..Default::default()
            },
            valid,
        }
    }

    fn grouped_for_warnings(
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
    fn test_check_tune_metric_ratio() {
        let values = vec![10, 15, 20];
        assert!(check_tune_metric_ratio("test", values.into_iter()).is_ok());

        let values = vec![10, 21];
        assert!(check_tune_metric_ratio("test", values.into_iter()).is_err());

        let values = vec![0, 10];
        assert!(check_tune_metric_ratio("test", values.into_iter()).is_err());
    }

    #[test]
    fn test_scored_identity_overlap() {
        let mut left = BTreeMap::new();
        let mut right = BTreeMap::new();

        let id1 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t1".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        let id2 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t2".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        left.insert(id1.clone(), 10);
        left.insert(id2.clone(), 5);

        right.insert(id1.clone(), 8);
        right.insert(id2.clone(), 12);

        assert_eq!(scored_identity_overlap(&left, &right, usize::min), 8 + 5);
        assert_eq!(scored_identity_overlap(&left, &right, usize::max), 10 + 12);
    }

    #[test]
    fn test_check_tune_coverage_comparability_overlap_failure() {
        let id1 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t1".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        let id2 = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "p1".to_string(),
            comm: "t2".to_string(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
        };

        let mut counts1 = BTreeMap::new();
        counts1.insert(id1.clone(), 100);

        let mut counts2 = BTreeMap::new();
        counts2.insert(id2.clone(), 100);

        let mut grouped = BTreeMap::new();
        grouped.insert(
            "p1".to_string(),
            vec![mock_candidate(
                "p1",
                TuneCoverageMetrics {
                    unique_tracked_tasks: 10,
                    unique_scored_tasks: 10,
                    active_target_min: 10,
                    active_target_max: 10,
                    scored_identity_counts: scored_identity_map_to_counts(counts1),
                    ..Default::default()
                },
            )],
        );
        grouped.insert(
            "p2".to_string(),
            vec![mock_candidate(
                "p2",
                TuneCoverageMetrics {
                    unique_tracked_tasks: 10,
                    unique_scored_tasks: 10,
                    active_target_min: 10,
                    active_target_max: 10,
                    scored_identity_counts: scored_identity_map_to_counts(counts2),
                    ..Default::default()
                },
            )],
        );

        let result = check_tune_coverage_comparability(&grouped);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("identity mismatch")
        );
    }

    #[test]
    fn frame_count_mismatch_produces_warning() {
        let grouped = grouped_for_warnings(vec![
            warning_candidate("a", 100, 100, 0, 1, true),
            warning_candidate("b", 100, 130, 0, 1, true),
        ]);

        let warnings = tune_comparability_warnings(&grouped);

        assert!(
            warnings
                .iter()
                .any(|warning| warning.kind == "frame-count-mismatch"
                    && warning.severity == TuneComparabilitySeverity::Warning)
        );
    }

    #[test]
    fn scored_sample_mismatch_produces_warning() {
        let grouped = grouped_for_warnings(vec![
            warning_candidate("a", 100, 0, 0, 1, true),
            warning_candidate("b", 130, 0, 0, 1, true),
        ]);

        let warnings = tune_comparability_warnings(&grouped);

        assert!(
            warnings
                .iter()
                .any(|warning| warning.kind == "scored-sample-count-mismatch"
                    && warning.severity == TuneComparabilitySeverity::Warning)
        );
    }

    #[test]
    fn drop_counter_produces_warning() {
        let grouped = grouped_for_warnings(vec![
            warning_candidate("a", 100, 0, 3, 1, true),
            warning_candidate("b", 100, 0, 0, 1, true),
        ]);

        let warnings = tune_comparability_warnings(&grouped);

        assert!(
            warnings
                .iter()
                .any(|warning| warning.kind == "drop-counters-nonzero"
                    && warning.profile.as_deref() == Some("a"))
        );
    }

    #[test]
    fn reject_level_existing_behavior_is_preserved() {
        let grouped = grouped_for_warnings(vec![
            warning_candidate("a", 0, 0, 0, 1, true),
            warning_candidate("b", 100, 0, 0, 1, true),
        ]);

        assert!(check_tune_sample_comparability(&grouped).is_err());
        assert!(
            tune_comparability_warnings(&grouped)
                .iter()
                .any(|warning| warning.kind == "scored-sample-count-mismatch"
                    && warning.severity == TuneComparabilitySeverity::Reject)
        );
    }

    #[test]
    fn tune_coverage_metrics_with_non_empty_identity_counts_serializes_to_json() {
        let identity = TaskIdentity {
            class: TaskClass::Game,
            process_comm: "game".to_string(),
            comm: "render".to_string(),
            process_starttime_ticks: Some(100),
            task_starttime_ticks: Some(101),
            exe_dev: Some(1),
            exe_ino: Some(2),
        };
        let metrics = TuneCoverageMetrics {
            unique_tracked_tasks: 1,
            unique_scored_tasks: 1,
            active_target_min: 1,
            active_target_max: 1,
            scored_identity_counts: vec![ScoredIdentityCount { identity, count: 1 }],
            ..Default::default()
        };

        let json = serde_json::to_string_pretty(&metrics).unwrap();
        assert!(json.contains("scored_identity_counts"));
        assert!(json.contains("\"identity\""));
        assert!(json.contains("\"count\""));

        let roundtrip: TuneCoverageMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.scored_identity_counts.len(), 1);
    }
