use std::path::PathBuf;

use super::*;
use crate::autotune::research::profile_applier::ray_tracing_off;

const SETTINGS: &str = r#"{"version":1,"data":[{"group_name":"/graphics/raytracing","options":[
{"name":"RayTracing","type":"bool","value":true},
{"name":"RayTracedReflections","type":"bool","value":true},
{"name":"RayTracedSunShadows","type":"bool","value":true},
{"name":"RayTracedLocalShadows","type":"bool","value":true},
{"name":"RayTracedPathTracing","type":"bool","value":false}]},
{"group_name":"/video/display","options":[
{"name":"Resolution","type":"string_list","value":"3440x1440","index":23}]}]}"#;

struct Arms {
    _root: tempfile::TempDir,
    files: Vec<StudySettingsFile>,
    paths: Vec<PathBuf>,
}

fn arms(count: usize, contents: &[&str]) -> Arms {
    let root = tempfile::tempdir().unwrap();
    let study = std::fs::canonicalize(root.path()).unwrap();
    let (mut files, mut paths) = (Vec::new(), Vec::new());
    for index in 0..count {
        let dir = study.join(format!("compatdata/arm{index}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("UserSettings.json");
        std::fs::write(&path, contents.get(index).unwrap_or(&SETTINGS)).unwrap();
        files
            .push(StudySettingsFile::new(&study, path.clone(), crate::syscall::geteuid()).unwrap());
        paths.push(path);
    }
    Arms {
        _root: root,
        files,
        paths,
    }
}

fn index_free(profile: Profile) -> Result<Option<ProfileSpec>, ProfileFailure> {
    match profile {
        Profile::Long => Ok(None),
        _ => Ok(Some(ray_tracing_off())),
    }
}

fn idle() -> bool {
    false
}

fn running() -> bool {
    true
}

fn bytes(arms: &Arms) -> Vec<Vec<u8>> {
    arms.paths
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect()
}

#[test]
fn every_arm_takes_the_profile_and_every_arm_is_restored_byte_for_byte() {
    let a = arms(3, &[]);
    let original = bytes(&a);
    let mut hook = FileProfileHook::new(a.files, index_free, idle);
    assert_eq!(hook.apply(Profile::Cpu), Ok(true));
    for path in &a.paths {
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("\"value\": false")
        );
    }
    assert!(hook.restore().is_ok());
    assert_eq!(bytes(&a_paths(&a.paths)), original);
    // A profile that changes no graphics settings applies nothing and needs no restore.
    let none = arms(1, &[]);
    let mut hook = FileProfileHook::new(none.files, index_free, idle);
    assert_eq!(hook.apply(Profile::Long), Ok(false));
}

fn a_paths(paths: &[PathBuf]) -> Arms {
    Arms {
        _root: tempfile::tempdir().unwrap(),
        files: Vec::new(),
        paths: paths.to_vec(),
    }
}

#[test]
fn one_arm_that_cannot_take_the_profile_puts_the_others_back_and_reports_why() {
    // The second arm lacks the settings entirely.
    let a = arms(3, &[SETTINGS, r#"{"version":1,"data":[]}"#, SETTINGS]);
    let original = bytes(&a);
    let mut hook = FileProfileHook::new(a.files, index_free, idle);
    assert_eq!(
        hook.apply(Profile::Cpu),
        Err(ProfileFailure::Failed("ProfileSettingMismatch"))
    );
    assert_eq!(
        bytes(&a_paths(&a.paths)),
        original,
        "no arm is left changed"
    );
}

#[test]
fn blocked_mappings_and_a_running_game_change_nothing() {
    let a = arms(2, &[]);
    let original = bytes(&a);
    let mut real = FileProfileHook::new(a.files, production_spec, idle);
    // The production CPU profile needs an unobserved resolution index; the GPU profile has none.
    assert_eq!(
        real.apply(Profile::Cpu),
        Err(ProfileFailure::Blocked("BlockedProfileMapping"))
    );
    assert_eq!(
        real.apply(Profile::Gpu),
        Err(ProfileFailure::Blocked("BlockedProfileMapping"))
    );
    assert_eq!(real.apply(Profile::Long), Ok(false));
    assert_eq!(bytes(&a_paths(&a.paths)), original);
    let b = arms(2, &[]);
    let mut busy = FileProfileHook::new(b.files, index_free, running);
    assert_eq!(
        busy.apply(Profile::Cpu),
        Err(ProfileFailure::Failed("GameRunning"))
    );
}

#[test]
fn a_restore_that_cannot_be_proven_is_reported() {
    let a = arms(2, &[]);
    let mut hook = FileProfileHook::new(a.files, index_free, idle);
    assert_eq!(hook.apply(Profile::Cpu), Ok(true));
    // The game rewrote one arm's settings meanwhile.
    std::fs::write(
        &a.paths[1],
        SETTINGS.replace("\"version\":1", "\"version\":2"),
    )
    .unwrap();
    assert_eq!(hook.restore(), Err("RestoreUnproven"));
}

#[test]
fn a_hook_recovers_every_arm_that_a_crash_left_edited() {
    let a = arms(2, &[]);
    let original = bytes(&a);
    let mut hook = FileProfileHook::new(a.files, index_free, idle);
    assert_eq!(hook.apply(Profile::Cpu), Ok(true));
    // The controller dies: a new process builds a new hook over the same files.
    let study = std::fs::canonicalize(a.paths[0].ancestors().nth(2).unwrap()).unwrap();
    let files = a
        .paths
        .iter()
        .map(|path| {
            StudySettingsFile::new(&study, path.clone(), crate::syscall::geteuid()).unwrap()
        })
        .collect();
    let mut restarted = FileProfileHook::new(files, index_free, idle);
    assert_ne!(bytes(&a_paths(&a.paths)), original);
    assert_eq!(restarted.recover(), Ok(()));
    assert_eq!(bytes(&a_paths(&a.paths)), original);
    // With nothing to recover it is a no-op.
    assert_eq!(restarted.recover(), Ok(()));
}

#[test]
fn a_restoration_that_failed_is_kept_and_succeeds_when_retried_after_the_cause_is_gone() {
    let a = arms(2, &[]);
    let original = bytes(&a);
    let mut hook = FileProfileHook::new(a.files, index_free, idle);
    assert_eq!(hook.apply(Profile::Cpu), Ok(true));
    let applied_second = std::fs::read(&a.paths[1]).unwrap();
    // Something else rewrote the second arm's settings: its restoration cannot be proven.
    std::fs::write(&a.paths[1], b"{\"version\":9,\"data\":[]}").unwrap();
    assert_eq!(hook.restore(), Err("RestoreUnproven"));
    // The first arm is back; the failed one is not forgotten and its snapshot is still durable.
    assert_eq!(std::fs::read(&a.paths[0]).unwrap(), original[0]);
    assert!(
        a.paths[1]
            .with_file_name("UserSettings.json.stutter-original")
            .exists()
    );
    // The cause goes away (the file is what was applied again): the same hook now restores it.
    std::fs::write(&a.paths[1], applied_second).unwrap();
    assert_eq!(hook.restore(), Ok(()));
    assert_eq!(std::fs::read(&a.paths[1]).unwrap(), original[1]);
    assert!(
        !a.paths[1]
            .with_file_name("UserSettings.json.stutter-original")
            .exists()
    );
}

#[test]
fn the_game_is_recognised_by_its_executable_in_a_command_line() {
    assert!(names_the_game(
        b"S:\\steamapps\\common\\Cyberpunk 2077\\bin\\x64\\Cyberpunk2077.exe\0-benchmark\0"
    ));
    assert!(names_the_game(b"wine\0CYBERPUNK2077.EXE\0"));
    assert!(!names_the_game(b"/usr/bin/firefox\0--new-window\0"));
    assert!(!names_the_game(b""));
    // The scan itself runs against the real /proc without panicking.
    let _ = cyberpunk_running();
}

#[test]
fn a_command_that_only_mentions_the_game_is_not_the_game() {
    assert!(names_the_game(b"wine\0Cyberpunk2077.exe\0"));
    assert!(names_the_game(
        b"/steam/reaper\0SteamLaunch\0AppId=1091500\0--\0proton\0"
    ));
    assert!(!names_the_game(
        b"bash\0-c\0pgrep -f Cyberpunk2077.exe && echo AppId=1091500\0"
    ));
    assert!(!names_the_game(b"nvim\0Cyberpunk2077.exe.log\0"));
    assert!(!names_the_game(b"steam\0AppId=570940\0"));
}

#[test]
fn an_unreadable_live_process_is_unknown_and_counts_as_running_but_a_vanished_one_does_not() {
    use std::io::{Error, ErrorKind};
    let denied = || Err(Error::from(ErrorKind::PermissionDenied));
    let vanished = || Err(Error::from(ErrorKind::NotFound));
    assert!(scan([Ok(b"firefox\0".to_vec()), denied()].into_iter()));
    assert!(!scan([Ok(b"firefox\0".to_vec()), vanished()].into_iter()));
    assert!(scan(
        [vanished(), Ok(b"Cyberpunk2077.exe\0".to_vec())].into_iter()
    ));
    assert!(!scan(std::iter::empty()));
}

fn cold_path(a: &Arms) -> PathBuf {
    a.paths[0]
        .ancestors()
        .nth(2)
        .unwrap()
        .join("compatdata/cold/UserSettings.json")
}

#[test]
fn an_unavailable_arm_refuses_the_profile_without_changing_any_arm() {
    let a = arms(1, &[]);
    let original = bytes(&a);
    let missing = cold_path(&a);
    let paths = a.paths.clone();
    let mut hook = FileProfileHook::new(a.files, index_free, idle).with_unavailable(vec![missing]);
    assert_eq!(
        hook.apply(Profile::Cpu),
        Err(ProfileFailure::Failed("SettingsFileUnavailable"))
    );
    assert_eq!(bytes(&a_paths(&paths)), original, "no arm was edited");
    // A study that needs no graphics profile still runs.
    assert_eq!(hook.apply(Profile::Long), Ok(false));
}

#[test]
fn recovery_restores_every_openable_arm_even_when_another_arm_has_no_settings_file() {
    let a = arms(2, &[]);
    let original = bytes(&a);
    let paths = a.paths.clone();
    let mut hook = FileProfileHook::new(a.files, index_free, idle);
    assert_eq!(hook.apply(Profile::Cpu), Ok(true));
    // The controller dies. Arm B's settings file is then gone (a damaged or removed prefix) while its
    // durable snapshot of the unrestored edit remains beside it.
    let study = std::fs::canonicalize(paths[0].ancestors().nth(2).unwrap()).unwrap();
    std::fs::remove_file(&paths[1]).unwrap();
    let arm_a =
        StudySettingsFile::new(&study, paths[0].clone(), crate::syscall::geteuid()).unwrap();
    let mut restarted = FileProfileHook::new(vec![arm_a], index_free, idle)
        .with_unavailable(vec![paths[1].clone()]);
    // Arm A is restored byte for byte; arm B's pending snapshot cannot be inspected, so the
    // recovery is reported unproven, never a silent success.
    assert_eq!(restarted.recover(), Err("RecoveryUnproven"));
    assert_eq!(std::fs::read(&paths[0]).unwrap(), original[0]);
    assert!(
        paths[1]
            .with_file_name("UserSettings.json.stutter-original")
            .exists(),
        "the unrestored arm's snapshot is kept"
    );
}

#[test]
fn an_arm_that_never_had_a_snapshot_does_not_make_recovery_unproven() {
    let a = arms(1, &[]);
    let original = bytes(&a);
    let cold = cold_path(&a);
    let paths = a.paths.clone();
    let mut hook = FileProfileHook::new(a.files, index_free, idle).with_unavailable(vec![cold]);
    assert_eq!(hook.recover(), Ok(()));
    assert_eq!(bytes(&a_paths(&paths)), original);
}
