use serde::{Deserialize, Serialize};

use crate::autotune::{
    comparison::{
        ExperimentComparisonInput, ExperimentDataQuality, ExperimentResult, compare_experiment,
    },
    experiment::WindowScore,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveKind {
    StutterScore,
    GameFramePacing,
    GameRunnableLatency,
    DesktopInteractivity,
    BrowserInteractivity,
    CompileThroughputWithForegroundProtection,
    IoLatency,
    IrqOverlapReduction,
    ThermalRecovery,
}

impl Default for ObjectiveKind {
    fn default() -> Self {
        Self::StutterScore
    }
}

#[derive(Clone, Debug)]
pub struct ObjectiveComparisonInput<'a> {
    pub objective: ObjectiveKind,
    pub baseline: &'a WindowScore,
    pub candidate: &'a WindowScore,
    pub data_quality: ExperimentDataQuality,
    pub target_disappeared: bool,
}

pub fn compare_for_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult {
    let base = compare_experiment(ExperimentComparisonInput {
        baseline: input.baseline,
        candidate: input.candidate,
        data_quality: input.data_quality,
        target_disappeared: input.target_disappeared,
    });

    if !matches!(base, ExperimentResult::Improved { .. }) {
        return base;
    }

    match input.objective {
        ObjectiveKind::GameFramePacing | ObjectiveKind::GameRunnableLatency => {
            reject_if_frame_pacing_regressed(input.baseline, input.candidate).unwrap_or(base)
        }
        ObjectiveKind::DesktopInteractivity | ObjectiveKind::BrowserInteractivity => {
            reject_if_interactivity_regressed(input.baseline, input.candidate).unwrap_or(base)
        }
        ObjectiveKind::CompileThroughputWithForegroundProtection => {
            reject_if_foreground_regressed(input.baseline, input.candidate).unwrap_or(base)
        }
        ObjectiveKind::StutterScore
        | ObjectiveKind::IoLatency
        | ObjectiveKind::IrqOverlapReduction
        | ObjectiveKind::ThermalRecovery => base,
    }
}

fn reject_if_frame_pacing_regressed(
    baseline: &WindowScore,
    candidate: &WindowScore,
) -> Option<ExperimentResult> {
    if candidate.score.frame_p99_ms > baseline.score.frame_p99_ms + 1.0 {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    if candidate.score.over_5ms > baseline.score.over_5ms {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    None
}

fn reject_if_interactivity_regressed(
    baseline: &WindowScore,
    candidate: &WindowScore,
) -> Option<ExperimentResult> {
    if candidate.score.over_5ms > baseline.score.over_5ms {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    None
}

fn reject_if_foreground_regressed(
    baseline: &WindowScore,
    candidate: &WindowScore,
) -> Option<ExperimentResult> {
    if candidate.score.frame_p99_ms > baseline.score.frame_p99_ms + 2.0
        || candidate.score.over_5ms > baseline.score.over_5ms
    {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::StutterScore;

    fn score(total: u64, frame_p99_ms: f64, over_5ms: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 1,
            finished_unix_nanos: 2,
            interval_count: 5,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total,
                frame_p99_ms,
                over_5ms,
                ..StutterScore::default()
            },
        }
    }

    #[test]
    fn game_objective_rejects_frame_p99_regression_even_when_total_improves() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 14.0, 1);

        let result = compare_for_objective(ObjectiveComparisonInput {
            objective: ObjectiveKind::GameFramePacing,
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        });

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn desktop_objective_rejects_over_5ms_regression() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 2);

        let result = compare_for_objective(ObjectiveComparisonInput {
            objective: ObjectiveKind::DesktopInteractivity,
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        });

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn default_score_objective_keeps_existing_improvement_path() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);

        let result = compare_for_objective(ObjectiveComparisonInput {
            objective: ObjectiveKind::StutterScore,
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        });

        assert!(matches!(result, ExperimentResult::Improved { .. }));
    }
}
