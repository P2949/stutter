use super::*;

#[test]
fn the_rendered_resolution_and_dynamic_scaling_are_read_and_judged_against_the_profile() {
    let frames_csv = || {
        let mut text = String::from("Frame index, Frame time (ms)\n");
        for index in 0..100 {
            text.push_str(&format!("{index}, {:.4}\n", 1000.0 / 71.0));
        }
        text.into_bytes()
    };
    use crate::autotune::research::child_study::{Profile, Stage};
    let summary = |w: u32, h: u32, drs: bool| {
        format!(
            r#"{{"RootType":"worldBenchmarkSummary","Data":{{"averageFps":71.0,"frameNumber":100,"gameVersion":"2.5","presetName":"P","gpuName":"G","upscalingType":0,"rayTracingEnabled":false,"frameGenerationType":0,"renderWidth":{w},"renderHeight":{h},"DRSEnabled":{drs}}}}}"#
        )
    };
    let context = |w, h, drs| verify_frames(summary(w, h, drs).as_bytes(), &frames_csv()).unwrap();
    let at_1080 = context(1920, 1080, false);
    assert_eq!(
        (
            at_1080.render_width,
            at_1080.render_height,
            at_1080.dynamic_resolution
        ),
        (Some(1920), Some(1080), Some(false))
    );
    // Each profile asks for its own resolution: the CPU profile 1080p and the GPU profile the native 3440x1440.
    assert_eq!(
        at_1080.effective_resolution(Profile::Cpu),
        EffectiveResolution::Confirmed
    );
    assert_eq!(
        at_1080.effective_resolution(Profile::Gpu),
        EffectiveResolution::Contradicted
    );
    assert_eq!(
        at_1080.conformance_in(Profile::Cpu, Stage::Screen),
        ProfileConformance::Conforms
    );
    assert_eq!(
        at_1080.conformance_in(Profile::Gpu, Stage::Screen),
        ProfileConformance::Violates
    );
    let native = context(3440, 1440, false);
    assert_eq!(
        native.effective_resolution(Profile::Gpu),
        EffectiveResolution::Confirmed
    );
    assert_eq!(
        native.effective_resolution(Profile::Cpu),
        EffectiveResolution::Contradicted
    );
    // Dynamic resolution scaling changes the internal resolution whatever the output says.
    let scaled = context(3440, 1440, true);
    assert_eq!(
        scaled.effective_resolution(Profile::Gpu),
        EffectiveResolution::Contradicted
    );
    assert_eq!(
        scaled.conformance_in(Profile::Gpu, Stage::Screen),
        ProfileConformance::Violates
    );
    // A result that does not report it proves nothing about it: unproven, never assumed, and not a violation.
    let silent = verify_frames(
            br#"{"Data":{"averageFps":71.0,"frameNumber":100,"gameVersion":"2.5","presetName":"P","gpuName":"G","upscalingType":0,"rayTracingEnabled":false,"frameGenerationType":0}}"#,
            &frames_csv(),
        )
        .unwrap();
    assert_eq!(
        silent.effective_resolution(Profile::Cpu),
        EffectiveResolution::Unproven
    );
    assert_eq!(
        silent.conformance_in(Profile::Cpu, Stage::Screen),
        ProfileConformance::Conforms
    );
    assert_eq!(
        silent.effective_resolution(Profile::Long),
        EffectiveResolution::NotSpecified
    );
    // A result sealed before the resolution was read still replays against the files that now yield it.
    assert!(silent.matches_replay(&at_1080));
    assert!(!at_1080.matches_replay(&native));
}

#[test]
fn a_result_rendered_on_a_software_rasteriser_violates_every_profile_whatever_else_it_shows() {
    use crate::autotune::research::child_study::{Profile, Stage};
    let context = |gpu: &str| NativeContext {
        game_version: Some("2.5".into()),
        preset_name: Some("P".into()),
        gpu_name: Some(gpu.into()),
        upscaling_type: Some(0),
        ray_tracing_enabled: Some(false),
        frame_generation_type: Some(0),
        render_width: Some(1920),
        render_height: Some(1080),
        dynamic_resolution: Some(false),
        frame_count: 100,
        frame_stats: None,
    };
    let real = context("AMD Radeon RX 9070 XT (RADV GFX1201)");
    assert!(!real.software_renderer());
    assert_eq!(
        real.conformance_in(Profile::Cpu, Stage::Screen),
        ProfileConformance::Conforms
    );
    for software in [
        "llvmpipe (LLVM 20.1.0, 256 bits)",
        "AMD Radeon Graphics (lavapipe)",
        "SwRast",
    ] {
        let soft = context(software);
        assert!(soft.software_renderer(), "{software}");
        assert_eq!(
            soft.conformance_in(Profile::Cpu, Stage::Screen),
            ProfileConformance::Violates
        );
    }
}
