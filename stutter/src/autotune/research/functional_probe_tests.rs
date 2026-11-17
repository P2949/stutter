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
        Functional::Observed
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
        Functional::Observed
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
