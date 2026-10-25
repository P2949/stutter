//! What executes a stage. Stage logic is separated from the state machine so the driver is tested
//! with scripted runners and real hardware work stays behind typed, operation-specific code.
use std::path::Path;

use stutter::api::autotune::ObjectiveKind;

use super::{
    benefit::{Order, PairResult, Scenario},
    graph::Stage,
    manifest::{MutationManifest, OperationSection},
    model::MutationState,
};
use crate::validation_campaign::{
    model::{BlockReason, FailureClass},
    repository::Binaries,
    state::CampaignDir,
};

/// Everything a stage may read; all of it derives from campaign state and the manifest.
pub struct StageContext<'a> {
    pub repository: &'a Path,
    pub dir: &'a CampaignDir,
    pub state: &'a MutationState,
    pub manifest: &'a MutationManifest,
    pub operation: &'a OperationSection,
    pub binaries: &'a Binaries,
}

/// How a stage attempt ended.
#[derive(Debug)]
pub enum StageOutcome {
    Passed {
        artifacts: Option<String>,
        receipt: Option<String>,
        limitations: Vec<String>,
    },
    Failed {
        class: FailureClass,
        stage: String,
        summary: String,
        artifacts: Option<String>,
    },
    Blocked(BlockReason),
}

impl StageOutcome {
    pub fn passed(artifacts: Option<String>, receipt: Option<String>) -> Self {
        Self::Passed {
            artifacts,
            receipt,
            limitations: Vec::new(),
        }
    }

    pub fn failed(class: FailureClass, stage: &str, summary: impl Into<String>) -> Self {
        Self::Failed {
            class,
            stage: stage.to_owned(),
            summary: summary.into(),
            artifacts: None,
        }
    }

    pub fn blocked(class: FailureClass, detail: impl Into<String>) -> Self {
        Self::Blocked(BlockReason {
            class,
            detail: detail.into(),
        })
    }
}

pub trait StageRunner {
    fn run(&self, context: &StageContext<'_>, stage: Stage, attempt: u32) -> StageOutcome;
}

/// Why a benefit measurement did not produce a pair.
#[derive(Debug)]
pub enum HarnessError {
    Blocked(BlockReason),
    /// A recoverable measurement problem (one corrupted capture): retried within the budget.
    Retryable(String),
    /// The control path itself misbehaved.
    Defect(String),
}

/// Measures one matched pair of a scenario through the real control path (observation,
/// situation, provider, planner, runtime eligibility, live experiment, the objective's
/// comparison). The pair's baseline and candidate summaries are the objective's own populations.
pub trait BenefitHarness {
    fn measure(
        &self,
        context: &StageContext<'_>,
        objective: ObjectiveKind,
        scenario: Scenario,
        pair: usize,
        order: Order,
    ) -> Result<PairResult, HarnessError>;
}

/// The harness of a build that has none for the operation: an honest block, never a pass.
pub struct UnavailableHarness;

impl BenefitHarness for UnavailableHarness {
    fn measure(
        &self,
        _: &StageContext<'_>,
        _: ObjectiveKind,
        _: Scenario,
        _: usize,
        _: Order,
    ) -> Result<PairResult, HarnessError> {
        Err(HarnessError::Blocked(BlockReason {
            class: FailureClass::BlockedResource,
            detail: "no benefit harness is available for this operation in this build".to_owned(),
        }))
    }
}
