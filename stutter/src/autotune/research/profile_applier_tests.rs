use std::os::unix::fs::PermissionsExt;

use super::*;

/// The shape of the real `UserSettings.json`: groups of options with `value` and, for list
/// options, `index`.
const SETTINGS: &str = r#"{
  "version": 152,
  "data": [
    {"group_name": "/graphics/raytracing", "options": [
      {"name": "RayTracing", "type": "bool", "value": true},
      {"name": "RayTracedReflections", "type": "bool", "value": true},
      {"name": "RayTracedSunShadows", "type": "bool", "value": true},
      {"name": "RayTracedLocalShadows", "type": "bool", "value": true},
      {"name": "RayTracedLighting", "type": "string_list", "value": "Ultra"},
      {"name": "RayTracedPathTracing", "type": "bool", "value": false}
    ]},
    {"group_name": "/video/display", "options": [
      {"name": "Resolution", "type": "string_list", "value": "3440x1440", "index": 23},
      {"name": "WindowMode", "type": "string_list", "value": "BorderlessWindowed", "index": 1},
      {"name": "MaximumFPS_Value", "type": "int", "value": 30}
    ]}
  ]
}"#;

struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    path: PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let study = root.path().join("study");
    let dir = study.join("compatdata/ge/pfx/drive_c/users/steamuser/AppData/Local/Game");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("UserSettings.json");
    std::fs::write(&path, SETTINGS).unwrap();
    Fixture {
        root: std::fs::canonicalize(&study).unwrap(),
        path: std::fs::canonicalize(&path).unwrap(),
        _root: root,
    }
}

fn open(f: &Fixture) -> StudySettingsFile {
    StudySettingsFile::new(&f.root, f.path.clone(), crate::syscall::geteuid()).unwrap()
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn value_of(doc: &Value, group: &str, name: &str) -> Value {
    doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["group_name"] == group)
        .unwrap()["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == name)
        .unwrap()["value"]
        .clone()
}

#[test]
fn an_index_free_profile_applies_reads_back_exactly_and_restores_the_original_bytes() {
    let f = fixture();
    let file = open(&f);
    let applied = apply(&file, &ray_tracing_off(), false).unwrap();
    assert_eq!(applied.profile, "ray_tracing_off");
    let now = read(&f.path);
    for name in ["RayTracing", "RayTracedReflections", "RayTracedSunShadows"] {
        assert_eq!(
            value_of(&now, "/graphics/raytracing", name),
            Value::Bool(false)
        );
    }
    // Nothing else moved.
    assert_eq!(value_of(&now, "/video/display", "Resolution"), "3440x1440");
    assert_eq!(
        value_of(&now, "/graphics/raytracing", "RayTracedLighting"),
        "Ultra"
    );
    assert_eq!(now["version"], 152);
    restore(&file, &applied).unwrap();
    assert_eq!(std::fs::read(&f.path).unwrap(), SETTINGS.as_bytes());
    // Restoring again is idempotent.
    restore(&file, &applied).unwrap();
}

#[test]
fn a_list_value_whose_index_was_never_observed_is_blocked_and_nothing_is_written() {
    let f = fixture();
    let file = open(&f);
    assert_eq!(
        apply(&file, &cpu_profile(), false).unwrap_err(),
        Refusal::BlockedProfileMapping("/video/display/Resolution".into())
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), SETTINGS.as_bytes());
    // The same list option with an observed index is accepted and writes both value and index.
    let spec = ProfileSpec {
        name: "observed",
        edits: vec![ProfileEdit {
            group: "/video/display",
            name: "WindowMode",
            to: Setting::Choice {
                value: "Windowed",
                index: Some(0),
            },
        }],
    };
    let applied = apply(&file, &spec, false).unwrap();
    let option = &read(&f.path)["data"][1]["options"][1];
    assert_eq!(
        (option["value"].clone(), option["index"].clone()),
        ("Windowed".into(), 0.into())
    );
    restore(&file, &applied).unwrap();
}

