//! Prefilter tests against realistic Steam and kernel process layouts.
use super::*;

#[test]
fn steams_outer_launcher_is_exempt_only_as_an_ancestor_of_the_pinned_client() {
    fn add(proc: &Path, pid: u32, parent: u32, ticks: u64, comm: &str) {
        let path = proc.join(pid.to_string());
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("comm"), format!("{comm}\n")).unwrap();
        std::fs::write(
            path.join("stat"),
            format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0"),
        )
        .unwrap();
    }
    let proc = tempfile::tempdir().unwrap();
    // Real layout: outer steam -> srt-bwrap -> pinned inner steam -> web helper.
    add(proc.path(), 200, 1, 20, "steam");
    add(proc.path(), 201, 200, 21, "srt-bwrap");
    add(proc.path(), 202, 201, 22, "steam");
    add(proc.path(), 203, 202, 23, "steamwebhelper");
    let client = SteamProcessIdentity {
        steam_pid: 202,
        steam_start_ticks: 22,
        webhelper_pid: 203,
        webhelper_start_ticks: 23,
        logged_in: true,
    };
    let uid = crate::syscall::geteuid();
    assert!(!host_has_game_or_foreign_steam(proc.path(), &client, uid));
    // An unrelated steam process is still foreign.
    add(proc.path(), 300, 1, 30, "steam");
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
}

#[test]
fn long_kernel_thread_names_are_not_mistaken_for_unknown_or_busy_processes() {
    fn add(proc: &Path, pid: u32, parent: u32, ticks: u64, comm: &str) {
        let path = proc.join(pid.to_string());
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("comm"), format!("{comm}\n")).unwrap();
        std::fs::write(
            path.join("stat"),
            format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0"),
        )
        .unwrap();
    }
    let proc = tempfile::tempdir().unwrap();
    add(proc.path(), 100, 1, 11, "steam");
    add(proc.path(), 101, 100, 12, "steamwebhelper");
    let client = SteamProcessIdentity {
        steam_pid: 100,
        steam_start_ticks: 11,
        webhelper_pid: 101,
        webhelper_start_ticks: 12,
        logged_in: true,
    };
    let uid = crate::syscall::geteuid();
    add(proc.path(), 2, 0, 1, "kworker/0:0H-events_highpri");
    assert!(!host_has_game_or_foreign_steam(proc.path(), &client, uid));
    // An unbounded name is still unknown, and an empty one is never evidence of a free host.
    add(proc.path(), 3, 0, 1, &"k".repeat(200));
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
    std::fs::remove_dir_all(proc.path().join("3")).unwrap();
    std::fs::write(proc.path().join("2/comm"), "").unwrap();
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
}
