use std::{cell::RefCell, fs};

use serde_json::{Value, json};

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    calibration_task::{DiagnosisCalibrationRunner, ScoreReviewRunner, family_limitations},
    executor::{RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::Graph,
    model::{AttemptOutcome, FailureClass, TaskAttempt, TaskFingerprint},
    repository::Binaries,
};

fn report(family: &str, sufficient: bool, precision: f64, false_high: u64) -> Value {
    json!({"family": family, "current_threshold": 2.0,
        "sufficiency": {"sufficient": sufficient},
        "sweep": [{"threshold": 1.0, "precision": 0.1, "recall": 0.1},
            {"threshold": 2.0, "precision": precision, "recall": 1.0,
             "false_high_observation_confidence": 0, "false_high_causal_confidence": false_high}]})
}

#[test]
fn weak_behavior_at_the_current_threshold_becomes_limitations_not_a_change() {
    assert!(family_limitations(&report("io", true, 1.0, 0)).is_empty());
    let weak = family_limitations(&report("io", true, 0.5, 2));
    assert_eq!(weak.len(), 2, "{weak:?}");
    assert!(weak[0].contains("Experimental") && weak[1].contains("High causal"));
    assert!(
        family_limitations(&json!({"family": "gpu", "current_threshold": 9.0, "sweep": []}))[0]
            .contains("no entry")
    );
}

fn run_calibration(sufficient_for: fn(&str) -> bool, weak: bool) -> (RunnerOutcome, Scratch) {
    let scratch = Scratch::new("calibration");
    let (dir, state, _) = fresh(&scratch);
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
    let runner = DiagnosisCalibrationRunner {
        calibrate: Box::new(move |_, family| {
            Ok(report(
                family,
                sufficient_for(family),
                if weak { 0.4 } else { 1.0 },
                0,
            ))
        }),
    };
    let spec = Graph::core()
        .unwrap()
        .get("diagnosis-calibration")
        .unwrap()
        .clone();
    let outcome = runner.run(&context, &spec, 1);
    (outcome, scratch)
}

#[test]
fn sufficient_evidence_retains_every_threshold_and_persists_all_reports() {
    let (outcome, scratch) = run_calibration(|_| true, true);
    let RunnerOutcome::Passed {
        updates,
        limitations,
        ..
    } = outcome
    else {
        panic!("expected a retain decision")
    };
    assert!(
        matches!(&updates[0], StateUpdate::Decision { decision, .. } if decision == "retain_current_diagnosis_thresholds")
    );
    assert_eq!(
        limitations.len(),
        4,
        "one limitation per weak family: {limitations:?}"
    );
    let (dir, ..) = fresh(&scratch);
    for name in ["scheduler", "io", "irq", "gpu", "review"] {
        assert!(
            dir.0
                .join("calibration")
                .join(format!("{name}.json"))
                .is_file(),
            "{name}"
        );
    }
}

#[test]
fn one_insufficient_family_stops_the_campaign_instead_of_deciding() {
    let (outcome, _) = run_calibration(|family| family != "irq", false);
    let RunnerOutcome::Blocked(block) = outcome else {
        panic!("insufficient evidence must block")
    };
    assert_eq!(block.class, FailureClass::BlockedResource);
    assert!(block.detail.contains("irq"));
}

fn attempt(artifacts: &str) -> TaskAttempt {
    TaskAttempt {
        number: 1,
        started_unix_ms: 0,
        finished_unix_ms: 0,
        outcome: AttemptOutcome::Passed,
        fingerprint: TaskFingerprint::default(),
        artifacts: Some(artifacts.to_owned()),
        receipt: None,
        limitations: Vec::new(),
    }
}

fn score_review(frame_monotonic: bool, runnable_monotonic: bool) -> RunnerOutcome {
    let scratch = Scratch::new("score-review");
    let (dir, mut state, _) = fresh(&scratch);
    let frame_dir = dir.attempt_dir("frame-score", 1);
    fs::create_dir_all(&frame_dir).unwrap();
    fs::write(
        frame_dir.join("evaluation.json"),
        json!({"monotonic_in_degradation": frame_monotonic}).to_string(),
    )
    .unwrap();
    state
        .tasks
        .get_mut("frame-score")
        .unwrap()
        .attempts
        .push(attempt(&dir.relative(&frame_dir)));
    let bundle = dir.bundles().join("score-runnable/cap-1/artifacts");
    fs::create_dir_all(&bundle).unwrap();
    fs::write(
        bundle.join("result.json"),
        json!({"measurements": {"evaluation": {"monotonic_in_degradation": runnable_monotonic}}})
            .to_string(),
    )
    .unwrap();
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
    let spec = Graph::core().unwrap().get("score-review").unwrap().clone();
    ScoreReviewRunner.run(&context, &spec, 1)
}

#[test]
fn the_score_weights_are_always_retained_and_non_monotonic_series_are_limitations() {
    for (frame, runnable, limits) in [
        (true, true, 0),
        (false, true, 1),
        (true, false, 1),
        (false, false, 2),
    ] {
        let RunnerOutcome::Passed {
            limitations,
            updates,
            ..
        } = score_review(frame, runnable)
        else {
            panic!("review must pass")
        };
        assert_eq!(limitations.len(), limits);
        assert!(
            matches!(&updates[0], StateUpdate::Decision { decision, .. } if decision == "retain_stutter_score_weights")
        );
    }
}

#[test]
fn a_missing_series_blocks_the_review() {
    let scratch = Scratch::new("score-review-missing");
    let (dir, state, _) = fresh(&scratch);
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
    let spec = Graph::core().unwrap().get("score-review").unwrap().clone();
    assert!(matches!(
        ScoreReviewRunner.run(&context, &spec, 1),
        RunnerOutcome::Blocked(_)
    ));
}
