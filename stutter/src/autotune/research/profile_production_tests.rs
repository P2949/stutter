//! The production CPU and GPU profiles, not the boolean-only stand-in, through the real file hook over realistic
//! per-arm settings: the two target resolutions with the upscaler and frame generation off, each list value resolved
//! against its observed index, an unobserved value or a file that cannot take the profile refusing the whole study
//! with every arm's bytes untouched, and byte-for-byte restoration. What the game then renders at is not shown by
//! the settings file; that stays unproven here.

use std::os::unix::fs::PermissionsExt;

use serde_json::{Value, json};

use super::*;
use crate::autotune::research::{
    ProtonSource, child_study::Profile, profile_applier::StudySettingsFile,
    profile_mappings::Mappings, study_executor::ProfileHook,
};

const ARMS: [ProtonSource; 3] = [
    ProtonSource::ValveStable,
    ProtonSource::CachyOs,
    ProtonSource::Ge,
];

fn list(name: &str, value: &str, index: u32) -> Value {
    json!({ "name": name, "type": "string_list", "value": value, "index": index })
}

/// What an arm's shipped file shows: native resolution, FSR2 selected, no frame generation, ray tracing on.
fn shipped(with_frame_generation: bool) -> Vec<u8> {
    let mut presets = vec![list("ResolutionScaling", "FSR2", 1)];
    if with_frame_generation {
        presets.push(list("FrameGeneration", "Off", 0));
    }
    let off = |name: &str| json!({ "name": name, "type": "bool", "value": true });
    serde_json::to_vec(&json!({ "version": 1, "data": [
        { "group_name": "/video/display", "options": [list("Resolution", "3440x1440", 23)] },
        { "group_name": "/graphics/presets", "options": presets },
        { "group_name": "/graphics/raytracing", "options": [
            off("RayTracing"), off("RayTracedReflections"), off("RayTracedSunShadows"),
            off("RayTracedLocalShadows"), off("RayTracedPathTracing") ] },
    ]}))
    .unwrap()
}

/// Mappings observed from files that showed each selection: both resolutions, the upscaler off and on.
fn observed(include_1080p: bool) -> Mappings {
    let doc = |resolution: (&str, u32), scaling: (&str, u32)| {
        serde_json::to_vec(&json!({ "data": [
            { "group_name": "/video/display", "options": [list("Resolution", resolution.0, resolution.1)] },
            { "group_name": "/graphics/presets", "options": [
                list("ResolutionScaling", scaling.0, scaling.1), list("FrameGeneration", "Off", 0)] },
        ]}))
        .unwrap()
    };
    let mut all = Mappings::observe(&doc(("3440x1440", 23), ("FSR2", 1))).unwrap();
    all.merge(&Mappings::observe(&doc(("3440x1440", 23), ("Off", 0))).unwrap())
        .unwrap();
    if include_1080p {
        all.merge(&Mappings::observe(&doc(("1920x1080", 12), ("Off", 0))).unwrap())
            .unwrap();
    }
    all
}

struct World {
    _root: tempfile::TempDir,
    paths: Vec<std::path::PathBuf>,
    hook: FileProfileHook,
}

fn world(mappings: Mappings, broken_arm: Option<usize>) -> World {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let canonical = std::fs::canonicalize(root.path()).unwrap();
    let uid = crate::syscall::geteuid();
    let mut paths = Vec::new();
    let mut files = Vec::new();
    for (index, arm) in ARMS.iter().enumerate() {
        let dir = canonical.join(arm.dir_name());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("UserSettings.json");
        std::fs::write(&path, shipped(broken_arm != Some(index))).unwrap();
        files.push(StudySettingsFile::new(&canonical, path.clone(), uid).unwrap());
        paths.push(path);
    }
    let hook = FileProfileHook::new(files, production_spec, || false).with_mappings(mappings);
    World {
        _root: root,
        paths,
        hook,
    }
}

fn read(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn option(settings: &Value, group: &str, name: &str) -> Value {
    settings["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["group_name"] == group)
        .and_then(|entry| entry["options"].as_array())
        .and_then(|options| options.iter().find(|option| option["name"] == name))
        .cloned()
        .unwrap_or(Value::Null)
}

#[test]
fn the_production_profiles_set_each_arms_resolution_and_turn_the_upscaler_and_frame_generation_off()
{
    for (profile, resolution, index) in [
        (Profile::Cpu, "1920x1080", 12),
        (Profile::Gpu, "3440x1440", 23),
    ] {
        let mut world = world(observed(true), None);
        let before: Vec<Vec<u8>> = world
            .paths
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect();
        assert_eq!(world.hook.apply(profile), Ok(true));
        for path in &world.paths {
            let applied = read(path);
            let selected = option(&applied, "/video/display", "Resolution");
            assert_eq!(
                (selected["value"].as_str(), selected["index"].as_u64()),
                (Some(resolution), Some(index))
            );
            let scaling = option(&applied, "/graphics/presets", "ResolutionScaling");
            assert_eq!(
                (scaling["value"].as_str(), scaling["index"].as_u64()),
                (Some("Off"), Some(0))
            );
            let frame_generation = option(&applied, "/graphics/presets", "FrameGeneration");
            assert_eq!(frame_generation["value"], "Off");
            for name in ["RayTracing", "RayTracedReflections", "RayTracedSunShadows"] {
                assert_eq!(
                    option(&applied, "/graphics/raytracing", name)["value"],
                    false,
                    "{name}"
                );
            }
        }
        // One digest per arm, as the profile left each file; restoring reproduces every original byte.
        assert_eq!(world.hook.applied_digests().len(), ARMS.len());
        assert_eq!(world.hook.restore(), Ok(()));
        let after: Vec<Vec<u8>> = world
            .paths
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect();
        assert_eq!(before, after);
    }
}

#[test]
fn an_unobserved_list_value_or_a_file_that_cannot_take_the_profile_refuses_the_whole_study_untouched()
 {
    // The 1080p index was never observed: nothing is guessed, no arm is edited.
    let mut unmapped = world(observed(false), None);
    let before: Vec<Vec<u8>> = unmapped
        .paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert!(unmapped.hook.apply(Profile::Cpu).is_err());
    let after: Vec<Vec<u8>> = unmapped
        .paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert_eq!(before, after);
    assert!(unmapped.hook.applied_digests().is_empty());
    // The native-resolution profile needs only what was observed, so it still applies.
    assert_eq!(unmapped.hook.apply(Profile::Gpu), Ok(true));
    assert_eq!(unmapped.hook.restore(), Ok(()));
    // One arm's file has no FrameGeneration option: the arms already edited are put back before it is reported.
    let mut partial = world(observed(true), Some(2));
    let before: Vec<Vec<u8>> = partial
        .paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert!(partial.hook.apply(Profile::Cpu).is_err());
    let after: Vec<Vec<u8>> = partial
        .paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert_eq!(before, after, "no arm may keep a half-applied profile");
    assert!(partial.hook.applied_digests().is_empty());
}
