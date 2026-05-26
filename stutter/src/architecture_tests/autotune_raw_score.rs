use std::path::Path;

fn collect_rs_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_rs_files(&path));
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files
}

fn count_occurrences(source: &str, needle: &str) -> usize {
    source.match_indices(needle).count()
}

#[test]
fn controller_does_not_compare_raw_score_totals() {
    let source = include_str!("../autotune/controller.rs");
    assert!(
        !source.contains("score_regression_percent"),
        "controller.rs should not use raw score_regression_percent"
    );
    assert!(
        !source.contains("score_improvement_percent"),
        "controller.rs should not use raw score_improvement_percent"
    );
    // The active controller path must store a full WindowScore, not an ambiguous
    // raw baseline total.
    assert!(
        !source.contains("baseline_score_total"),
        "controller.rs should not use baseline_score_total"
    );
}

#[test]
fn diagnostic_raw_score_total_names_are_readable() {
    let sources = [
        include_str!("../autotune/controller.rs"),
        include_str!("../autotune/candidate_memory/diagnostics.rs"),
        include_str!("../autotune/candidate_memory/persistence.rs"),
        include_str!("../autotune/kept.rs"),
        include_str!("../autotune/runtime/daemon_state.rs"),
    ];

    for source in sources {
        assert!(!source.contains("diagnostic_baseline_diagnostic_score_total"));
        assert!(!source.contains("diagnostic_current_diagnostic_score_total"));
        assert!(!source.contains("diagnostic_candidate_diagnostic_score_total"));
    }
}

#[test]
fn controller_does_not_directly_branch_on_raw_score_totals() {
    let source = include_str!("../autotune/controller.rs");

    for forbidden in [
        ".score.total >",
        ".score.total <",
        ".score.total >=",
        ".score.total <=",
        "observation.score.total",
    ] {
        assert!(
            !source.contains(forbidden),
            "controller.rs should not directly branch on raw score totals: {forbidden}"
        );
    }
}

#[test]
fn raw_score_total_usage_is_confined_to_known_diagnostic_or_test_paths() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/autotune");

    let allowed = [
        "candidate_memory/diagnostics.rs",
        "candidate_memory/persistence.rs",
        "runtime/daemon_state.rs",
    ];

    for path in collect_rs_files(&root) {
        let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        let uses_raw_total = source.contains(".score.total")
            || source.contains("score_total()")
            || source.contains("raw_score_total");

        if uses_raw_total && !allowed.iter().any(|allowed| relative.ends_with(allowed)) {
            assert!(
                source.contains("to_window_score")
                    || source.contains("compare_scores_with_config")
                    || source.contains("compare_for_objective")
                    || source.contains("diagnostic")
                    || source.contains("#[test]")
                    || source.contains("#[cfg(test)]"),
                "raw score total usage in {relative} needs normalization or an explicit diagnostic marker"
            );
        }
    }
}

#[test]
fn active_experiment_stores_full_window_score() {
    let source = include_str!("../autotune/controller.rs");

    assert!(
        source.contains("pub baseline_score: WindowScore"),
        "ActiveExperiment must store the full baseline WindowScore"
    );

    assert!(
        !source.contains("pub baseline_score_total"),
        "ActiveExperiment must not store only a raw baseline score total"
    );
}

#[test]
fn no_duplicated_diagnostic_score_total_names_exist_in_source_tree() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    for path in collect_rs_files(&root) {
        let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        for forbidden in [
            "diagnostic_baseline_diagnostic_score_total",
            "diagnostic_current_diagnostic_score_total",
            "diagnostic_candidate_diagnostic_score_total",
        ] {
            if relative.starts_with("architecture_tests/") {
                continue;
            }

            if relative == "daemon/state.rs"
                && forbidden == "diagnostic_candidate_diagnostic_score_total"
            {
                continue;
            }

            assert!(
                !source.contains(forbidden),
                "{relative} should not contain awkward duplicated diagnostic raw-score name {forbidden}"
            );
        }
    }
}

#[test]
fn fake_daemon_interval_uses_raw_score_total_name() {
    let source = include_str!("../autotune/simulation.rs");

    assert!(
        source.contains("diagnostic_raw_score_total: u64"),
        "FakeDaemonStep::Interval should use the explicit raw-score field name"
    );
    assert!(
        !source.contains("diagnostic_score_total: u64"),
        "FakeDaemonStep::Interval must not reintroduce the ambiguous diagnostic_score_total field name"
    );
}

