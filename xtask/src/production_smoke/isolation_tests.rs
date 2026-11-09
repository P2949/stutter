use std::{fs, path::PathBuf};

use serde_json::json;

use super::*;

fn proc_with(name: &str, commands: &[(u32, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-iso-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (pid, command) in commands {
        fs::create_dir_all(root.join(pid.to_string())).unwrap();
        // The kernel separates arguments with NUL.
        fs::write(
            root.join(pid.to_string()).join("cmdline"),
            command.replace(' ', "\0"),
        )
        .unwrap();
    }
    root
}

const OUT: &str = "/home/p2949/Development/stutter/.llm-runs/smoke";

#[test]
fn a_real_game_is_found_whatever_launcher_or_path_style_started_it() {
    let root = proc_with(
        "games",
        &[
            // A Steam library path with Windows backslashes, as Proton reports it.
            (10, r"Z:\home\u\steamapps\common\GAME\game.exe"),
            // Forward slashes, no Windows names.
            (11, "/home/u/.steam/steamapps/common/Game/game.x86_64"),
            // No steamapps in the path at all.
            (12, "/nix/store/x-gamescope/bin/gamescope -- ./play"),
            (13, "/nix/store/x-proton-ge/files/bin/wineserver -f"),
            (14, "/home/u/other/launcher.exe --fullscreen"),
            (15, "/usr/bin/reaper SteamLaunch AppId=570940 -- ./game"),
            (16, "/x/pressure-vessel/bin/pv-bwrap --bind / /"),
        ],
    );
    let found = competing_workloads(&root, &PathBuf::from(OUT));
    assert_eq!(found.len(), 7, "{found:#?}");
}

#[test]
fn ordinary_desktop_and_stutter_processes_are_not_competitors() {
    let root = proc_with(
        "ordinary",
        &[
            (20, "/run/current-system/sw/bin/firefox --name firefox"),
            (21, "/nix/store/x-vscode/lib/vscode/code --type=zygote"),
            (
                22,
                "/nix/store/x-stutter autotune --config /etc/stutter/x.toml --mode apply-medium-risk",
            ),
            (
                23,
                "cargo run -q -p xtask -- production-smoke --binary-sha256 abc --out out",
            ),
            (24, "/nix/store/x-hyprland/bin/Hyprland --watchdog-fd 4"),
        ],
    );
    assert!(competing_workloads(&root, &PathBuf::from(OUT)).is_empty());
}

#[test]
fn the_smokes_own_fixture_under_its_output_directory_is_not_a_competitor() {
    let fixture = format!(
        "{OUT}/steamapps/common/StutterAcceptanceGame/StutterAcceptanceGame live-workload --wine-helper"
    );
    let helper = format!("{OUT}/steamapps/common/StutterAcceptanceGame/wineserver live-workload");
    let root = proc_with(
        "own",
        &[
            (30, &fixture),
            (31, &helper),
            (32, "/home/u/steamapps/common/G/g.exe"),
        ],
    );
    let found = competing_workloads(&root, &PathBuf::from(OUT));
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].starts_with("pid 32"));
}

#[test]
fn a_competitor_that_disappears_and_reappears_is_seen_each_time() {
    let mut root = proc_with("flap", &[(40, "/x/steamapps/common/G/g.exe")]);
    let own = PathBuf::from(OUT);
    assert_eq!(reasons_at(&root, &own, Some(5.0)).len(), 1);
    fs::remove_dir_all(root.join("40")).unwrap();
    assert!(
        reasons_at(&root, &own, Some(5.0)).is_empty(),
        "gone: quiet again"
    );
    root = proc_with("flap", &[(41, "/x/proton/bin/wine64 game")]);
    assert_eq!(
        reasons_at(&root, &own, Some(5.0)).len(),
        1,
        "back under another name"
    );
}

#[test]
fn a_busy_host_is_not_quiet_even_with_no_game_in_sight() {
    let root = proc_with("busy", &[(50, "/usr/bin/true")]);
    let own = PathBuf::from(OUT);
    assert!(reasons_at(&root, &own, Some(MAX_HOST_BUSY_PERCENT)).is_empty());
    let reasons = reasons_at(&root, &own, Some(MAX_HOST_BUSY_PERCENT + 1.0));
    assert_eq!(reasons.len(), 1);
    assert!(reasons[0].contains("busy"), "{reasons:?}");
    // An unreadable sample is not a reason by itself, only the processes are.
    assert!(reasons_at(&root, &own, None).is_empty());
}

#[test]
fn cpu_busy_share_is_computed_from_two_proc_stat_samples() {
    let first = "cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 1 1 1 1\n";
    let second = "cpu  200 0 200 750 150 0 0 0 0 0\n";
    let (a, b) = (
        parse_cpu_line(first).unwrap(),
        parse_cpu_line(second).unwrap(),
    );
    assert_eq!(a, (200, 1000));
    // 200 of 300 new jiffies were busy (idle and iowait grew by 100).
    assert!((busy_percent(a, b).unwrap() - 200.0 / 300.0 * 100.0).abs() < 1e-9);
    assert_eq!(
        busy_percent(b, a),
        None,
        "counters that go backwards are not a reading"
    );
    assert_eq!(busy_percent(a, a), None, "no elapsed time is not a reading");
    assert_eq!(parse_cpu_line("garbage"), None);
    assert_eq!(parse_cpu_line("cpu  1 2\n"), None);
}

#[test]
fn every_state_change_must_be_on_a_fixture_this_run_started() {
    let record = |decision: &str, pid: u32| json!({"decision": {"decision": decision}, "target": {"root_pid": pid}});
    let fixtures = [100_u32, 101];
    let good = [
        record("observed", 5555),
        record("candidate_applied", 100),
        record("candidate_kept", 101),
        record("cooldown_entered", 777),
    ];
    let check = only_fixture_targets(&good, &fixtures);
    assert!(check.passed, "{}", check.detail);
    // Observing or cooling down on someone else is policy activity, not a change.
    let bad = [
        record("candidate_applied", 100),
        record("candidate_applied", 4242),
    ];
    let check = only_fixture_targets(&bad, &fixtures);
    assert!(!check.passed);
    assert!(check.detail.contains("4242"), "{}", check.detail);
    // With no fixtures at all, any state change is a stranger.
    assert!(!only_fixture_targets(&[record("restored", 100)], &[]).passed);
    assert!(only_fixture_targets(&[], &[]).passed);
}
