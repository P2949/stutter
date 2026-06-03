use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::fixture::*;

#[derive(Debug, Serialize)]
pub struct ValidationCoverageReport {
    pub real_fixture_count: usize,
    pub synthetic_fixture_count: usize,
    pub distinct_capture_ids: usize,
    pub vendors: BTreeMap<String, usize>,
    pub compositors: BTreeMap<String, usize>,
    pub scenarios: BTreeMap<String, usize>,
    pub kernels: BTreeMap<String, usize>,
    pub known_false_positive_count: usize,
    pub known_false_negative_count: usize,
    pub missing_cells: Vec<String>,
}

const MIN_REAL_FIXTURES: usize = 20;
const MIN_FALSE_POSITIVE_FIXTURES: usize = 3;
const MIN_KNOWN_FALSE_NEGATIVE_FIXTURES: usize = 3;
const MIN_DISTINCT_CAPTURE_IDS: usize = 20;

fn build_validation_coverage_report() -> ValidationCoverageReport {
    let root = fixture_path("");
    let mut real_fixture_count = 0;
    let mut synthetic_fixture_count = 0;
    let mut vendors = BTreeMap::new();
    let mut compositors = BTreeMap::new();
    let mut scenarios = BTreeMap::new();
    let mut kernels = BTreeMap::new();
    let mut known_false_positive_count = 0;
    let mut known_false_negative_count = 0;
    let mut capture_ids = BTreeSet::new();

    for entry in std::fs::read_dir(&root).expect("fixture root should be readable") {
        let entry = entry.expect("fixture entry should be readable");
        if !entry.file_type().expect("fixture type").is_dir() {
            continue;
        }
        let spec = load_fixture_toml(&entry.path().join("fixture.toml"));
        let is_real = spec.name.starts_with("real_")
            || spec.source == "sanitized-real-recording"
            || spec.source == "validation-corpus";
        if is_real {
            real_fixture_count += 1;
        } else {
            synthetic_fixture_count += 1;
        }

        if spec.expected.expected_behavior
            == super::expectation::ExpectedDiagnosisBehavior::KnownMiss
        {
            known_false_negative_count += 1;
        }

        if let Some(platform) = spec.platform {
            if is_real {
                capture_ids.insert(platform.sanitized_capture_id.clone());
            }
            increment(&mut vendors, platform.gpu_vendor);
            increment(&mut compositors, platform.compositor);
            increment(&mut scenarios, platform.scenario.clone());
            if platform.scenario == "false-positive" {
                known_false_positive_count += 1;
            }
            if let Some(kernel) = platform.kernel_version_bucket {
                increment(&mut kernels, kernel);
            }
        }
    }

    let mut missing_cells = Vec::new();
    for required in ["AMD", "NVIDIA", "Intel"] {
        push_missing(&mut missing_cells, "vendor", required, &vendors);
    }
    for required in ["Sway", "Hyprland", "Gamescope", "KWin", "GNOME"] {
        push_missing(&mut missing_cells, "compositor", required, &compositors);
    }
    for required in [
        "clean",
        "false-positive",
        "cpu-bound",
        "gpu-bound",
        "irq",
        "compositor",
    ] {
        push_missing(&mut missing_cells, "scenario", required, &scenarios);
    }

    ValidationCoverageReport {
        real_fixture_count,
        synthetic_fixture_count,
        distinct_capture_ids: capture_ids.len(),
        vendors,
        compositors,
        scenarios,
        kernels,
        known_false_positive_count,
        known_false_negative_count,
        missing_cells,
    }
}

fn increment(map: &mut BTreeMap<String, usize>, key: String) {
    if key.trim().is_empty() {
        return;
    }
    *map.entry(key).or_default() += 1;
}

fn push_missing(
    missing: &mut Vec<String>,
    label: &str,
    required: &str,
    map: &BTreeMap<String, usize>,
) {
    if !map.contains_key(required) {
        missing.push(format!("{label}:{required}"));
    }
}

#[test]
fn validation_corpus_coverage_reports_required_matrix() {
    let report = build_validation_coverage_report();

    assert!(
        report.real_fixture_count >= MIN_REAL_FIXTURES,
        "real fixture count regressed: {report:?}"
    );
    assert!(
        report.missing_cells.is_empty(),
        "validation coverage missing cells: {:?}",
        report.missing_cells
    );
    assert!(
        report.known_false_positive_count >= MIN_FALSE_POSITIVE_FIXTURES,
        "expected at least {MIN_FALSE_POSITIVE_FIXTURES} tracked false-positive fixtures"
    );
    assert!(
        report.known_false_negative_count >= MIN_KNOWN_FALSE_NEGATIVE_FIXTURES,
        "expected at least {MIN_KNOWN_FALSE_NEGATIVE_FIXTURES} tracked known false-negative fixtures"
    );
    assert!(
        report.distinct_capture_ids >= MIN_DISTINCT_CAPTURE_IDS,
        "expected at least {MIN_DISTINCT_CAPTURE_IDS} distinct sanitized capture ids"
    );
    assert!(
        !report.kernels.is_empty(),
        "expected kernel buckets in real-matrix fixture metadata"
    );
}

#[test]
fn validation_corpus_tracks_known_false_negative_cases() {
    let parsed: super::expectation::FixtureExpectationFile = toml::from_str(
        r#"
name = "known_miss_fixture"
schema_version = 23
source = "sanitized-real-recording"
quality_expectation = "High"
description = "known miss parser coverage"

[expected]
primary_cause = "GpuBoundCandidate"
expected_behavior = "known_miss"
accepted_confidence = []
data_quality = "High"
"#,
    )
    .expect("known_miss metadata should parse");

    assert_eq!(
        parsed.expected.expected_behavior,
        super::expectation::ExpectedDiagnosisBehavior::KnownMiss
    );
}
