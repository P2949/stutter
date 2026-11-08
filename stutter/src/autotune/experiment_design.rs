//! Immutable experiment design and evidence (package W10).
//!
//! Everything that decides a result is fixed in the design before any measurement: objective,
//! unit, candidate, practical effect, equivalence margin, seed, the block ids and the stopping
//! rule. A fixed-N design is analysed once, at its planned size. A sequential design is analysed at
//! most its planned number of times at an error level that already pays for every look. Looking
//! early at a fixed-N design is refused, because repeated unadjusted inspection manufactures
//! significance.
use serde::{Deserialize, Serialize};

use super::statistics::{Interval, bonferroni_confidence, paired_interval};
use crate::daemon::state::daemon_profile_stable_hash;

pub const EXPERIMENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum StoppingRule {
    /// One analysis, at exactly `blocks` independent blocks, at the given overall confidence.
    FixedN { blocks: u32, confidence: f64 },
    /// At most `looks` analyses sharing one total error budget `alpha`.
    Sequential {
        looks: u32,
        blocks_per_look: u32,
        alpha: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentDesign {
    pub schema_version: u32,
    pub objective: String,
    pub metric: String,
    pub unit: String,
    pub candidate: String,
    /// Source, build, boot, topology and owner context this design applies to.
    pub context_fingerprint: String,
    pub min_effect_percent: f64,
    pub equivalence_margin_percent: f64,
    pub seed: u64,
    pub block_ids: Vec<String>,
    pub stopping: StoppingRule,
    pub warmup_seconds: u64,
    pub washout_seconds: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesignError {
    TooFewBlocks,
    EffectNotAboveMargin,
    BadStoppingRule,
    DuplicateBlockId,
    BlockIdsDoNotCoverThePlan,
    /// A practical-effect or margin threshold is NaN or infinite.
    NonFiniteThreshold,
    /// `looks * blocks_per_look` does not fit: the plan is impossible, not merely large.
    PlanOverflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Beneficial,
    Equivalent,
    Harmful,
    /// Not enough to decide; with a sequential design and looks left, collect more.
    Uncertain,
    InvalidExperiment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisRefused {
    /// A fixed-N design may only be analysed at its planned size.
    EarlyLookAtFixedN,
    /// More looks than the design paid for.
    LooksExhausted,
    TooFewBlocks,
    /// The design's own plan overflows, so there is no expected block count to analyse against.
    ImpossiblePlan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub look: u32,
    pub blocks: usize,
    pub confidence: f64,
    pub interval: (f64, f64, f64),
    pub decision: Decision,
}

impl ExperimentDesign {
    pub fn validate(&self) -> Result<(), DesignError> {
        if !self.min_effect_percent.is_finite() || !self.equivalence_margin_percent.is_finite() {
            return Err(DesignError::NonFiniteThreshold);
        }
        if self.equivalence_margin_percent <= 0.0
            || self.min_effect_percent <= self.equivalence_margin_percent
        {
            return Err(DesignError::EffectNotAboveMargin);
        }
        let planned = match self.stopping {
            StoppingRule::FixedN { blocks, confidence } => {
                if !(confidence > 0.0 && confidence < 1.0) {
                    return Err(DesignError::BadStoppingRule);
                }
                blocks
            }
            StoppingRule::Sequential {
                looks,
                blocks_per_look,
                alpha,
            } => {
                if looks < 2 || blocks_per_look < 2 || bonferroni_confidence(alpha, looks).is_none()
                {
                    return Err(DesignError::BadStoppingRule);
                }
                looks
                    .checked_mul(blocks_per_look)
                    .ok_or(DesignError::PlanOverflow)?
            }
        };
        if planned < 2 {
            return Err(DesignError::TooFewBlocks);
        }
        let unique: std::collections::BTreeSet<&String> = self.block_ids.iter().collect();
        if unique.len() != self.block_ids.len() {
            return Err(DesignError::DuplicateBlockId);
        }
        if self.block_ids.len() < planned as usize {
            return Err(DesignError::BlockIdsDoNotCoverThePlan);
        }
        Ok(())
    }

    /// A stable identity of the frozen design; evidence names it, so a changed threshold is a
    /// different design and never a quiet edit.
    pub fn digest(&self) -> String {
        daemon_profile_stable_hash([serde_json::to_string(self).unwrap_or_default().as_str()])
    }

    /// Analyses the per-block improvements (percent, positive is better) at `look` (1-based).
    /// `drifted` marks a block set whose scenario, owner or source changed: it is invalid, not a
    /// result.
    pub fn analyse(
        &self,
        improvements: &[f64],
        look: u32,
        drifted: bool,
    ) -> Result<Analysis, AnalysisRefused> {
        let (confidence, expected_blocks, last_look) = match self.stopping {
            StoppingRule::FixedN { blocks, confidence } => {
                if improvements.len() != blocks as usize || look != 1 {
                    return Err(AnalysisRefused::EarlyLookAtFixedN);
                }
                (confidence, blocks as usize, true)
            }
            StoppingRule::Sequential {
                looks,
                blocks_per_look,
                alpha,
            } => {
                if look == 0 || look > looks {
                    return Err(AnalysisRefused::LooksExhausted);
                }
                let confidence =
                    bonferroni_confidence(alpha, looks).ok_or(AnalysisRefused::TooFewBlocks)?;
                let expected = look
                    .checked_mul(blocks_per_look)
                    .ok_or(AnalysisRefused::ImpossiblePlan)?;
                (confidence, expected as usize, look == looks)
            }
        };
        if improvements.len() < expected_blocks || improvements.len() < 2 {
            return Err(AnalysisRefused::TooFewBlocks);
        }
        let interval = paired_interval(&improvements[..expected_blocks], confidence)
            .ok_or(AnalysisRefused::TooFewBlocks)?;
        let decision = if drifted {
            Decision::InvalidExperiment
        } else {
            self.classify(&interval, last_look)
        };
        Ok(Analysis {
            look,
            blocks: expected_blocks,
            confidence,
            interval: (interval.low, interval.mean, interval.high),
            decision,
        })
    }

    fn classify(&self, interval: &Interval, last_look: bool) -> Decision {
        if interval.low >= self.min_effect_percent {
            Decision::Beneficial
        } else if interval.high <= -self.equivalence_margin_percent {
            Decision::Harmful
        } else if interval.low > -self.equivalence_margin_percent
            && interval.high < self.equivalence_margin_percent
        {
            Decision::Equivalent
        } else if last_look && interval.high < self.min_effect_percent {
            // Too small to matter, though not shown equivalent: no meaningful gain.
            Decision::Equivalent
        } else {
            Decision::Uncertain
        }
    }
}

/// What a finished experiment records: the design it ran, every analysis, and the final decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentEvidence {
    pub design_digest: String,
    pub analyses: Vec<Analysis>,
    pub decision: Decision,
    pub overhead_note: String,
}

impl ExperimentEvidence {
    /// Evidence is acceptable only if it names this exact design, never exceeds its looks, and
    /// every recorded analysis is what that design would have produced: the look numbers run
    /// 1, 2, ... in order, each analysis used the planned block count and confidence for its
    /// look, its interval is finite and ordered, and its decision is the design's own
    /// classification of that interval. Only the final analysis may have stopped the experiment,
    /// so every earlier one is `Uncertain`, and the evidence's decision is the final analysis's.
    /// A matching digest alone proves nothing: a decision is checked against the numbers.
    pub fn consistent_with(&self, design: &ExperimentDesign) -> bool {
        if self.design_digest != design.digest() || design.validate().is_err() {
            return false;
        }
        let (looks_allowed, last_look_number) = match design.stopping {
            StoppingRule::FixedN { .. } => (1, 1),
            StoppingRule::Sequential { looks, .. } => (looks as usize, looks),
        };
        let Some(final_analysis) = self.analyses.last() else {
            return false;
        };
        if self.analyses.len() > looks_allowed || final_analysis.decision != self.decision {
            return false;
        }
        let last_index = self.analyses.len() - 1;
        self.analyses.iter().enumerate().all(|(index, analysis)| {
            let look = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let (confidence, blocks) = match design.stopping {
                StoppingRule::FixedN { blocks, confidence } => (confidence, blocks as usize),
                StoppingRule::Sequential {
                    looks,
                    blocks_per_look,
                    alpha,
                } => {
                    let (Some(confidence), Some(blocks)) = (
                        bonferroni_confidence(alpha, looks),
                        look.checked_mul(blocks_per_look),
                    ) else {
                        return false;
                    };
                    (confidence, blocks as usize)
                }
            };
            let (low, mean, high) = analysis.interval;
            let ordered = [low, mean, high].iter().all(|value| value.is_finite())
                && low <= mean
                && mean <= high;
            let interval = Interval {
                mean,
                low,
                high,
                blocks,
                confidence,
            };
            // A drifted block set is recorded as invalid whatever its numbers say; any other
            // decision must be exactly what the numbers classify to.
            let decision_follows = analysis.decision == Decision::InvalidExperiment
                || analysis.decision == design.classify(&interval, look == last_look_number);
            let stops_early = analysis.decision != Decision::Uncertain;
            analysis.look == look
                && analysis.blocks == blocks
                && analysis.confidence == confidence
                && ordered
                && decision_follows
                // Only the last analysis may have ended the experiment.
                && (index == last_index || !stops_early)
        })
    }
}

#[cfg(test)]
#[path = "experiment_design_tests.rs"]
mod tests;
