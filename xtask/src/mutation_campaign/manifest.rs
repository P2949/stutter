//! The committed mutation-graduation manifest (`docs/validation/mutation-graduation.toml`). Its
//! SHA-256 is part of every stage fingerprint, so editing it invalidates source-bound results.
use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use super::graph::{self, Stage};
use crate::evidence_bundle::sha256::sha256_hex;

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "docs/validation/mutation-graduation.toml";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetrySection {
    pub default_attempts: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BenefitSection {
    pub initial_pairs: usize,
    pub max_pairs: usize,
    pub step: usize,
    pub calibration_parts: usize,
    pub holdout_parts: usize,
    pub min_beneficial_keep_rate: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkipEntry {
    pub stage: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SuiteMap {
    pub mechanical: Option<String>,
    pub fault_matrix: Option<String>,
    pub suspend_resume: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScenarioMap {
    pub beneficial: String,
    pub neutral: String,
    pub regression: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ThresholdGrid {
    pub min_improvement_percent: Vec<f64>,
    pub max_regression_percent: Vec<f64>,
    pub min_samples: Vec<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OperationSection {
    pub operation: String,
    pub objective: String,
    pub capability: String,
    pub workload: String,
    pub pressure: String,
    pub effect_scope: String,
    pub stages: Vec<String>,
    #[serde(default)]
    pub skip: Vec<SkipEntry>,
    pub suites: SuiteMap,
    pub scenarios: ScenarioMap,
    pub thresholds: ThresholdGrid,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MutationManifest {
    pub schema_version: u32,
    pub name: String,
    pub retry: RetrySection,
    pub benefit: BenefitSection,
    #[serde(default)]
    pub operation: BTreeMap<String, OperationSection>,
}

impl MutationManifest {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(text).context("mutation manifest is not valid")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Reads and validates the manifest, returning it with the digest of its exact bytes.
    pub fn load(path: &Path) -> anyhow::Result<(Self, String)> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let text = std::str::from_utf8(&bytes).context("manifest is not UTF-8")?;
        Ok((Self::parse(text)?, sha256_hex(&bytes)))
    }

    pub fn operation(&self, name: &str) -> anyhow::Result<&OperationSection> {
        self.operation.get(name).with_context(|| {
            format!(
                "no operation {name:?} in the manifest (known: {})",
                self.operation
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == MANIFEST_SCHEMA_VERSION,
            "unsupported manifest schema {}",
            self.schema_version
        );
        ensure!(!self.name.trim().is_empty(), "campaign name is empty");
        ensure!(
            self.retry.default_attempts >= 1,
            "retry attempts must be >= 1"
        );
        let benefit = &self.benefit;
        ensure!(
            benefit.initial_pairs >= 1
                && benefit.step >= 1
                && benefit.max_pairs >= benefit.initial_pairs
                && benefit.calibration_parts >= 1
                && benefit.holdout_parts >= 1
                && benefit.min_beneficial_keep_rate > 0.0
                && benefit.min_beneficial_keep_rate <= 1.0,
            "benefit pair counts and the split must be positive and ordered"
        );
        ensure!(!self.operation.is_empty(), "the manifest has no operation");
        for (name, operation) in &self.operation {
            operation
                .validate()
                .with_context(|| format!("operation {name}"))?;
            ensure!(
                operation.operation == *name,
                "operation section {name} names operation {}",
                operation.operation
            );
        }
        Ok(())
    }
}

impl OperationSection {
    /// The stages this operation runs, in the fixed generic order.
    pub fn stage_list(&self) -> anyhow::Result<Vec<Stage>> {
        let mut stages = Vec::new();
        for name in &self.stages {
            stages.push(Stage::parse(name).with_context(|| format!("unknown stage {name:?}"))?);
        }
        Ok(stages)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let stages = self.stage_list()?;
        ensure!(
            stages.windows(2).all(|pair| pair[0] < pair[1]),
            "stages must be listed once, in the generic order"
        );
        // A stage may be left out only by an explicit, reasoned skip, and only if the generic
        // graph allows skipping it.
        let skipped: Vec<Stage> = self
            .skip
            .iter()
            .map(|entry| {
                ensure!(
                    !entry.reason.trim().is_empty(),
                    "skip of {} has no reason",
                    entry.stage
                );
                Stage::parse(&entry.stage)
                    .with_context(|| format!("unknown stage {:?}", entry.stage))
            })
            .collect::<anyhow::Result<_>>()?;
        for stage in Stage::ALL {
            let listed = stages.contains(&stage);
            let skip = skipped.contains(&stage);
            ensure!(
                !(listed && skip),
                "stage {} is both run and skipped",
                stage.as_str()
            );
            if !listed {
                if !skip {
                    bail!(
                        "stage {} is neither run nor explicitly skipped",
                        stage.as_str()
                    );
                }
                ensure!(
                    graph::may_skip(stage),
                    "stage {} is required and cannot be skipped",
                    stage.as_str()
                );
            }
        }
        ensure!(
            self.suites.mechanical.is_some() && self.suites.fault_matrix.is_some(),
            "mechanical and fault_matrix suites must be named"
        );
        ensure!(
            !self.thresholds.min_improvement_percent.is_empty()
                && !self.thresholds.max_regression_percent.is_empty()
                && !self.thresholds.min_samples.is_empty(),
            "the threshold grid must not be empty"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn committed() -> MutationManifest {
        let text = include_str!("../../../docs/validation/mutation-graduation.toml");
        MutationManifest::parse(text).unwrap()
    }

    #[test]
    fn the_committed_manifest_is_valid_and_names_affinity() {
        let manifest = committed();
        let affinity = manifest.operation("cpu_affinity_profile").unwrap();
        assert_eq!(affinity.objective, "game_runnable_latency");
        assert!(
            affinity
                .stage_list()
                .unwrap()
                .contains(&Stage::SuspendResume)
        );
        assert!(
            !affinity
                .stage_list()
                .unwrap()
                .contains(&Stage::KernelMatrix)
        );
    }

    #[test]
    fn a_stage_cannot_vanish_without_an_explicit_reasoned_skip() {
        let text = include_str!("../../../docs/validation/mutation-graduation.toml")
            .replace("  \"suspend_resume\",\n", "");
        let error = MutationManifest::parse(&text).unwrap_err();
        assert!(format!("{error:#}").contains("suspend_resume"), "{error:#}");
    }

    #[test]
    fn a_skip_needs_a_reason_and_only_optional_stages_may_be_skipped() {
        let text = include_str!("../../../docs/validation/mutation-graduation.toml")
            .replace("  \"preflight\",\n", "")
            .replace(
                "skip = [",
                "skip = [\n  { stage = \"preflight\", reason = \"no\" },",
            );
        let error = MutationManifest::parse(&text).unwrap_err();
        assert!(
            format!("{error:#}").contains("cannot be skipped"),
            "{error:#}"
        );
        let empty = include_str!("../../../docs/validation/mutation-graduation.toml").replace(
            "reason = \"per-process",
            "reason = \"\", note = \"per-process",
        );
        assert!(MutationManifest::parse(&empty).is_err());
    }

    #[test]
    fn stage_order_and_operation_names_are_enforced() {
        let text = include_str!("../../../docs/validation/mutation-graduation.toml").replace(
            "[operation.cpu_affinity_profile]\noperation = \"cpu_affinity_profile\"",
            "[operation.cpu_affinity_profile]\noperation = \"other\"",
        );
        assert!(MutationManifest::parse(&text).is_err());
    }
}
