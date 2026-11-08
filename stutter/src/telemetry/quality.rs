//! Health of one frame stream, and whether it may be frame-primary.
use serde::{Deserialize, Serialize};

use super::frame_source::{Attribution, MeasurementStage};

/// Alignment uncertainty above which frames cannot be placed in a measurement window.
pub const MAX_ALIGNMENT_UNCERTAINTY_MS: f64 = 2.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamQuality {
    pub attribution: Attribution,
    pub stage: MeasurementStage,
    /// Gaps in the source's own sequence not explained by a known cause.
    pub unexplained_sequence_gaps: u64,
    /// Frames dropped between source and window (parser, channel, bus).
    pub dropped_frames: u64,
    pub alignment_uncertainty_ms: f64,
    pub source_healthy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityDenial {
    NotValidatedDirect,
    NotApplicationFrame,
    UnexplainedLoss,
    AlignmentUncertain,
    SourceUnhealthy,
}

impl StreamQuality {
    /// `Ok` only for a verified application-frame stream with zero unexplained loss, so a
    /// wrong-game, stale or lossy stream can never produce a frame-primary keep.
    pub fn frame_primary_eligible(&self) -> Result<(), QualityDenial> {
        if self.attribution != Attribution::ValidatedDirect {
            return Err(QualityDenial::NotValidatedDirect);
        }
        if self.stage != MeasurementStage::ApplicationFrame {
            return Err(QualityDenial::NotApplicationFrame);
        }
        if !self.source_healthy {
            return Err(QualityDenial::SourceUnhealthy);
        }
        if self.unexplained_sequence_gaps > 0 || self.dropped_frames > 0 {
            return Err(QualityDenial::UnexplainedLoss);
        }
        if !self.alignment_uncertainty_ms.is_finite()
            || self.alignment_uncertainty_ms < 0.0
            || self.alignment_uncertainty_ms > MAX_ALIGNMENT_UNCERTAINTY_MS
        {
            return Err(QualityDenial::AlignmentUncertain);
        }
        Ok(())
    }
}
