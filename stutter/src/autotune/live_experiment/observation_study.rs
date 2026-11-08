//! Frozen paired-block analysis of authenticated KPI observations. This has no apply/keep path.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::LiveExperimentManager;
use crate::autotune::{
    experiment_design::{ExperimentDesign, ExperimentEvidence},
    objective::source::{Direction, KpiContract, ObservedKpiBlock},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KpiObservationStudyConfig {
    pub design: ExperimentDesign,
    pub contract: KpiContract,
    /// Exact completed-work/input identity, fixed before collecting the first pair.
    #[serde(default)]
    pub work_unit: String,
    /// Exact maximum regressions, percent, fixed before the first block. No default threshold.
    pub hard_guardrail_regression_percent: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KpiObservationStudyEvidence {
    pub frozen_config_sha256: String,
    pub prepared_monotonic_ns: u64,
    /// Hashes of the serialized raw captures, in baseline/treatment pairs, in design order.
    pub raw_block_sha256: Vec<(String, String)>,
    pub evidence: ExperimentEvidence,
    pub authorizes_mutation: bool,
}

#[derive(Debug)]
pub(super) struct ObservationStudy {
    config: KpiObservationStudyConfig,
    prepared_ns: u64,
    improvements: Vec<f64>,
    raw_hashes: Vec<(String, String)>,
    analyses: Vec<crate::autotune::experiment_design::Analysis>,
    run_ids: BTreeSet<String>,
    source_session: Option<String>,
    last_end_ns: Option<u64>,
    invalid: bool,
}

fn improvement(direction: Direction, baseline: f64, treatment: f64) -> anyhow::Result<f64> {
    anyhow::ensure!(
        baseline.is_finite() && treatment.is_finite() && baseline >= 0.0 && treatment >= 0.0,
        "invalid measurement"
    );
    if baseline == 0.0 {
        anyhow::ensure!(
            treatment == 0.0,
            "percent effect undefined for a zero baseline"
        );
        return Ok(0.0);
    }
    let effect = match direction {
        Direction::LowerIsBetter => (baseline - treatment) / baseline * 100.0,
        Direction::HigherIsBetter => (treatment - baseline) / baseline * 100.0,
    };
    anyhow::ensure!(effect.is_finite(), "effect overflow");
    Ok(effect)
}

impl LiveExperimentManager {
    /// Freeze before collection. The immutable design and guardrails belong to this observation
    /// study; source declarations never alter the operation registry or the manager's action state.
    pub fn begin_kpi_observation_study(
        &mut self,
        config: KpiObservationStudyConfig,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.observation_study.is_none(),
            "an observation study is already frozen"
        );
        anyhow::ensure!(
            !self.has_active_experiment() && self.pending_objective_evidence.is_none(),
            "an action experiment is already active"
        );
        config
            .design
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid design: {error:?}"))?;
        config
            .contract
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid contract: {error:?}"))?;
        anyhow::ensure!(
            config.design.schema_version
                == crate::autotune::experiment_design::EXPERIMENT_SCHEMA_VERSION
                && config.design.block_ids.len() <= 128,
            "unsupported or unbounded study"
        );
        anyhow::ensure!(
            config.design.metric == config.contract.primary.id
                && config.design.unit == config.contract.primary.unit,
            "design metric/unit differs from frozen contract"
        );
        anyhow::ensure!(
            config.design.min_effect_percent == config.contract.primary.meaningful_effect_percent
                && config.design.equivalence_margin_percent
                    == config.contract.primary.equivalent_within_percent,
            "design thresholds differ from frozen contract"
        );
        anyhow::ensure!(
            !config.design.context_fingerprint.is_empty(),
            "missing study context"
        );
        anyhow::ensure!(
            !config.work_unit.is_empty()
                && config.work_unit.len() <= 128
                && !config.work_unit.chars().any(char::is_control),
            "missing or invalid frozen work identity"
        );
        anyhow::ensure!(
            config.hard_guardrail_regression_percent.len() == config.contract.hard_guardrails.len(),
            "guardrail set is not exact"
        );
        for guard in &config.contract.hard_guardrails {
            anyhow::ensure!(
                config
                    .hard_guardrail_regression_percent
                    .get(&guard.id)
                    .is_some_and(|limit| limit.is_finite() && *limit >= 0.0),
                "missing/invalid guardrail limit {}",
                guard.id
            );
        }
        self.observation_study = Some(ObservationStudy {
            config,
            prepared_ns: crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC)?,
            improvements: Vec::new(),
            raw_hashes: Vec::new(),
            analyses: Vec::new(),
            run_ids: BTreeSet::new(),
            source_session: None,
            last_end_ns: None,
            invalid: false,
        });
        Ok(())
    }

    /// Consumes actual collector captures, never deserialized claims about aggregate values.
    /// Both metrics and statistical intervals are reconstructed from the captured raw samples.
    pub fn observe_kpi_pair(
        &mut self,
        baseline: &ObservedKpiBlock,
        treatment: &ObservedKpiBlock,
    ) -> anyhow::Result<()> {
        let study = self
            .observation_study
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no frozen observation study"))?;
        let result = study.add_pair(baseline, treatment);
        if result.is_err() {
            study.invalid = true;
        }
        result
    }

    pub fn analyse_kpi_observation_study(
        &mut self,
        look: u32,
    ) -> anyhow::Result<KpiObservationStudyEvidence> {
        let study = self
            .observation_study
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no frozen observation study"))?;
        anyhow::ensure!(
            look as usize == study.analyses.len() + 1,
            "replayed or out-of-order analysis look"
        );
        anyhow::ensure!(
            study
                .analyses
                .last()
                .is_none_or(|analysis| analysis.decision
                    == crate::autotune::experiment_design::Decision::Uncertain),
            "study has already stopped"
        );
        let analysis = study
            .config
            .design
            .analyse(&study.improvements, look, study.invalid)
            .map_err(|error| anyhow::anyhow!("analysis refused: {error:?}"))?;
        anyhow::ensure!(
            analysis.blocks == study.improvements.len(),
            "look must occur at its preregistered block count"
        );
        study.analyses.push(analysis.clone());
        let evidence = ExperimentEvidence { design_digest: study.config.design.digest(), analyses: study.analyses.clone(), decision: analysis.decision, overhead_note: "Observation-only; each statistical unit is an independently completed workload block. No operation authority or calibration is granted.".into() };
        anyhow::ensure!(
            evidence.consistent_with(&study.config.design),
            "reconstructed evidence does not match the frozen plan"
        );
        Ok(KpiObservationStudyEvidence {
            frozen_config_sha256: stutter_core::hash::sha256_hex(&serde_json::to_vec(
                &study.config,
            )?),
            raw_block_sha256: study.raw_hashes.clone(),
            prepared_monotonic_ns: study.prepared_ns,
            evidence,
            authorizes_mutation: false,
        })
    }
}

