//! Sensor readings are context for diagnosis. No planner, objective, provider, experiment or
//! support-registry code may read them, so a pressure or power reading can never choose, rank,
//! keep or revert an operation.
use std::{fs, path::Path};

use super::workspace_root;

const DECISION_PATHS: &[&str] = &[
    "stutter/src/autotune/objective",
    "stutter/src/autotune/planning",
    "stutter/src/autotune/planner_tests",
    "stutter/src/autotune/providers",
    "stutter/src/autotune/autonomous_support",
    "stutter/src/autotune/live_experiment",
    "stutter/src/autotune/decision.rs",
    "stutter/src/autotune/experiment_design.rs",
    "stutter/src/autotune/comparison",
    "stutter/src/autotune/comparison.rs",
];

const FORBIDDEN: &[&str] = &[
    "sensor_readings",
    "SensorReading",
    "autotune::sensors",
    "sensors::",
];

fn rust_files(path: &Path, out: &mut Vec<std::path::PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path.to_owned());
        }
        return;
    }
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            rust_files(&entry.path(), out);
        }
    }
}

#[test]
fn decision_code_never_reads_sensor_readings() {
    let root = workspace_root();
    let mut files = Vec::new();
    for relative in DECISION_PATHS {
        rust_files(&root.join(relative), &mut files);
    }
    assert!(
        files.len() > 20,
        "the decision paths moved; update this test ({} files)",
        files.len()
    );
    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap_or_default();
        for (index, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if let Some(word) = FORBIDDEN.iter().find(|word| line.contains(**word)) {
                violations.push(format!("{}:{} mentions {word}", file.display(), index + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "sensor readings reached decision code:\n{}",
        violations.join("\n")
    );
}
