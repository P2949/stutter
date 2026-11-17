use std::os::unix::fs::PermissionsExt;

use serde_json::json;

use super::*;
use crate::autotune::research::profile_applier::{cpu_profile, gpu_profile};

/// A settings document whose tracked options currently show these selections.
fn settings(
    resolution: (&str, u32),
    scaling: (&str, u32),
    frame_generation: (&str, u32),
) -> Vec<u8> {
    let list = |name: &str, (value, index): (&str, u32)| json!({ "name": name, "type": "string_list", "value": value, "index": index });
    serde_json::to_vec(&json!({ "data": [
        { "group_name": "/video/display", "options": [list("Resolution", resolution)] },
        { "group_name": "/graphics/presets", "options": [
            list("ResolutionScaling", scaling), list("FrameGeneration", frame_generation),
            { "name": "RayTracing", "type": "bool", "value": true } ] },
    ]}))
    .unwrap()
}

fn shipped() -> Mappings {
    // What the arms' own settings file shows today: native resolution, FSR2 selected, no frame
    // generation.
    Mappings::observe(&settings(("3440x1440", 23), ("FSR2", 1), ("Off", 0))).unwrap()
}

fn index_of(spec: &ProfileSpec, name: &str) -> Option<u32> {
    spec.edits
        .iter()
        .find_map(|edit| match (&edit.to, edit.name == name) {
            (Setting::Choice { index, .. }, true) => *index,
            _ => None,
        })
}

#[test]
fn observation_reads_only_the_current_selection_of_the_tracked_list_options() {
    let seen = shipped();
    assert_eq!(
        seen.index("/video/display", "Resolution", "3440x1440"),
        Some(23)
    );
    assert_eq!(
        seen.index("/graphics/presets", "ResolutionScaling", "FSR2"),
        Some(1)
    );
    assert_eq!(
        seen.index("/graphics/presets", "FrameGeneration", "Off"),
        Some(0)
    );
    // Only what is selected is known: the options never selected are not guessed.
    assert_eq!(
        seen.index("/video/display", "Resolution", "1920x1080"),
        None
    );
    assert_eq!(
        seen.index("/graphics/presets", "ResolutionScaling", "Off"),
        None
    );
    // A bool is not a list option, and a document that is not settings observes nothing.
    assert!(!seen.choices.contains_key("/graphics/presets/RayTracing"));
    assert!(Mappings::observe(b"not json").is_none());
}

#[test]
fn a_profile_is_resolved_only_as_far_as_values_have_been_observed() {
    // The CPU profile needs 1920x1080, never seen: still blocked.
    let cpu = shipped().resolve(cpu_profile());
    assert_eq!(index_of(&cpu, "Resolution"), None);
    // The GPU profile gets the resolution and frame generation it needs, and stays blocked only on
    // the "no upscaler" value nothing has ever shown selected.
    let gpu = shipped().resolve(gpu_profile());
    assert_eq!(index_of(&gpu, "Resolution"), Some(23));
    assert_eq!(index_of(&gpu, "FrameGeneration"), Some(0));
    assert_eq!(index_of(&gpu, "ResolutionScaling"), None);
    // Once a file shows each needed value selected, the profile is fully resolved.
    let mut all = shipped();
    all.merge(&Mappings::observe(&settings(("1920x1080", 5), ("Off", 0), ("Off", 0))).unwrap())
        .unwrap();
    assert_eq!(index_of(&all.resolve(cpu_profile()), "Resolution"), Some(5));
    let gpu = all.resolve(gpu_profile());
    assert!(
        gpu.edits
            .iter()
            .all(|edit| !matches!(edit.to, Setting::Choice { index: None, .. }))
    );
}

#[test]
fn an_observation_that_contradicts_an_earlier_one_is_refused_not_overwritten() {
    let mut known = shipped();
    // The same value at another index.
    let moved = Mappings::observe(&settings(("3440x1440", 9), ("FSR2", 1), ("Off", 0))).unwrap();
    assert!(known.merge(&moved).is_err());
    // Another value at an index already taken.
    let taken = Mappings::observe(&settings(("2560x1440", 23), ("FSR2", 1), ("Off", 0))).unwrap();
    assert!(known.merge(&taken).is_err());
    // The refused merges changed nothing.
    assert_eq!(known, shipped());
    // The same observation again is fine.
    known.merge(&shipped()).unwrap();
}

#[test]
fn observations_are_remembered_beside_the_study_and_untrusted_ones_are_not_used() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = File::open(root.path()).unwrap();
    let uid = crate::syscall::geteuid();
    assert!(Mappings::load(&dir, uid).unwrap().is_empty());
    shipped().save(&dir).unwrap();
    shipped().save(&dir).unwrap(); // the same content has the same name: harmless
    let other = Mappings::observe(&settings(("1920x1080", 5), ("Off", 0), ("Off", 0))).unwrap();
    other.save(&dir).unwrap();
    let loaded = Mappings::load(&dir, uid).unwrap();
    assert_eq!(
        loaded.index("/video/display", "Resolution", "1920x1080"),
        Some(5)
    );
    assert_eq!(
        loaded.index("/video/display", "Resolution", "3440x1440"),
        Some(23)
    );
    // A remembered file that contradicts another makes the whole store unusable.
    let clash = Mappings::observe(&settings(("3440x1440", 7), ("FSR2", 1), ("Off", 0))).unwrap();
    clash.save(&dir).unwrap();
    assert!(Mappings::load(&dir, uid).is_err());
}
