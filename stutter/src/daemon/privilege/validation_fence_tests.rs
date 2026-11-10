use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use super::*;

const NOW: u64 = 1_000_000;
const BOOT: &str = "boot-1";
const SERVICE_UID: u32 = 1000;

fn me() -> u32 {
    crate::syscall::geteuid()
}

/// A directory standing in for the worker's root-owned state directory: owned by the test user and
/// not writable by anyone else.
fn state_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-fence-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn fence_json(root_pid: u32, ticks: u64, expires: u64) -> String {
    format!(
        r#"{{"version":1,"uid":{SERVICE_UID},"root_pid":{root_pid},"start_ticks":{ticks},"boot_id":"{BOOT}","expires_unix":{expires}}}"#
    )
}

fn write_fence(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join(FENCE_FILE);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    path
}

fn load_at(path: &Path, now: u64) -> anyhow::Result<FenceState> {
    load(path, now, BOOT, SERVICE_UID, me())
}

fn refusal(result: anyhow::Result<FenceState>) -> String {
    format!("{:#}", result.unwrap_err())
}

#[test]
fn no_fence_file_means_ordinary_operation() {
    let dir = state_dir("absent");
    assert_eq!(
        load_at(&dir.join(FENCE_FILE), NOW).unwrap(),
        FenceState::Absent
    );
}

#[test]
fn a_valid_fence_names_exactly_one_process() {
    let dir = state_dir("valid");
    let path = write_fence(&dir, &fence_json(500, 777, NOW + 60));
    assert_eq!(
        load_at(&path, NOW).unwrap(),
        FenceState::Active {
            root_pid: 500,
            start_ticks: 777
        }
    );
}

#[test]
fn a_fence_that_is_stale_for_another_account_or_boot_or_malformed_refuses_every_mutation() {
    let dir = state_dir("refusals");
    let ok = fence_json(500, 777, NOW + 60);
    let cases = [
        (
            fence_json(500, 777, NOW),
            "validation_fence_expired_remove_it",
        ),
        (
            fence_json(500, 777, NOW - 5),
            "validation_fence_expired_remove_it",
        ),
        (
            ok.replace(BOOT, "another-boot"),
            "validation_fence_wrong_boot",
        ),
        (
            ok.replace("\"uid\":1000", "\"uid\":1001"),
            "validation_fence_wrong_account",
        ),
        (
            ok.replace("\"version\":1", "\"version\":2"),
            "unknown version",
        ),
        (
            ok.replace("\"root_pid\":500", "\"root_pid\":1"),
            "no target",
        ),
        (
            ok.replace("\"version\":1", "\"version\":1,\"allow\":\"all\""),
            "validation_fence_malformed",
        ),
        ("not json".to_owned(), "validation_fence_malformed"),
        (String::new(), "validation_fence_malformed"),
    ];
    for (body, expected) in cases {
        let path = write_fence(&dir, &body);
        let reason = refusal(load_at(&path, NOW));
        assert!(reason.contains(expected), "{body}: {reason}");
    }
}

#[test]
fn only_a_fence_written_where_only_the_trusted_account_can_write_is_honoured() {
    let dir = state_dir("trust");
    let path = write_fence(&dir, &fence_json(500, 777, NOW + 60));
    // Another account is the trusted one: this file was not written by it.
    assert!(
        refusal(load(&path, NOW, BOOT, SERVICE_UID, me().wrapping_add(1)))
            .contains("validation_fence_untrusted")
    );
    // Writable by group, or in a directory writable by others.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
    assert!(refusal(load_at(&path, NOW)).contains("validation_fence_untrusted"));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(refusal(load_at(&path, NOW)).contains("validation_fence_untrusted"));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    // A symlink standing where the fence should be is never followed.
    let real = dir.join("elsewhere.json");
    fs::rename(&path, &real).unwrap();
    symlink(&real, &path).unwrap();
    assert!(refusal(load_at(&path, NOW)).contains("validation_fence_untrusted"));
    // An oversize file.
    fs::remove_file(&path).unwrap();
    let big = write_fence(&dir, &" ".repeat(5000));
    assert!(refusal(load_at(&big, NOW)).contains("validation_fence_untrusted"));
}

