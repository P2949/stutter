use super::*;

const GE: ProtonSource = ProtonSource::Ge;
const EM: ProtonSource = ProtonSource::Em;

/// Two arms, three runs each, interleaved; the first run of each is slow (shaders compiling).
fn runs() -> Vec<Run> {
    vec![
        (0, GE, 41.0),
        (1, EM, 39.0),
        (2, GE, 70.0),
        (3, EM, 68.0),
        (4, GE, 72.0),
        (5, EM, 69.0),
    ]
}

#[test]
fn in_a_cold_cohort_the_first_run_of_each_arm_is_cold_and_the_rest_are_warm() {
    let states = classify(Some("cold"), &runs());
    let cold: Vec<u32> = states
        .iter()
        .filter(|(_, state)| *state == CacheState::ColdFirstUse)
        .map(|(slot, _)| *slot)
        .collect();
    assert_eq!(cold, vec![0, 1]);
    assert_eq!(states.len(), 6);
}

#[test]
fn a_warm_or_unrecorded_cohort_keeps_every_run_as_it_was() {
    for cohort in [Some("warm"), None] {
        assert!(
            classify(cohort, &runs())
                .iter()
                .all(|(_, state)| *state == CacheState::Warm)
        );
        assert_eq!(warm_fps(cohort, &runs())[&GE], vec![41.0, 70.0, 72.0]);
    }
}

#[test]
fn the_cold_first_runs_are_never_averaged_into_an_arms_warm_fps() {
    let fps = warm_fps(Some("cold"), &runs());
    assert_eq!(fps[&GE], vec![70.0, 72.0]);
    assert_eq!(fps[&EM], vec![68.0, 69.0]);
    // Order of the input does not matter: the first run is the lowest slot, not the first listed.
    let mut shuffled = runs();
    shuffled.reverse();
    assert_eq!(warm_fps(Some("cold"), &shuffled), fps);
    // An arm with one run only has no warm run at all.
    let single = [(0, GE, 40.0)];
    assert!(warm_fps(Some("cold"), &single).is_empty());
}
