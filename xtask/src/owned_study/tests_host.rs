//! E11: the host preflight is Safe, Busy or Incomplete, and never reads unreadable as "no game".
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use super::{
    host::{HostState, inspect},
    tests::scratch,
};

fn add(root: &Path, pid: u32, comm: &str, cmdline: &str) {
    let dir = root.join(pid.to_string());
    fs::create_dir_all(&dir).expect("dir");
    fs::write(dir.join("comm"), format!("{comm}\n")).expect("comm");
    fs::write(dir.join("cmdline"), cmdline.replace(' ', "\0")).expect("cmdline");
    // Field 22 (start time) is the 20th after the command name's closing parenthesis.
    fs::write(
        dir.join("stat"),
        format!("{pid} ({comm}) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 424242 0 0"),
    )
    .expect("stat");
}

#[test]
fn an_idle_table_and_a_valid_owned_process_are_safe_and_the_callers_own_pid_is_ignored() {
    let root = scratch("host-safe");
    add(&root, 10, "stress-ng", "stress-ng --cpu 12");
    add(&root, 11, "cargo", "cargo build --offline");
    add(&root, 12, "gamescope", "gamescope -- game");
    assert_eq!(
        inspect(&root, 12),
        HostState::Safe,
        "the caller's own pid is excluded"
    );
    assert_eq!(inspect(&scratch("host-empty"), 1), HostState::Safe);
}

#[test]
fn a_game_or_launcher_is_busy() {
    for (comm, cmdline) in [
        ("gamescope", "gamescope -f -- %command%"),
        ("game.exe", "proton run steamapps/common/Game/game.exe"),
        ("reaper", "reaper SteamLaunch AppId=12345 -- x"),
        ("pv-bwrap", "bwrap pressure-vessel-adverb"),
    ] {
        let root = scratch("host-busy");
        add(&root, 20, "bash", "bash");
        add(&root, 21, comm, cmdline);
        assert!(matches!(inspect(&root, 1), HostState::Busy(_)), "{comm}");
    }
}

#[test]
fn a_shell_that_only_mentions_a_game_is_not_a_game() {
    let root = scratch("host-mention");
    add(&root, 30, "zsh", "zsh -c grep gamescope");
    add(&root, 31, "grep", "grep steamapps/common");
    add(
        &root,
        32,
        "xtask",
        "xtask owned-study run --manifest SteamLaunch AppId",
    );
    assert_eq!(inspect(&root, 1), HostState::Safe);
}

#[test]
fn an_unreadable_process_table_is_incomplete_never_safe() {
    let missing = scratch("host-missing").join("absent");
    assert!(matches!(inspect(&missing, 1), HostState::Incomplete(_)));
    let not_a_directory = scratch("host-file").join("proc");
    fs::write(&not_a_directory, "x").expect("file");
    assert!(matches!(
        inspect(&not_a_directory, 1),
        HostState::Incomplete(_)
    ));
}

#[test]
fn a_denied_or_broken_entry_is_incomplete_not_ignored() {
    let root = scratch("host-denied");
    add(&root, 40, "cargo", "cargo build");
    let cmdline = root.join("40/cmdline");
    fs::set_permissions(&cmdline, fs::Permissions::from_mode(0o000)).expect("chmod");
    if fs::read(&cmdline).is_err() {
        assert!(
            matches!(inspect(&root, 1), HostState::Incomplete(_)),
            "hidden/denied cmdline"
        );
    }
    fs::set_permissions(&cmdline, fs::Permissions::from_mode(0o600)).expect("restore");
    let root = scratch("host-broken");
    add(&root, 41, "cargo", "cargo build");
    fs::remove_file(root.join("41/comm")).expect("rm");
    fs::create_dir(root.join("41/comm"))
        .expect("comm as a directory: a read error that is not NotFound");
    assert!(matches!(inspect(&root, 1), HostState::Incomplete(_)));
}

#[test]
fn a_process_that_disappears_or_is_reused_mid_read_is_skipped() {
    let root = scratch("host-vanish");
    fs::create_dir_all(root.join("50"))
        .expect("a pid directory with nothing in it: exited between listing and reading");
    add(&root, 51, "cargo", "cargo build");
    assert_eq!(inspect(&root, 1), HostState::Safe);
    // A garbled stat has no start time to compare: treated as a vanished process, not as a game.
    fs::write(root.join("51/stat"), "garbage").expect("stat");
    assert_eq!(inspect(&root, 1), HostState::Safe);
}

#[test]
fn busy_takes_precedence_over_incomplete() {
    let root = scratch("host-both");
    add(&root, 60, "cargo", "cargo build");
    fs::remove_file(root.join("60/comm")).expect("rm");
    fs::create_dir(root.join("60/comm")).expect("broken");
    add(&root, 61, "gamescope", "gamescope");
    assert!(matches!(inspect(&root, 1), HostState::Busy(_)));
}
