use super::*;
use crate::{
    process_tree::TaskClass,
    tune::comparability::{ScoredIdentityCount, TaskIdentity},
};

fn high_input() -> OnlineDataQualityInput<'static> {
    OnlineDataQualityInput {
        scored_intervals: DEFAULT_MIN_SCORED_INTERVALS,
        scored_samples: DEFAULT_MIN_SCORED_SAMPLES,
        scored_task_count: 1,
        drop_counter_total: 0,
        target_identity_shifted: false,
        target_present: true,
        frame_data_required: false,
        frame_count: 0,
        baseline_frame_count: None,
        candidate_frame_count: None,
        baseline_scored_identity_counts: &[],
        candidate_scored_identity_counts: &[],
    }
}

fn identity(comm: &str) -> ScoredIdentityCount {
    ScoredIdentityCount {
        identity: TaskIdentity {
            class: TaskClass::Game,
            process_comm: "game".to_owned(),
            comm: comm.to_owned(),
            process_starttime_ticks: Some(100),
            task_starttime_ticks: Some(200),
            exe_dev: Some(1),
            exe_ino: Some(2),
        },
        count: 100,
    }
}

#[test]
fn high_when_all_required_online_gates_pass() {
    let quality = OnlineDataQuality::evaluate(high_input());

    assert_eq!(quality, OnlineDataQuality::High);
    assert!(!quality.blocks_action());
}

#[test]
fn default_policy_does_not_require_frame_data() {
    let policy = OnlineDataQualityPolicy::default();
    let input = high_input();

    assert_eq!(policy.frame_data_policy, DEFAULT_FRAME_DATA_POLICY);
    assert_eq!(policy.frame_data_policy, FrameDataPolicy::Advisory);
    assert_eq!(input.evaluate_with_policy(&policy), OnlineDataQuality::High);
}

#[test]
fn policy_required_frame_data_without_frames_is_low_quality() {
    let policy = OnlineDataQualityPolicy {
        frame_data_policy: FrameDataPolicy::Required,
        ..OnlineDataQualityPolicy::default()
    };
    let input = high_input();

    let quality = input.evaluate_with_policy(&policy);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("no frame data"))
    );
}

#[test]
fn ignore_frame_data_policy_skips_frame_count_mismatch() {
    let mut input = high_input();
    input.frame_count = 100;
    input.baseline_frame_count = Some(100);
    input.candidate_frame_count = Some(0);
    let policy = OnlineDataQualityPolicy {
        frame_data_policy: FrameDataPolicy::Ignore,
        ..OnlineDataQualityPolicy::default()
    };
    assert_eq!(input.evaluate_with_policy(&policy), OnlineDataQuality::High);
}
#[test]
fn low_when_scored_intervals_are_below_policy_minimum() {
    let mut input = high_input();
    input.scored_intervals = DEFAULT_MIN_SCORED_INTERVALS - 1;

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("fewer than min_scored_intervals"))
    );
}

#[test]
fn low_when_scored_samples_are_below_policy_minimum() {
    let mut input = high_input();
    input.scored_samples = DEFAULT_MIN_SCORED_SAMPLES - 1;

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("fewer than min_scored_samples"))
    );
}

#[test]
fn low_when_scored_task_count_is_zero() {
    let mut input = high_input();
    input.scored_task_count = 0;

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason == "zero scored tasks")
    );
}

#[test]
fn low_when_drop_counters_exceed_policy_max() {
    let mut input = high_input();
    input.drop_counter_total = 1;

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("drop counters above policy max"))
    );
}

#[test]
fn low_when_target_identity_shifted() {
    let mut input = high_input();
    input.target_identity_shifted = true;

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("target identity shifted during candidate measurement"))
    );
}

#[test]
fn low_when_target_disappeared() {
    let mut input = high_input();
    input.target_present = false;

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason == "target disappeared")
    );
}

#[test]
fn low_when_frame_data_required_but_missing() {
    let mut input = high_input();
    input.frame_data_required = true;
    input.frame_count = 0;

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("no frame data"))
    );
}

#[test]
fn low_when_frame_count_has_zero_nonzero_mismatch() {
    let mut input = high_input();
    input.frame_data_required = true;
    input.frame_count = 100;
    input.baseline_frame_count = Some(100);
    input.candidate_frame_count = Some(0);

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("one window has frames and the other has none"))
    );
}

#[test]
fn low_when_scored_identity_overlap_is_below_tune_threshold() {
    let baseline = vec![identity("render")];
    let candidate = vec![identity("worker")];

    let input = OnlineDataQualityInput {
        baseline_scored_identity_counts: &baseline,
        candidate_scored_identity_counts: &candidate,
        ..high_input()
    };

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_low());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("scored identity overlap"))
    );
}

#[test]
fn medium_when_frame_count_differs_but_not_enough_for_low() {
    let mut input = high_input();
    input.baseline_frame_count = Some(100);
    input.candidate_frame_count = Some(130);

    let quality = OnlineDataQuality::evaluate(input);

    assert!(quality.is_medium());
    assert!(
        quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("frame count differs"))
    );
}

#[test]
fn reason_codes_are_stable_and_deduplicated_for_low_quality() {
    let mut input = high_input();
    input.scored_intervals = 0;
    input.scored_samples = 0;
    input.drop_counter_total = 7;
    input.target_present = false;

    let quality = OnlineDataQuality::evaluate(input);

    assert_eq!(
        quality.reason_code_strings(),
        vec![
            "insufficient_samples".to_owned(),
            "target_missing".to_owned(),
            "drop_counters_high".to_owned()
        ]
    );
    assert_eq!(
        quality.primary_reason_code(),
        Some(OnlineDataQualityReasonCode::InsufficientSamples)
    );
}

#[test]
fn reason_codes_cover_workload_frame_focus_and_thermal_reasons() {
    let quality = OnlineDataQuality::Low {
        reasons: vec![
            "target identity shifted during candidate measurement".to_owned(),
            "frame data mismatch when frame-based scoring is required: no frame data".to_owned(),
            "focus confidence below policy threshold".to_owned(),
            "thermal degraded".to_owned(),
        ],
    };

    assert_eq!(
        quality.reason_code_strings(),
        vec![
            "focus_low_confidence".to_owned(),
            "workload_changed".to_owned(),
            "thermal_degraded".to_owned(),
            "frame_data_invalid".to_owned()
        ]
    );
}

#[test]
fn medium_quality_reason_codes_include_frame_count_mismatch() {
    let mut input = high_input();
    input.baseline_frame_count = Some(100);
    input.candidate_frame_count = Some(130);

    let quality = OnlineDataQuality::evaluate(input);

    assert_eq!(
        quality.reason_code_strings(),
        vec!["frame_count_mismatch".to_owned()]
    );
}