#[test]
fn daemon_decision_state_uses_current_raw_score_total_name() {
    let source = include_str!("../daemon/state.rs");

    assert!(
        source.contains("pub diagnostic_current_raw_score_total: Option<u64>"),
        "DaemonDecisionState should expose the persisted status score as diagnostic_current_raw_score_total"
    );
    assert!(
        source.contains(r#"alias = "diagnostic_score_total""#),
        "DaemonDecisionState should keep a serde alias for old persisted daemon-state JSON"
    );
    assert!(
        !source.contains("pub diagnostic_score_total: Option<u64>"),
        "DaemonDecisionState must not expose the ambiguous diagnostic_score_total field name"
    );
}

#[test]
fn daemon_workload_profile_uses_candidate_raw_score_total_name() {
    let source = include_str!("../daemon/state.rs");

    assert!(
        source.contains("pub diagnostic_candidate_raw_score_total: Option<u64>"),
        "DaemonWorkloadProfile should expose diagnostic_candidate_raw_score_total"
    );

    assert!(
        source.contains(r#"alias = "diagnostic_candidate_diagnostic_score_total""#),
        "DaemonWorkloadProfile should keep a serde alias for old persisted state"
    );
}

#[test]
fn daemon_workload_profile_does_not_expose_legacy_candidate_diagnostic_field_name() {
    let source = include_str!("../daemon/state.rs");

    assert!(
        !source.contains("pub diagnostic_candidate_diagnostic_score_total"),
        "DaemonWorkloadProfile must not expose the old duplicated diagnostic field name"
    );

    assert!(
        source.contains("pub diagnostic_candidate_raw_score_total: Option<u64>"),
        "DaemonWorkloadProfile should expose diagnostic_candidate_raw_score_total"
    );
}

#[test]
fn daemon_state_only_mentions_legacy_candidate_diagnostic_name_for_serde_compatibility() {
    let state_source = include_str!("../daemon/state.rs");
    let state_tests = include_str!("../daemon/state/tests.rs");

    assert!(
        state_source.contains(r#"#[serde(alias = "diagnostic_candidate_diagnostic_score_total")]"#),
        "daemon/state.rs should keep the serde alias for old persisted daemon-state JSON"
    );

    assert!(
        state_tests.contains("legacy_candidate_raw_score_total_name"),
        "daemon/state/tests.rs should keep a legacy compatibility fixture without spelling the awkward field name as one token"
    );

    assert!(
        !state_source.contains("pub diagnostic_candidate_diagnostic_score_total"),
        "daemon/state.rs must not reintroduce the old field name"
    );

    assert_eq!(
        count_occurrences(state_source, "diagnostic_candidate_diagnostic_score_total"),
        1,
        "daemon/state.rs should mention the legacy candidate name only in the serde alias"
    );
    assert_eq!(
        count_occurrences(state_tests, "diagnostic_candidate_diagnostic_score_total"),
        0,
        "daemon/state/tests.rs should build the legacy compatibility key from pieces so the awkward name cannot spread"
    );
}

#[test]
fn tune_profile_stats_use_raw_score_total_names_for_serialized_raw_totals() {
    let tune_source = include_str!("../tune/model.rs");
    let recommendation_source = include_str!("../tune/recommendation.rs");

    for source in [tune_source, recommendation_source] {
        assert!(source.contains("median_diagnostic_raw_score_total"));
        assert!(source.contains("iqr_diagnostic_raw_score_total"));
        assert!(source.contains("worst_diagnostic_raw_score_total"));
        assert!(source.contains(r#"alias = "median_diagnostic_score_total""#));
        assert!(source.contains(r#"alias = "iqr_diagnostic_score_total""#));
        assert!(source.contains(r#"alias = "worst_diagnostic_score_total""#));
    }
}

#[test]
fn observation_summary_uses_raw_score_total_name() {
    let source = include_str!("../autotune/history.rs");

    assert!(source.contains("pub diagnostic_raw_score_total: u64"));
    assert!(source.contains(r#"#[serde(alias = "diagnostic_score_total")]"#));
    assert!(!source.contains("pub diagnostic_score_total: u64"));
}

#[test]
fn decision_stream_entry_uses_raw_score_total_name() {
    let source = include_str!("../autotune/runtime/stream.rs");

    assert!(source.contains("pub diagnostic_raw_score_total: u64"));
    assert!(source.contains(r#"#[serde(alias = "diagnostic_score_total")]"#));
    assert!(source.contains("Serialize, Deserialize"));
    assert!(!source.contains("pub diagnostic_score_total: u64"));
}

#[test]
fn tune_candidate_summary_uses_raw_score_total_name() {
    let source = include_str!("../tune/model.rs");

    assert!(source.contains("pub diagnostic_raw_score_total: u64"));
    assert!(source.contains(r#"#[serde(alias = "diagnostic_score_total")]"#));
    assert!(!source.contains("pub diagnostic_score_total: u64"));
}

#[test]
fn decision_jsonl_entry_uses_raw_score_total_name() {
    let source = include_str!("../autotune/decision_log.rs");

    assert!(source.contains("pub diagnostic_raw_score_total: u64"));
    assert!(source.contains(r#"#[serde(alias = "diagnostic_score_total")]"#));
    assert!(!source.contains("pub diagnostic_score_total: u64"));
}
