//! The reported native context decides whether a campaign's runs are one comparable measurement.

use super::*;

#[test]
fn a_changed_or_missing_game_reported_context_is_not_a_consistent_comparison() {
    use super::super::super::{
        cyberpunk::{
            CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
            NativeBenchmarkMetrics,
        },
        native_context::NativeContext,
    };
    let context = |preset: &str| NativeContext {
        game_version: Some("2.5".into()),
        preset_name: Some(preset.into()),
        gpu_name: Some("GPU".into()),
        upscaling_type: Some(2),
        ray_tracing_enabled: Some(true),
        frame_generation_type: Some(0),
        frame_count: 100,
    };
    let with = |index: u32, ctx: Option<NativeContext>| {
        let native = CyberpunkBenchmarkCompletion {
            schema_version: 1,
            app_id: 1_091_500,
            build_id: 7,
            scene: CyberpunkScenario::BuiltinBenchmark,
            settings_sha256: "c".repeat(64),
            proton_source: ProtonSource::Ge,
            proton_identity_sha256: "d".repeat(64),
            process: CyberpunkProcessIdentity {
                pid: stutter_core::ids::Pid::new(1234),
                start_ticks: 1,
                uid: 1000,
                app_id: 1_091_500,
                build_id: 7,
                executable_sha256: "e".repeat(64),
            },
            started_unix_nanos: 1,
            completed_unix_nanos: 2,
            result_directory: "result".into(),
            metrics: NativeBenchmarkMetrics {
                average_fps: 60.0,
                minimum_fps: None,
                maximum_fps: None,
            },
            context: ctx,
            loaded_runtime: Some(matched_runtime()),
            runtime_binding: Some(super::super::super::loaded_runtime::RuntimeBinding::Matched),
            files: vec![],
        };
        (
            index,
            AttemptEvidenceV1 {
                schema_version: 1,
                slot_index: index,
                average_fps: 60.0,
                native_completion: Some(Box::new(native)),
            },
        )
    };
    let same: BTreeMap<_, _> = [
        with(0, Some(context("RayTracingUltra"))),
        with(1, Some(context("RayTracingUltra"))),
    ]
    .into();
    assert_eq!(context_state(&same), ContextState::Consistent);
    // The built-in benchmark is a fixed fly-through, so a run of another length is a scene that did not
    // repeat: 150 frames at the same FPS is half as long again, while 103 is within the real runs' spread.
    let with_frames = |frames: u64| {
        let mut map = same.clone();
        map.get_mut(&1)
            .unwrap()
            .native_completion
            .as_mut()
            .unwrap()
            .context
            .as_mut()
            .unwrap()
            .frame_count = frames;
        context_state(&map)
    };
    assert_eq!(with_frames(103), ContextState::Consistent);
    assert_eq!(with_frames(150), ContextState::Drifted);
    assert_eq!(with_frames(60), ContextState::Drifted);
    // The mapped runtime must be bound to the arm's own installation.
    use super::super::super::loaded_runtime::RuntimeBinding;
    let with_binding = |first: Option<RuntimeBinding>, second: Option<RuntimeBinding>| {
        let mut map = same.clone();
        map.get_mut(&0)
            .unwrap()
            .native_completion
            .as_mut()
            .unwrap()
            .runtime_binding = first;
        map.get_mut(&1)
            .unwrap()
            .native_completion
            .as_mut()
            .unwrap()
            .runtime_binding = second;
        context_state(&map)
    };
    let matched = Some(RuntimeBinding::Matched);
    assert_eq!(with_binding(matched, matched), ContextState::Consistent);
    assert_eq!(with_binding(matched, None), ContextState::Unproven);
    assert_eq!(with_binding(None, matched), ContextState::Unproven);
    assert_eq!(
        with_binding(matched, Some(RuntimeBinding::Unknown)),
        ContextState::Unproven
    );
    assert_eq!(
        with_binding(matched, Some(RuntimeBinding::Mismatched)),
        ContextState::Drifted
    );
    assert_eq!(
        with_binding(Some(RuntimeBinding::Mismatched), matched),
        ContextState::Drifted
    );
    let drifted: BTreeMap<_, _> = [
        with(0, Some(context("RayTracingUltra"))),
        with(1, Some(context("Low"))),
    ]
    .into();
    assert_eq!(context_state(&drifted), ContextState::Drifted);
    let missing: BTreeMap<_, _> = [with(0, Some(context("RayTracingUltra"))), with(1, None)].into();
    assert_eq!(context_state(&missing), ContextState::Unproven);
    // Two reports that are both silent about a graphics field are not proven the same.
    let silent = |index: u32| {
        let mut hole = context("RayTracingUltra");
        hole.ray_tracing_enabled = None;
        with(index, Some(hole))
    };
    let both_silent: BTreeMap<_, _> = [silent(0), silent(1)].into();
    assert_eq!(context_state(&both_silent), ContextState::Unproven);
    // A single complete completion is consistent with itself; a single incomplete one is not.
    let alone: BTreeMap<_, _> = [with(0, Some(context("RayTracingUltra")))].into();
    assert_eq!(context_state(&alone), ContextState::Consistent);
    let alone_silent: BTreeMap<_, _> = [silent(0)].into();
    assert_eq!(context_state(&alone_silent), ContextState::Unproven);
    // A real difference is still drift even when another field is unreported.
    let mut hole = context("Low");
    hole.gpu_name = None;
    let drift_with_hole: BTreeMap<_, _> = [
        with(0, Some(context("RayTracingUltra"))),
        with(1, Some(hole)),
    ]
    .into();
    assert_eq!(context_state(&drift_with_hole), ContextState::Drifted);
}
