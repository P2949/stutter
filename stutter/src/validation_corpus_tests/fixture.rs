use std::{
    fs,
    path::{Path, PathBuf},
};

use super::expectation::*;
use crate::{
    diagnosis::Diagnosis,
    report::{self, ReportAnalysisJson},
    test_fixture_builder,
};

pub(super) fn fixture_path(name: &str) -> PathBuf {
    test_fixture_builder::fixture_path(name)
}

pub(super) fn load_fixture_toml(path: &Path) -> FixtureExpectationFile {
    let text = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!(
            "failed to read fixture metadata {}: {err:#}",
            path.display()
        )
    });
    toml::from_str(&text).unwrap_or_else(|err| {
        panic!(
            "failed to parse fixture metadata {}: {err:#}",
            path.display()
        )
    })
}

pub(super) fn build_fixture_analysis(name: &str) -> ReportAnalysisJson {
    report::build_report_analysis(&fixture_path(name), 10, 5, None)
        .unwrap_or_else(|err| panic!("failed to build analysis for {name}: {err:#}"))
}

pub(super) fn primary_diagnosis(analysis: &ReportAnalysisJson) -> Option<&Diagnosis> {
    analysis
        .cluster_analysis
        .clusters
        .iter()
        .filter_map(|cluster| cluster.diagnosis.as_ref())
        .next()
}