fn proc_with(name: &str, processes: &[(u32, u64)]) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("stutter-fence-proc-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (pid, ticks) in processes {
        fs::create_dir_all(root.join(pid.to_string())).unwrap();
        fs::write(
            root.join(format!("{pid}/stat")),
            format!("{pid} (app) S 1 {pid} {pid} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0 0\n"),
        )
        .unwrap();
    }
    root
}

#[test]
fn a_task_passes_only_if_its_process_is_the_fenced_one_and_still_the_same_process() {
    let proc = proc_with("check", &[(500, 777), (501, 800)]);
    let fenced = FenceState::Active {
        root_pid: 500,
        start_ticks: 777,
    };
    check_task(&fenced, &proc, 500).unwrap();
    assert!(format!("{:#}", check_task(&fenced, &proc, 501).unwrap_err()).contains("not_fenced"));
    // The fenced pid was reused by a different process.
    let reused = FenceState::Active {
        root_pid: 500,
        start_ticks: 111,
    };
    assert!(format!("{:#}", check_task(&reused, &proc, 500).unwrap_err()).contains("reused"));
    // The fenced process is gone.
    fs::remove_dir_all(proc.join("500")).unwrap();
    assert!(format!("{:#}", check_task(&fenced, &proc, 500).unwrap_err()).contains("gone"));
    // Without a fence everything passes: the fence is the only thing that narrows.
    check_task(&FenceState::Absent, &proc, 501).unwrap();
}

#[test]
fn a_deny_all_session_refuses_every_target_but_is_not_malformed() {
    let dir = state_dir("deny-all");
    let path = write_fence(&dir, &fence_json(0, 0, 9_999_999_999));
    let state = load_at(&path, 1_000).expect("a deny-all fence is valid");
    assert_eq!(state, FenceState::DenyAll);
    let proc_root = proc_with("deny-all-proc", &[(500, 42)]);
    let error = check_task(&state, &proc_root, 500).unwrap_err().to_string();
    assert!(error.contains("validation_fence_deny_all"), "{error}");
    // An expired deny-all session still refuses, so a forgotten one stops tuning until removed.
    assert!(refusal(load_at(&path, 10_000_000_000)).contains("validation_fence_expired_remove_it"));
}

#[test]
fn a_process_the_fenced_one_started_is_inside_the_fence_and_a_stranger_or_reused_parent_is_not() {
    let proc = proc_with("tree", &[(500, 777), (501, 800), (502, 801), (600, 900)]);
    // 501 is a child of 500 (the fixture's helper); 502 is a child of 600 (a stranger's child).
    for (pid, parent, ticks) in [(501, 500, 800), (502, 600, 801)] {
        fs::write(
            proc.join(format!("{pid}/stat")),
            format!(
                "{pid} (app) S {parent} {pid} {pid} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0 0\n"
            ),
        )
        .unwrap();
    }
    let fenced = FenceState::Active {
        root_pid: 500,
        start_ticks: 777,
    };
    check_task(&fenced, &proc, 501).unwrap();
    assert!(format!("{:#}", check_task(&fenced, &proc, 502).unwrap_err()).contains("not_fenced"));
    assert!(format!("{:#}", check_task(&fenced, &proc, 600).unwrap_err()).contains("not_fenced"));
    // A parent pid that was reused by another process (different start time) does not qualify.
    let reused = FenceState::Active {
        root_pid: 500,
        start_ticks: 111,
    };
    assert!(format!("{:#}", check_task(&reused, &proc, 501).unwrap_err()).contains("not_fenced"));
}