impl KpiObservationStudyEvidence {
    /// Hash verification is only storage integrity. Recompute measurements and every interval
    /// from collector-origin raw captures as well. This still grants no autonomous authority.
    pub fn verify_captures(
        &self,
        config: &KpiObservationStudyConfig,
        pairs: &[(&ObservedKpiBlock, &ObservedKpiBlock)],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.authorizes_mutation
                && self.frozen_config_sha256
                    == stutter_core::hash::sha256_hex(&serde_json::to_vec(config)?),
            "evidence config/authority mismatch"
        );
        anyhow::ensure!(
            self.evidence.consistent_with(&config.design),
            "internally inconsistent evidence"
        );
        let mut manager = LiveExperimentManager::new();
        manager.begin_kpi_observation_study(config.clone())?;
        let study = manager
            .observation_study
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("observation study was not initialized"))?;
        study.prepared_ns = self.prepared_monotonic_ns;
        for (baseline, treatment) in pairs {
            study.add_pair(baseline, treatment)?;
        }
        anyhow::ensure!(
            study.raw_hashes == self.raw_block_sha256,
            "raw capture hashes differ"
        );
        anyhow::ensure!(
            self.evidence
                .analyses
                .last()
                .is_some_and(|analysis| analysis.blocks == pairs.len()),
            "final analysis does not cover all captured blocks"
        );
        for supplied in &self.evidence.analyses {
            anyhow::ensure!(
                supplied.blocks <= study.improvements.len(),
                "missing raw measurement blocks"
            );
            let reconstructed = config
                .design
                .analyse(
                    &study.improvements[..supplied.blocks],
                    supplied.look,
                    study.invalid,
                )
                .map_err(|error| anyhow::anyhow!("raw analysis refused: {error:?}"))?;
            anyhow::ensure!(
                reconstructed == *supplied,
                "reported interval/decision differs from raw blocks"
            );
        }
        Ok(())
    }
}

