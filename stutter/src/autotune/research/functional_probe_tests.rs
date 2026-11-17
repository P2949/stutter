use super::*;

fn context(frame_generation: Option<i64>, upscaler: Option<i64>) -> NativeContext {
    NativeContext {
        game_version: Some("2.5".into()),
        preset_name: Some("P".into()),
        gpu_name: Some("G".into()),
        upscaling_type: upscaler,
        ray_tracing_enabled: Some(false),
        frame_generation_type: frame_generation,
        frame_count: 10,
    }
}

fn judged(factor: ResearchFactor, attempts: &[(Treatment, &NativeContext)]) -> Functional {
    let attempts: Vec<_> = attempts.iter().map(|(t, c)| (*t, Some(*c))).collect();
    probe(factor, &attempts)
}

#[test]
fn frame_generation_is_observed_working_only_when_on_reports_it_and_off_does_not() {
    let on = context(Some(2), Some(0));
    let off = context(Some(0), Some(0));
    let factor = ResearchFactor::FrameGeneration;
    assert_eq!(
        judged(
            factor,
            &[
                (Treatment::Off, &off),
                (Treatment::On, &on),
                (Treatment::On, &on)
            ]
        ),
        Functional::GameReportedSelection
    );
    // The control reached the game but the feature did not turn on: activated, not functional.
    assert_eq!(
        judged(factor, &[(Treatment::Off, &off), (Treatment::On, &off)]),
        Functional::NotObserved
    );
    // The feature on in an attempt that should have been off: the control leaked.
    assert_eq!(
        judged(factor, &[(Treatment::Off, &on), (Treatment::On, &on)]),
        Functional::NotObserved
    );
}

#[test]
fn the_upscaler_is_read_from_its_own_field_not_frame_generations() {
    let on = context(Some(0), Some(3));
    let off = context(Some(0), Some(0));
    assert_eq!(
        judged(
            ResearchFactor::Fsr4,
            &[(Treatment::Off, &off), (Treatment::On, &on)]
        ),
        Functional::GameReportedSelection
    );
    // Frame generation being on says nothing about the upscaler.
    let fg_only = context(Some(2), Some(0));
    assert_eq!(
        judged(
            ResearchFactor::Fsr4,
            &[(Treatment::Off, &off), (Treatment::On, &fg_only)]
        ),
        Functional::NotObserved
    );
}

#[test]
fn missing_evidence_is_unproven_and_a_factor_the_game_never_reports_is_not_observable() {
    let on = context(Some(2), Some(0));
    let off = context(Some(0), Some(0));
    let factor = ResearchFactor::FrameGeneration;
    // One treatment never ran.
    assert_eq!(
        judged(factor, &[(Treatment::On, &on)]),
        Functional::Unproven
    );
    assert_eq!(judged(factor, &[]), Functional::Unproven);
    // A completed attempt with no sealed context, or a context without the field.
    assert_eq!(
        probe(
            factor,
            &[(Treatment::Off, Some(&off)), (Treatment::On, None)]
        ),
        Functional::Unproven
    );
    let blank = context(None, Some(0));
    assert_eq!(
        judged(factor, &[(Treatment::Off, &off), (Treatment::On, &blank)]),
        Functional::Unproven
    );
    // No report exists for these: they are never assumed to work.
    for other in [
        ResearchFactor::WineWayland,
        ResearchFactor::Vkd3dLowLatency,
        ResearchFactor::NtSync,
        ResearchFactor::AntiLag2,
        ResearchFactor::Hdr,
    ] {
        assert_eq!(
            judged(other, &[(Treatment::Off, &off), (Treatment::On, &on)]),
            Functional::NotObservable,
            "{other:?}"
        );
    }
}

/// The probe over the real, preserved native results (read-only):
/// `STUTTER_REAL_RESULTS=<benchmarkResults dir> cargo test real_feature_probe -- --ignored --nocapture`.
/// These runs all used one configuration, so they cannot show a feature working: that needs runs with
/// the factor off and on, which only new launches produce.
#[test]
#[ignore = "needs the preserved real native results"]
fn real_feature_probe() {
    let root = std::env::var_os("STUTTER_REAL_RESULTS").expect("set STUTTER_REAL_RESULTS");
    let mut contexts = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap().flatten() {
        let (summary, frames) = (
            std::fs::read(entry.path().join("summary.json")),
            std::fs::read(entry.path().join("frames.csv")),
        );
        let (Ok(summary), Ok(frames)) = (summary, frames) else {
            continue;
        };
        contexts.push(
            crate::autotune::research::native_context::verify_frames(&summary, &frames).unwrap(),
        );
    }
    println!("real results parsed: {}", contexts.len());
    let first = &contexts[0];
    println!(
        "upscaling_type={:?} frame_generation_type={:?} ray_tracing={:?} gpu={:?}",
        first.upscaling_type,
        first.frame_generation_type,
        first.ray_tracing_enabled,
        first.gpu_name
    );
    // Every real result reports the same state, so no treatment pairing can show a difference.
    assert!(
        contexts
            .iter()
            .all(|c| c.upscaling_type == first.upscaling_type
                && c.frame_generation_type == first.frame_generation_type)
    );
    let on_only: Vec<_> = contexts.iter().map(|c| (Treatment::On, Some(c))).collect();
    let off_only: Vec<_> = contexts.iter().map(|c| (Treatment::Off, Some(c))).collect();
    // The upscaler really is on in these runs: treated "On" they show it in use but there is no Off run.
    println!(
        "upscaler as On only : {:?}",
        probe(ResearchFactor::Fsr4, &on_only)
    );
    println!(
        "upscaler as Off only: {:?}",
        probe(ResearchFactor::Fsr4, &off_only)
    );
    println!(
        "frame generation as On only: {:?}",
        probe(ResearchFactor::FrameGeneration, &on_only)
    );
    assert_eq!(probe(ResearchFactor::Fsr4, &on_only), Functional::Unproven);
    assert_eq!(
        probe(ResearchFactor::Fsr4, &off_only),
        Functional::NotObserved
    );
    assert_eq!(
        probe(ResearchFactor::FrameGeneration, &on_only),
        Functional::NotObserved
    );
    // A mixed schedule over the real results would need runs that differ; none exist.
    let mixed: Vec<_> = contexts
        .iter()
        .enumerate()
        .map(|(i, c)| {
            (
                if i % 2 == 0 {
                    Treatment::On
                } else {
                    Treatment::Off
                },
                Some(c),
            )
        })
        .collect();
    println!(
        "upscaler, alternating labels on identical runs: {:?}",
        probe(ResearchFactor::Fsr4, &mixed)
    );
    assert_eq!(probe(ResearchFactor::Fsr4, &mixed), Functional::NotObserved);
}
