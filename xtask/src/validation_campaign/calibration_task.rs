//! The two deterministic policy tasks that read collected evidence: diagnosis calibration and
//! StutterScore review. Neither tunes anything. With sufficient evidence the current thresholds
//! and weights are retained and every weak result becomes a recorded limitation; with
//! insufficient evidence the campaign stops instead of deciding.
use std::{fs, path::Path};

use serde_json::{Value, json};

use super::{
    executor::{RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::TaskSpec,
    integration::FIXTURES,
    model::{AttemptOutcome, BlockReason, FailureClass},
    state::write_atomic,
};
use crate::diagnosis_calibration::{self, FAMILIES};

/// Calibration report of one family; injectable for tests.
pub type Calibrator = dyn Fn(&Path, &str) -> anyhow::Result<Value>;

pub struct DiagnosisCalibrationRunner {
    pub calibrate: Box<Calibrator>,
}

impl DiagnosisCalibrationRunner {
    pub fn system() -> Self {
        Self {
            calibrate: Box::new(|root, family| {
                diagnosis_calibration::report(root, Path::new(FIXTURES), family)
            }),
        }
    }
}

fn blocked(class: FailureClass, detail: String) -> RunnerOutcome {
    RunnerOutcome::Blocked(BlockReason { class, detail })
}

/// Limitations a family's behavior at its *current* threshold implies.
pub fn family_limitations(report: &Value) -> Vec<String> {
    let family = report["family"].as_str().unwrap_or("?");
    let current = &report["current_threshold"];
    let Some(at) = report["sweep"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| &row["threshold"] == current))
    else {
        return vec![format!(
            "family {family}: the sweep has no entry at the current threshold"
        )];
    };
    let mut limits = Vec::new();
    let precision = at["precision"].as_f64();
    let recall = at["recall"].as_f64();
    if precision.is_some_and(|p| p < 0.9) || recall.is_some_and(|r| r < 0.9) {
        limits.push(format!(
            "family {family}: precision {precision:?} / recall {recall:?} at the current threshold are below 0.9; the threshold is retained and the diagnosis stays Experimental"
        ));
    }
    for (key, label) in [
        ("false_high_observation_confidence", "observation"),
        ("false_high_causal_confidence", "causal"),
    ] {
        if at[key].as_u64().unwrap_or(0) > 0 {
            limits.push(format!(
                "family {family}: {} negative control(s) were called a family positive with High {label} confidence",
                at[key]
            ));
        }
    }
    limits
}

impl TaskRunner for DiagnosisCalibrationRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let out = context.dir.0.join("calibration");
        if let Err(error) = fs::create_dir_all(&out) {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "storage".to_owned(),
                summary: error.to_string(),
                artifacts: None,
            };
        }
        let (mut limitations, mut families) = (Vec::new(), serde_json::Map::new());
        for spec in &FAMILIES {
            let report = match (self.calibrate)(context.repository, spec.name) {
                Ok(report) => report,
                Err(error) => {
                    return RunnerOutcome::Failed {
                        class: FailureClass::SoftwareDefect,
                        stage: spec.name.to_owned(),
                        summary: format!("{error:#}"),
                        artifacts: None,
                    };
                }
            };
            if report["sufficiency"]["sufficient"] != true {
                return blocked(
                    FailureClass::BlockedResource,
                    format!(
                        "the {} evidence floor is not met ({}): #32 cannot complete",
                        spec.name, report["sufficiency"]
                    ),
                );
            }
            let written = serde_json::to_vec_pretty(&report)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    write_atomic(&out.join(format!("{}.json", spec.name)), &bytes)
                        .map_err(|e| e.to_string())
                });
            if let Err(error) = written {
                return RunnerOutcome::Failed {
                    class: FailureClass::Retryable,
                    stage: "storage".to_owned(),
                    summary: error,
                    artifacts: None,
                };
            }
            limitations.extend(family_limitations(&report));
            families.insert(
                spec.name.to_owned(),
                json!({"decision": "retain_current_threshold", "sweep_complete": report["sweep"].as_array().is_some_and(|s| !s.is_empty())}),
            );
        }
        let review = json!({
            "policy": "retain thresholds when floors are sufficient; never optimize against one corpus",
            "families": families,
            "limitations": limitations,
            "diagnosis_capabilities": "remain Experimental for v0.1.0",
        });
        if let Err(error) = write_atomic(&out.join("review.json"), review.to_string().as_bytes()) {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "storage".to_owned(),
                summary: error.to_string(),
                artifacts: None,
            };
        }
        RunnerOutcome::Passed {
            artifacts: Some(context.dir.relative(&out)),
            receipt: None,
            limitations: limitations.clone(),
            fingerprint: Box::default(),
            updates: vec![StateUpdate::Decision {
                decision: "retain_current_diagnosis_thresholds".to_owned(),
                limitations,
            }],
        }
    }
}

