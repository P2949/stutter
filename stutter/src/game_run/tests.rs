use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
};

use super::*;

fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-game-run-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    root
}

fn plan(root: &Path, script: &str, mangohud: bool) -> GameRunPlan {
    GameRunPlan {
        argv: vec!["sh".into(), "-c".into(), script.into()],
        capture_root: root.to_owned(),
        mangohud,
    }
}

#[test]
fn the_childs_exit_code_and_signal_status_are_preserved() {
    let root = root("exit");
    assert_eq!(run(&plan(&root, "exit 7", false), false).unwrap(), 7);
    assert_eq!(
        run(&plan(&root, "kill -TERM $$", false), false).unwrap(),
        128 + 15
    );
    assert!(
        run(
            &GameRunPlan {
                argv: vec![],
                ..plan(&root, "", false)
            },
            false
        )
        .is_err()
    );
    assert!(
        run(
            &GameRunPlan {
                argv: vec!["/nonexistent/binary".into()],
                ..plan(&root, "", false)
            },
            false
        )
        .is_err()
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_launch_is_recorded_securely_and_mangohud_is_opt_in() {
    let root = root("env");
    let out = std::env::temp_dir().join(format!("stutter-game-run-seen-{}", std::process::id()));
    let script = format!(
        "printf '%s|%s|%s' \"$STUTTER_LAUNCH_ID\" \"$MANGOHUD\" \"$MANGOHUD_CONFIG\" > {}",
        out.display()
    );
    assert_eq!(run(&plan(&root, &script, false), false).unwrap(), 0);
    let seen = fs::read_to_string(&out).unwrap();
    let parts: Vec<&str> = seen.split('|').collect();
    assert_eq!(parts[0].len(), 32);
    // Not opted in: no MangoHud variables are added.
    assert_eq!((parts[1], parts[2]), ("", ""));
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let dir = root.join(parts[0]);
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let record = read_launch_record(&dir, crate::syscall::geteuid()).unwrap();
    assert_eq!(record.launch_id, parts[0]);
    assert!(!record.mangohud_requested);
    assert_eq!(record.argv[0], "sh");

    assert_eq!(run(&plan(&root, &script, true), false).unwrap(), 0);
    let seen = fs::read_to_string(&out).unwrap();
    let parts: Vec<&str> = seen.split('|').collect();
    assert_eq!(parts[1], "1");
    assert!(parts[2].contains(&format!("output_folder={}", root.join(parts[0]).display())));
    assert!(parts[2].contains("autostart_log=1"));
    assert!(parts[2].contains("log_interval=0"), "per-frame logging");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_existing_mangohud_config_is_extended_not_replaced() {
    let config = mangohud_config(Some("fps,gpu_stats"), Path::new("/run/x")).unwrap();
    assert!(config.starts_with("fps,gpu_stats,output_folder=/run/x"));
    assert!(mangohud_config(None, Path::new("/run/a,b")).is_err());
}

#[test]
fn pruning_touches_only_this_owners_real_capture_directories() {
    let root = root("prune");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let uid = crate::syscall::geteuid();
    let victim_target = root.join("outside");
    fs::create_dir_all(&victim_target).unwrap();
    for index in 0..5 {
        fs::create_dir(root.join(format!("{index:032x}"))).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    symlink(&victim_target, root.join(format!("{:032x}", 99))).unwrap();
    fs::create_dir(root.join("not-a-capture")).unwrap();
    assert_eq!(prune_captures(&root, uid, 2).unwrap(), 3);
    assert!(root.join(format!("{:032x}", 4)).exists());
    assert!(!root.join(format!("{:032x}", 0)).exists());
    assert!(victim_target.exists());
    assert!(root.join("not-a-capture").exists());
    fs::remove_dir_all(&root).unwrap();
}
