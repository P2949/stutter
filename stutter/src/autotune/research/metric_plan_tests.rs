use super::*;
use crate::autotune::research::{
    cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkMetrics,
    },
    frame_stats::FrameStats,
    native_context::NativeContext,
};

pub(crate) fn item(index: u32, stats: Option<FrameStats>, fps: f64) -> AttemptEvidenceV1 {
    let intent = crate::autotune::research::tests::intent();
    AttemptEvidenceV1 {
        long_session: None,
        schema_version: 1,
        slot_index: index,
        average_fps: fps,
        native_completion: Some(Box::new(CyberpunkBenchmarkCompletion {
            schema_version: 1,
            app_id: intent.app_id,
            build_id: intent.build_id,
            scene: CyberpunkScenario::BuiltinBenchmark,
            settings_sha256: "c".repeat(64),
            proton_source: intent.tools[0].source,
            proton_identity_sha256: intent.tools[0].identity_sha256.clone(),
            process: CyberpunkProcessIdentity {
                pid: stutter_core::ids::Pid::new(1234),
                start_ticks: 1,
                uid: intent.uid,
                app_id: intent.app_id,
                build_id: intent.build_id,
                executable_sha256: intent.game_executable_sha256.clone(),
            },
            started_unix_nanos: 1,
            completed_unix_nanos: 2,
            result_directory: "result".into(),
            metrics: NativeBenchmarkMetrics {
                average_fps: fps,
                minimum_fps: None,
                maximum_fps: None,
            },
            context: Some(NativeContext {
                game_version: Some("2.5".into()),
                preset_name: Some("p".into()),
                gpu_name: Some("g".into()),
                upscaling_type: Some(0),
                ray_tracing_enabled: Some(false),
                frame_generation_type: Some(0),
                frame_count: 2000,
                frame_stats: stats,
            }),
            loaded_runtime: None,
            runtime_binding: None,
            files: Vec::new(),
        })),
    }
}

fn stats(p99: f64) -> Option<FrameStats> {
    Some(FrameStats {
        frames: 2000,
        p99_frame_ms: p99,
        one_percent_low_fps: 1000.0 / p99,
    })
}

#[test]
fn the_tail_is_primary_only_when_every_attempt_has_a_known_population() {
    let all: BTreeMap<u32, _> = (0..4).map(|i| (i, item(i, stats(20.0), 60.0))).collect();
    let (metric, values) = values(&all, 0..4, None).unwrap();
    assert_eq!(metric, NativeMetric::P99FrameMs);
    assert!(values.values().all(|v| *v == 20.0));

    // One attempt without tail statistics sends the whole campaign to the average, never a mix.
    let mut mixed = all.clone();
    mixed.insert(2, item(2, None, 61.0));
    let (metric, values) = self::values(&mixed, 0..4, None).unwrap();
    assert_eq!(metric, NativeMetric::AverageFps);
    assert_eq!(values[&2], 61.0);
    assert_eq!(values[&0], 60.0);

    // Nothing completed: nothing to judge, the legacy metric.
    assert_eq!(choose([]), NativeMetric::AverageFps);
}

#[test]
fn a_completed_attempt_without_evidence_is_an_error_not_a_skipped_value() {
    let evidence: BTreeMap<u32, _> = [(0, item(0, stats(20.0), 60.0))].into();
    assert!(values(&evidence, 0..2, None).is_err());
}

#[test]
fn a_non_finite_or_non_positive_tail_is_not_a_known_population() {
    for bad in [f64::NAN, 0.0, -3.0] {
        let evidence: BTreeMap<u32, _> = [(0, item(0, stats(bad), 60.0))].into();
        assert_eq!(
            values(&evidence, 0..1, None).unwrap().0,
            NativeMetric::AverageFps
        );
    }
}

#[test]
fn a_frozen_rule_keeps_its_metric_and_evidence_that_cannot_supply_it_is_an_error() {
    let with_stats: BTreeMap<u32, _> = (0..2).map(|i| (i, item(i, stats(20.0), 60.0))).collect();
    let without: BTreeMap<u32, _> = (0..2).map(|i| (i, item(i, None, 60.0))).collect();
    // Frozen on the average: the tail the evidence has is not used.
    assert_eq!(
        values(&with_stats, 0..2, Some(false)).unwrap().0,
        NativeMetric::AverageFps
    );
    // Frozen on the tail: evidence without it fails rather than falling back.
    assert_eq!(
        values(&with_stats, 0..2, Some(true)).unwrap().0,
        NativeMetric::P99FrameMs
    );
    assert!(values(&without, 0..2, Some(true)).is_err());
}

#[test]
fn run_values_are_tail_fps_for_every_run_or_the_average_for_every_run() {
    let natives = |stats_for: [Option<FrameStats>; 3]| -> Vec<_> {
        stats_for
            .into_iter()
            .enumerate()
            .map(|(i, s)| *item(i as u32, s, 60.0).native_completion.unwrap())
            .collect()
    };
    let all = natives([stats(20.0), stats(25.0), stats(20.0)]);
    let (tail, values) = run_values(all.iter());
    assert!(tail);
    assert_eq!(
        values.iter().map(|(_, v)| *v).collect::<Vec<_>>(),
        [50.0, 40.0, 50.0]
    );
    let mixed = natives([stats(20.0), None, stats(20.0)]);
    let (tail, values) = run_values(mixed.iter());
    assert!(!tail);
    assert!(values.iter().all(|(_, v)| *v == 60.0));
    assert_eq!(run_values([]), (false, Vec::new()));
}