pub struct ScoreReviewRunner;

fn passed_attempt_dir(context: &RunContext<'_>, task: &str) -> Option<std::path::PathBuf> {
    let state = context.state.tasks.get(task)?;
    let attempt = state
        .attempts
        .iter()
        .rev()
        .find(|a| matches!(a.outcome, AttemptOutcome::Passed))?;
    Some(context.dir.0.join(attempt.artifacts.as_deref()?))
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// The first published `result.json` of a task's bundle.
fn bundle_result(context: &RunContext<'_>, task: &str) -> Option<Value> {
    let mut dirs: Vec<_> = fs::read_dir(context.dir.bundles().join(task))
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            !p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    dirs.sort();
    read_json(&dirs.first()?.join("artifacts/result.json"))
}

impl TaskRunner for ScoreReviewRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let Some(frame) = passed_attempt_dir(context, "frame-score")
            .and_then(|d| read_json(&d.join("evaluation.json")))
        else {
            return blocked(
                FailureClass::BlockedResource,
                "the frame-score evaluation is missing".to_owned(),
            );
        };
        let Some(runnable) = bundle_result(context, "score-runnable") else {
            return blocked(
                FailureClass::BlockedResource,
                "the runnable score series bundle is missing".to_owned(),
            );
        };
        let runnable_monotonic =
            runnable["measurements"]["evaluation"]["monotonic_in_degradation"] == true;
        let frame_monotonic = frame["monotonic_in_degradation"] == true;
        let mut limitations = Vec::new();
        if !runnable_monotonic {
            limitations.push("the score is not monotonic in the controlled runnable series; weights retained, tune_evaluation stays Experimental".to_owned());
        }
        if !frame_monotonic {
            limitations.push("the score is not monotonic in the frame series; weights retained, tune_evaluation stays Experimental".to_owned());
        }
        let out = context.dir.0.join("score");
        let review = json!({
            "weights": "retained (100 x >5 ms, 20 x >2 ms, 1 x >1 ms)",
            "runnable_monotonic": runnable_monotonic,
            "frame_monotonic": frame_monotonic,
            "primaries_unchanged": ["GameRunnableLatency: runnable latency", "CompileThroughputWithForegroundProtection: fixed-work elapsed time"],
            "limitations": limitations,
        });
        let written = fs::create_dir_all(&out)
            .map_err(|e| e.to_string())
            .and_then(|()| {
                write_atomic(&out.join("review.json"), review.to_string().as_bytes())
                    .map_err(|e| e.to_string())
            });
        if let Err(error) = written {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "storage".to_owned(),
                summary: error,
                artifacts: None,
            };
        }
        RunnerOutcome::Passed {
            artifacts: Some(context.dir.relative(&out)),
            receipt: None,
            limitations: limitations.clone(),
            fingerprint: Box::default(),
            updates: vec![StateUpdate::Decision {
                decision: "retain_stutter_score_weights".to_owned(),
                limitations,
            }],
        }
    }
}