#[test]
fn unknown_or_mistyped_settings_and_a_running_game_are_refused_before_any_write() {
    let f = fixture();
    let file = open(&f);
    let edit = |group, name, to| ProfileSpec {
        name: "bad",
        edits: vec![ProfileEdit { group, name, to }],
    };
    assert_eq!(
        apply(
            &file,
            &edit("/graphics/raytracing", "Nope", Setting::Bool(true)),
            false
        )
        .unwrap_err(),
        Refusal::UnknownSetting("/graphics/raytracing/Nope".into())
    );
    assert_eq!(
        apply(
            &file,
            &edit("/nowhere", "RayTracing", Setting::Bool(true)),
            false
        )
        .unwrap_err(),
        Refusal::UnknownSetting("/nowhere/RayTracing".into())
    );
    assert_eq!(
        apply(
            &file,
            &edit("/graphics/raytracing", "RayTracing", Setting::Int(1)),
            false
        )
        .unwrap_err(),
        Refusal::TypeMismatch("/graphics/raytracing/RayTracing".into())
    );
    assert_eq!(
        apply(&file, &ray_tracing_off(), true).unwrap_err(),
        Refusal::GameRunning
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), SETTINGS.as_bytes());
}

#[test]
fn a_file_the_game_rewrote_after_the_apply_is_reported_not_blindly_overwritten() {
    let f = fixture();
    let file = open(&f);
    let applied = apply(&file, &ray_tracing_off(), false).unwrap();
    // Something else rewrote the settings (for example the game saving its options).
    std::fs::write(&f.path, SETTINGS.replace("\"value\": 30", "\"value\": 60")).unwrap();
    assert_eq!(
        restore(&file, &applied).unwrap_err(),
        Refusal::RestoreMismatch
    );
    assert!(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .contains("\"value\": 60")
    );
}

#[test]
fn a_restore_of_an_untouched_original_is_a_no_op_that_still_proves_the_bytes() {
    let f = fixture();
    let file = open(&f);
    let applied = apply(&file, &ray_tracing_off(), false).unwrap();
    std::fs::write(&f.path, SETTINGS).unwrap();
    restore(&file, &applied).unwrap();
}

#[test]
fn only_a_study_owned_regular_settings_file_is_ever_opened() {
    let f = fixture();
    let uid = crate::syscall::geteuid();
    // Outside the study root.
    let other = tempfile::tempdir().unwrap();
    let outside = other.path().join("UserSettings.json");
    std::fs::write(&outside, SETTINGS).unwrap();
    assert!(matches!(
        StudySettingsFile::new(&f.root, std::fs::canonicalize(&outside).unwrap(), uid),
        Err(Refusal::UnsafeFile)
    ));
    // A symlink inside the study, even to a genuine file.
    let link = f.path.with_file_name("link").join("UserSettings.json");
    std::fs::create_dir(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&f.path, &link).unwrap();
    assert!(matches!(
        StudySettingsFile::new(&f.root, link, uid),
        Err(Refusal::UnsafeFile)
    ));
    // The wrong file name, a foreign owner and a hard-linked file.
    let named = f.path.with_file_name("Other.json");
    std::fs::write(&named, SETTINGS).unwrap();
    assert!(matches!(
        StudySettingsFile::new(&f.root, named, uid),
        Err(Refusal::UnsafeFile)
    ));
    assert!(matches!(
        StudySettingsFile::new(&f.root, f.path.clone(), uid + 1),
        Err(Refusal::UnsafeFile)
    ));
    std::fs::hard_link(&f.path, f.path.with_file_name("twin.json")).unwrap();
    assert!(matches!(
        StudySettingsFile::new(&f.root, f.path.clone(), uid),
        Err(Refusal::UnsafeFile)
    ));
}

#[test]
fn a_malformed_settings_file_is_refused_and_left_alone() {
    let f = fixture();
    std::fs::write(&f.path, b"{ not json").unwrap();
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let file = open(&f);
    assert_eq!(
        apply(&file, &ray_tracing_off(), false).unwrap_err(),
        Refusal::Malformed
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), b"{ not json");
    std::fs::write(&f.path, br#"{"version": 1}"#).unwrap();
    assert_eq!(
        apply(&file, &ray_tracing_off(), false).unwrap_err(),
        Refusal::Malformed
    );
}

/// Read-only: the pure planner against a real preserved `UserSettings.json`
/// (`STUTTER_REAL_SETTINGS=<file> cargo test real_settings_plan -- --ignored`). Nothing is written.
#[test]
#[ignore = "needs a preserved real UserSettings.json"]
fn real_settings_plan() {
    let path = std::env::var_os("STUTTER_REAL_SETTINGS").expect("set STUTTER_REAL_SETTINGS");
    let bytes = std::fs::read(path).unwrap();
    let (_, planned) = plan(&bytes, &ray_tracing_off()).unwrap();
    assert_eq!(planned.len(), ray_tracing_off().edits.len());
    assert_eq!(
        plan(&bytes, &cpu_profile()).unwrap_err(),
        Refusal::BlockedProfileMapping("/video/display/Resolution".into())
    );
}
