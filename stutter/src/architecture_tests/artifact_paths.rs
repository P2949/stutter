//! Artifact file-name architecture guard tests.

use std::{fs, path::Path};

use super::{
    crate_src_root, relative_to_crate_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

const ARTIFACT_FILENAMES: &[&str] = &[
    "metadata.json",
    "session.json",
    "interval.json",
    "spike_events.json",
    "tree_events.json",
    "irq_events.json",
    "gpu_samples.json",
    "frame_correlation.json",
    "frame_events.json",
    "migration_events.json",
    "cpu_freq_samples.json",
    "io_events.json",
    "scx_events.json",
    "runtime_slices.json",
    "focus_events.json",
    "foreground_events.json",
    "kms_flip_events.json",
    "drm_fence_events.json",
    "wayland_presentation_events.json",
    "display_topology.json",
    "dmabuf_events.json",
    "gpu_engine_samples.json",
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectArtifactPathJoin {
    path: String,
    line_number: usize,
    line: String,
}

fn direct_artifact_path_joins_in_file(path: &Path) -> Vec<DirectArtifactPathJoin> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let relative_path = relative_to_crate_root(path);
    direct_artifact_path_joins_in_source(&source, &relative_path)
}

fn direct_artifact_path_joins_in_source(source: &str, path: &str) -> Vec<DirectArtifactPathJoin> {
    if is_test_source_path(path) {
        return Vec::new();
    }

    let mut joins = Vec::new();
    for (line_number, line) in production_code_lines_outside_cfg_test_modules(source) {
        if ARTIFACT_FILENAMES
            .iter()
            .any(|file_name| line.contains(&format!(".join(\"{file_name}\")")))
        {
            joins.push(DirectArtifactPathJoin {
                path: path.to_owned(),
                line_number,
                line: line.trim().to_owned(),
            });
        }
    }
    joins
}

fn is_test_source_path(path: &str) -> bool {
    path.ends_with("_tests.rs")
        || path.ends_with("/tests.rs")
        || path.contains("/tests/")
        || path.contains("/architecture_tests/")
        || path.split('/').any(|segment| segment.ends_with("_tests"))
}

#[test]
fn artifact_path_scanner_ignores_test_code() {
    let source = r#"
fn bad(run_dir: &std::path::Path) {
    let _ = run_dir.join("session.json");
}

#[cfg(test)]
mod tests {
    fn fixture(run_dir: &std::path::Path) {
        let _ = run_dir.join("session.json");
    }
}
"#;

    assert_eq!(
        direct_artifact_path_joins_in_source(source, "src/recorder/session.rs"),
        vec![DirectArtifactPathJoin {
            path: "src/recorder/session.rs".to_owned(),
            line_number: 3,
            line: "let _ = run_dir.join(\"session.json\");".to_owned(),
        }]
    );
    assert!(
        direct_artifact_path_joins_in_source(source, "src/regression_tests/streaming.rs")
            .is_empty()
    );
}

#[test]
fn production_artifact_paths_use_artifact_kind_helpers() {
    let mut violations = Vec::new();
    for file in rust_files_under(&crate_src_root()) {
        for join in direct_artifact_path_joins_in_file(&file) {
            violations.push(format!(
                "{}:{} uses direct artifact filename join; use ArtifactKind + ArtifactPath/artifact_path instead: {}",
                join.path, join.line_number, join.line
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "artifact path guard failed:\n{}",
        violations.join("\n")
    );
}
