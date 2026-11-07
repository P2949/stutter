//! The frozen manifest of one real-workload comparison.
//!
//! Everything that decides a result is fixed here before any measurement: the workload, the
//! instrumentation (identical in both arms), the counterbalanced sequence, the primary metric, the
//! effect and equivalence thresholds and the guardrails. `frozen_digest` identifies the frozen
//! text, so a later change to a threshold is visible as a different manifest, not a quiet edit.
use std::collections::BTreeMap;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

use crate::evidence_bundle::sha256::sha256_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// An owned synthetic workload (vkcube and similar): never real-game benefit evidence.
    ControlledReal,
    /// An unattended run of a real game with a repeatable benchmark or replay.
    RealGame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    Native,
    Steam,
    Proton,
    Gamescope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    LowerIsBetter,
    HigherIsBetter,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub unit: String,
    pub direction: Direction,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guardrail {
    pub metric: Metric,
    /// Largest tolerated regression, in percent of the baseline, at the confidence bound.
    pub max_regression_percent: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub evidence_kind: EvidenceKind,
    pub source_commit: String,
    pub binary_sha256: String,
    pub workload: String,
    pub launch_mode: LaunchMode,
    pub work_unit: String,
    /// Instrumentation (for example MangoHud) is the same in both arms; this records which.
    pub instrumentation: Vec<String>,
    /// Kernel, driver, compositor, display, refresh, cap, HDR and VRR settings.
    pub context: BTreeMap<String, String>,
    pub warmup_seconds: u64,
    pub measure_seconds: u64,
    pub cooldown_seconds: u64,
    pub timeout_seconds: u64,
    pub max_pairs: u32,
    pub candidate: String,
    pub primary: Metric,
    /// The smallest improvement, in percent, that counts as a meaningful gain.
    pub min_effect_percent: f64,
    /// Changes within this many percent either way count as equivalent.
    pub equivalence_margin_percent: f64,
    pub guardrails: Vec<Guardrail>,
    pub abort_rules: Vec<String>,
}

/// The fewest pairs from which an effect and its uncertainty may be judged.
pub const MIN_PAIRS: u32 = 4;
pub const MAX_PAIRS_LIMIT: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    Baseline,
    Treatment,
}

impl Manifest {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.schema_version == 1, "unsupported schema_version");
        for (name, value, len) in [
            ("source_commit", &self.source_commit, 40),
            ("binary_sha256", &self.binary_sha256, 64),
        ] {
            ensure!(
                value.len() == len && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "{name} must be {len} hex characters"
            );
        }
        ensure!(!self.workload.trim().is_empty(), "workload is empty");
        ensure!(!self.work_unit.trim().is_empty(), "work_unit is empty");
        ensure!(!self.candidate.trim().is_empty(), "candidate is empty");
        ensure!(self.measure_seconds > 0, "measure_seconds must be positive");
        ensure!(
            self.timeout_seconds
                >= self.warmup_seconds + self.measure_seconds + self.cooldown_seconds,
            "timeout_seconds is shorter than one measurement"
        );
        ensure!(
            (MIN_PAIRS..=MAX_PAIRS_LIMIT).contains(&self.max_pairs),
            "max_pairs must be {MIN_PAIRS}..={MAX_PAIRS_LIMIT}: trials are capped, never repeated until beneficial"
        );
        ensure!(
            self.equivalence_margin_percent > 0.0
                && self.min_effect_percent > self.equivalence_margin_percent,
            "min_effect_percent must exceed a positive equivalence_margin_percent"
        );
        ensure!(
            !self.guardrails.is_empty(),
            "at least one guardrail is required"
        );
        for guardrail in &self.guardrails {
            ensure!(
                guardrail.max_regression_percent > 0.0
                    && guardrail.metric.name != self.primary.name,
                "a guardrail must be a different metric with a positive limit"
            );
        }
        ensure!(
            !self.abort_rules.is_empty(),
            "abort rules must be declared before the run"
        );
        Ok(())
    }

    /// SHA-256 of the canonical serialized manifest.
    pub fn frozen_digest(&self) -> anyhow::Result<String> {
        Ok(sha256_hex(
            serde_json::to_string(self)
                .context("serialize manifest")?
                .as_bytes(),
        ))
    }

    /// The arm of `slot` (0-based) in the counterbalanced order: pair 0 is A then B, pair 1 is B
    /// then A, and so on, so neither arm systematically runs first or warm.
    pub fn arm_at(&self, slot: u32) -> Arm {
        let pair = slot / 2;
        let first = slot.is_multiple_of(2);
        match (pair.is_multiple_of(2), first) {
            (true, true) | (false, false) => Arm::Baseline,
            (true, false) | (false, true) => Arm::Treatment,
        }
    }
}
