use std::path::Path;

fn collect_rs_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_rs_files(&path));
            } else if path.extension().map_or(false, |ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files
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
    let controller = include_str!("../autotune/controller.rs");
    let memory = include_str!("../autotune/candidate_memory.rs");

    for source in [controller, memory] {
        assert!(!source.contains("diagnostic_baseline_diagnostic_score_total"));
        assert!(!source.contains("diagnostic_current_diagnostic_score_total"));
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
        "candidate_memory.rs",
        "runtime/daemon_state.rs",
    ];

    for path in collect_rs_files(&root) {
        let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        let uses_raw_total =
            source.contains(".score.total")
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
