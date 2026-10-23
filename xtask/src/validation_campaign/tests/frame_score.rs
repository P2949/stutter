use std::{cell::RefCell, fs};

use super::{
    Scratch,
    capture_exec::{FakeHost, Script},
    fresh, manifest,
};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    frame_score::{FrameScoreRunner, level_spec},
    graph::Graph,
    model::FailureClass,
    repository::Binaries,
    scenarios::model::{ControlType, InterventionKind},
};

fn run(script: fn() -> Script, scratch: &Scratch) -> RunnerOutcome {
    let (dir, state, _) = fresh(scratch);
    let manifest = manifest();
    let binaries = Binaries {
        stutter: "s".into(),
        xtask: "x".into(),
    };
    let updates = RefCell::new(Vec::new());
    let context = RunContext {
        dir: &dir,
        state: &state,
        manifest: &manifest,
        repository: scratch.path(),
        binaries: &binaries,
        updates: &updates,
    };
    let spec = Graph::core().unwrap().get("frame-score").unwrap().clone();
    FrameScoreRunner {
        make_host: Box::new(move |_, _| Ok(Box::new(FakeHost::new(script())))),
    }
    .run(&context, &spec, 1)
}

#[test]
fn each_level_is_a_matched_experiment_with_only_the_degradation_varying() {
    let control = level_spec(0, 30);
    assert!(control.interventions.is_empty() && control.control == ControlType::NegativeControl);
    let degraded = level_spec(8, 30);
    assert_eq!(degraded.interventions, [InterventionKind::SchedulerBurst]);
    assert_eq!(control.target, degraded.target);
}

#[test]
fn every_level_is_captured_and_marked_complete_before_the_series_is_evaluated() {
    let scratch = Scratch::new("frame-score");
    let outcome = run(
        || Script {
            frames: 60,
            sanitize_writes_frames: true,
            ..Script::default()
        },
        &scratch,
    );
    // The scripted host writes no real session, so evaluation (the last stage) is what fails.
    let RunnerOutcome::Failed { stage, .. } = outcome else {
        panic!("a scripted recording cannot be a scored session");
    };
    assert_eq!(stage, "evaluate");
    let (dir, ..) = fresh(&scratch);
    let captures = dir.attempt_dir("frame-score", 1).join("captures");
    let marked = fs::read_dir(&captures)
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().join(".complete").is_file())
        .count();
    assert_eq!(marked, manifest().score.levels_ms.len());
}

#[test]
fn an_unverified_degradation_fails_that_level_and_keeps_the_finished_ones() {
    let scratch = Scratch::new("frame-score-bad");
    let outcome = run(
        || Script {
            frames: 60,
            unverified: true,
            ..Script::default()
        },
        &scratch,
    );
    let RunnerOutcome::Failed { class, stage, .. } = outcome else {
        panic!("an unverified burst must fail the level");
    };
    assert_eq!(
        (class, stage.as_str()),
        (FailureClass::Retryable, "level-1")
    );
    let (dir, ..) = fresh(&scratch);
    let captures = dir.attempt_dir("frame-score", 1).join("captures");
    assert!(captures.join("0001/.complete").is_file());
    assert!(!captures.join("0002/.complete").exists());
}
