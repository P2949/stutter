//! The generic mutation-graduation stage order. Stages are linear: each depends on the previous
//! stage that runs. Only stages marked skippable can be left out, and only by an explicit,
//! reasoned entry in the operation's manifest section, never by executor choice.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preflight,
    Mechanical,
    FaultMatrix,
    SuspendResume,
    BenefitCalibration,
    BenefitHoldout,
    KernelMatrix,
    EvidenceIntegrate,
    TierDecision,
}

impl Stage {
    pub const ALL: [Self; 9] = [
        Self::Preflight,
        Self::Mechanical,
        Self::FaultMatrix,
        Self::SuspendResume,
        Self::BenefitCalibration,
        Self::BenefitHoldout,
        Self::KernelMatrix,
        Self::EvidenceIntegrate,
        Self::TierDecision,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Mechanical => "mechanical_apply_verify_rollback",
            Self::FaultMatrix => "fault_matrix",
            Self::SuspendResume => "suspend_resume",
            Self::BenefitCalibration => "benefit_calibration",
            Self::BenefitHoldout => "benefit_holdout",
            Self::KernelMatrix => "kernel_matrix",
            Self::EvidenceIntegrate => "evidence_integrate",
            Self::TierDecision => "tier_decision",
        }
    }

    /// Accepts the full stage id and the short manifest name `mechanical`.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|stage| stage.as_str() == name)
            .or((name == "mechanical").then_some(Self::Mechanical))
    }

    /// Whether the stage reads or produces evidence bound to the pinned source commit.
    pub fn source_bound(self) -> bool {
        !matches!(self, Self::EvidenceIntegrate | Self::TierDecision)
    }
}

/// Only the kernel matrix may be skipped, and only for operations whose behaviour does not
/// depend on a kernel interface. Every other stage is mandatory for a mutating capability.
pub fn may_skip(stage: Stage) -> bool {
    matches!(stage, Stage::KernelMatrix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_names_round_trip_and_the_order_is_total() {
        for stage in Stage::ALL {
            assert_eq!(Stage::parse(stage.as_str()), Some(stage));
        }
        assert_eq!(Stage::parse("mechanical"), Some(Stage::Mechanical));
        assert!(Stage::ALL.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(Stage::parse("nonsense").is_none());
    }

    #[test]
    fn only_the_kernel_matrix_is_skippable() {
        for stage in Stage::ALL {
            assert_eq!(may_skip(stage), stage == Stage::KernelMatrix);
        }
    }
}