impl ObservationStudy {
    fn add_pair(
        &mut self,
        baseline: &ObservedKpiBlock,
        treatment: &ObservedKpiBlock,
    ) -> anyhow::Result<()> {
        let design = &self.config.design;
        let index = self.improvements.len();
        let max_blocks = match design.stopping {
            crate::autotune::experiment_design::StoppingRule::FixedN { blocks, .. } => blocks,
            crate::autotune::experiment_design::StoppingRule::Sequential {
                looks,
                blocks_per_look,
                ..
            } => looks
                .checked_mul(blocks_per_look)
                .ok_or_else(|| anyhow::anyhow!("study size overflow"))?,
        };
        anyhow::ensure!(
            index < max_blocks as usize
                && self.analyses.last().is_none_or(|analysis| analysis.decision
                    == crate::autotune::experiment_design::Decision::Uncertain),
            "study already reached its stopping limit"
        );
        anyhow::ensure!(
            index < design.block_ids.len()
                && baseline.block_id() == design.block_ids[index]
                && treatment.block_id() == design.block_ids[index],
            "duplicate, unplanned or out-of-order block"
        );
        anyhow::ensure!(
            baseline.context() == design.context_fingerprint
                && treatment.context() == design.context_fingerprint,
            "measurement context drifted"
        );
        anyhow::ensure!(
            baseline.work_unit() == self.config.work_unit
                && treatment.work_unit() == self.config.work_unit,
            "work inputs differ"
        );
        anyhow::ensure!(
            baseline.source_session() == treatment.source_session()
                && self
                    .source_session
                    .as_deref()
                    .is_none_or(|session| session == baseline.source_session()),
            "source session changed"
        );
        anyhow::ensure!(
            baseline.run_id() != treatment.run_id()
                && !self.run_ids.contains(baseline.run_id())
                && !self.run_ids.contains(treatment.run_id()),
            "completed run was replayed"
        );
        let (b_from, b_to) = baseline.interval();
        let (t_from, t_to) = treatment.interval();
        let first = b_from.min(t_from);
        let end = b_to.max(t_to);
        anyhow::ensure!(
            first >= self.prepared_ns
                && (b_to < t_from || t_to < b_from)
                && self.last_end_ns.is_none_or(|previous| first > previous),
            "pre-design, overlapping or reused measurement interval"
        );
        let baseline_first = index as u64 % 2 == design.seed % 2;
        anyhow::ensure!(
            (b_from < t_from) == baseline_first,
            "pair violates frozen counterbalanced arm order"
        );
        let gap = if baseline_first {
            t_from.saturating_sub(b_to)
        } else {
            b_from.saturating_sub(t_to)
        };
        anyhow::ensure!(
            gap >= design
                .washout_seconds
                .saturating_add(design.warmup_seconds)
                .saturating_mul(1_000_000_000),
            "pair omits preregistered washout/warmup"
        );
        let base = baseline.recompute(&self.config.contract)?;
        let treat = treatment.recompute(&self.config.contract)?;
        let primary = &self.config.contract.primary;
        let effect = improvement(
            primary.direction,
            base[&primary.id].value,
            treat[&primary.id].value,
        )?;
        for guard in &self.config.contract.hard_guardrails {
            let good = improvement(
                guard.direction,
                base[&guard.id].value,
                treat[&guard.id].value,
            )
            .is_ok_and(|effect| {
                effect >= -self.config.hard_guardrail_regression_percent[&guard.id]
            });
            if !good {
                self.invalid = true;
            }
        }
        self.raw_hashes
            .push((baseline.artifact_sha256()?, treatment.artifact_sha256()?));
        self.run_ids.insert(baseline.run_id().to_owned());
        self.run_ids.insert(treatment.run_id().to_owned());
        self.source_session = Some(baseline.source_session().to_owned());
        self.last_end_ns = Some(end);
        self.improvements.push(effect);
        Ok(())
    }
}

#[cfg(test)]
#[path = "observation_study_tests.rs"]
mod tests;
