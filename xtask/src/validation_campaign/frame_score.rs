//! The `frame-score` task: one frame-producing recording per controlled scheduler-degradation
//! level, evaluated with the production score and the recorded frame stream. Each level's
//! recording is retained and marked complete only after it passed its checks, so a killed run
//! resumes at the first unfinished level and never reuses a half-written recording.
use std::{fs, path::Path};

use super::{
    executor::{RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::TaskSpec,
    model::{BlockReason, FailureClass},
    scenarios::{
        exec::CaptureHost,
        model::{ControlType, Family, InterventionKind, ScenarioSpec, Target},
    },
};
use crate::score_evaluation;

/// The contender period every degradation level shares; only the burst length varies.
pub const PERIOD_MS: u64 = 32;

pub type LevelHostMaker = dyn Fn(&RunContext<'_>, u32) -> Result<Box<dyn CaptureHost>, BlockReason>;

pub struct FrameScoreRunner {
    pub make_host: Box<LevelHostMaker>,
}

/// The experiment of one level, expressed as a scenario so the same capture machinery drives it.
pub fn level_spec(level_ms: u32, seconds: u64) -> ScenarioSpec {
    ScenarioSpec {
        id: format!("score-level-{level_ms}"),
        control: if level_ms == 0 {
            ControlType::NegativeControl
        } else {
            ControlType::Positive
        },
        covers: vec![Family::Scheduler],
        target: Target::VkmarkLight,
        interventions: if level_ms == 0 {
            Vec::new()
        } else {
            vec![InterventionKind::SchedulerBurst]
        },
        expected_primary_cause: if level_ms == 0 {
            "Unknown".to_owned()
        } else {
            Family::Scheduler.positive_cause().to_owned()
        },
        seconds,
        count: 1,
        accepted_wakeup_coalescing: 0,
    }
}

fn failed(stage: &str, summary: String, class: FailureClass) -> RunnerOutcome {
    RunnerOutcome::Failed {
        class,
        stage: stage.to_owned(),
        summary,
        artifacts: None,
    }
}

/// One level end to end; whatever happens, nothing it started is left running.
fn capture_level(
    host: &mut dyn CaptureHost,
    spec: &ScenarioSpec,
    raw: &Path,
) -> Result<(), String> {
    if let Some(dir) = raw.parent() {
        host.set_capture_dir(dir);
    }
    let result = (|| {
        host.preflight(spec)?;
        let target = host.start_target(spec)?;
        host.start_interventions(spec, target)?;
        let summary = host.record(spec, target, raw)?;
        let reports = host.verify_interventions(spec, raw);
        if let Some(bad) = reports.iter().find(|r| !r.verified) {
            return Err(format!(
                "intervention {:?} was not verified: {}",
                bad.kind, bad.detail
            ));
        }
        if summary.drops != 0 || summary.write_errors != 0 || summary.frame_samples == 0 {
            return Err(format!(
                "recording unusable: {} drops, {} write errors, {} frame samples",
                summary.drops, summary.write_errors, summary.frame_samples
            ));
        }
        Ok(())
    })();
    host.stop_all();
    result
}

impl TaskRunner for FrameScoreRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let captures = context.dir.attempt_dir(spec.id, attempt).join("captures");
        let seconds = context.manifest.overhead.seconds.max(20);
        let mut levels = Vec::new();
        for (index, level) in context.manifest.score.levels_ms.iter().enumerate() {
            let slot = captures.join(format!("{:04}", index + 1));
            let raw = slot.join("raw");
            if !slot.join(".complete").is_file() {
                let _ = fs::remove_dir_all(&slot);
                let mut host = match (self.make_host)(context, *level) {
                    Ok(host) => host,
                    Err(block) => return RunnerOutcome::Blocked(block),
                };
                if let Err(error) = capture_level(host.as_mut(), &level_spec(*level, seconds), &raw)
                {
                    return failed(
                        &format!("level-{level}"),
                        format!("degradation level {level} ms: {error}"),
                        FailureClass::Retryable,
                    );
                }
                if fs::create_dir_all(&slot)
                    .and_then(|()| fs::write(slot.join(".complete"), b"ok\n"))
                    .is_err()
                {
                    return failed(
                        "storage",
                        "could not mark the level complete".into(),
                        FailureClass::Retryable,
                    );
                }
            }
            levels.push((f64::from(*level), raw));
        }
        let evaluation = match score_evaluation::run_from_recordings(&levels, true) {
            Ok(value) => value,
            Err(error) => {
                return failed("evaluate", format!("{error:#}"), FailureClass::Retryable);
            }
        };
        let artifacts = context.dir.attempt_dir(spec.id, attempt);
        let written = serde_json::to_vec_pretty(&evaluation)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                fs::write(artifacts.join("evaluation.json"), bytes).map_err(|e| e.to_string())
            });
        if let Err(error) = written {
            return failed("storage", error, FailureClass::Retryable);
        }
        let monotonic = evaluation["monotonic_in_degradation"] == true;
        let decision = evaluation["weights_decision"]
            .as_str()
            .unwrap_or("review")
            .to_owned();
        let limitations = if monotonic {
            Vec::new()
        } else {
            vec![format!(
                "the score is not monotonic in controlled degradation ({} adjacent inversions)",
                evaluation["adjacent_inversions"]
            )]
        };
        RunnerOutcome::Passed {
            artifacts: Some(context.dir.relative(&artifacts)),
            receipt: None,
            limitations: limitations.clone(),
            fingerprint: Box::default(),
            updates: vec![StateUpdate::Decision {
                decision,
                limitations,
            }],
        }
    }
}
