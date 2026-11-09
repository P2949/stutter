use std::{fs, path::PathBuf};

use serde_json::json;

use super::*;

/// One fake process: its command line, parent and start time.
struct Proc<'a> {
    pid: u32,
    ppid: u32,
    ticks: u64,
    cmdline: &'a str,
}

fn proc_with(name: &str, processes: &[Proc<'_>]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-iso-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    for process in processes {
        let dir = root.join(process.pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        // The kernel separates arguments with NUL.
        fs::write(dir.join("cmdline"), process.cmdline.replace(' ', "\0")).unwrap();
        fs::write(
            dir.join("stat"),
            format!(
                "{pid} (app) S {ppid} {pid} {pid} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0 0\n",
                pid = process.pid,
                ppid = process.ppid,
                ticks = process.ticks
            ),
        )
        .unwrap();
        fs::write(dir.join("comm"), "StutterAcceptan\n").unwrap();
    }
    root
}

fn p(pid: u32, cmdline: &str) -> Proc<'_> {
    Proc {
        pid,
        ppid: 1,
        ticks: 100 + u64::from(pid),
        cmdline,
    }
}

fn fixture(pid: u32, ticks: u64) -> FixtureIdentity {
    FixtureIdentity {
        pid,
        start_ticks: ticks,
        comm: "StutterAcceptan".into(),
    }
}

#[test]
fn a_real_game_is_found_whatever_launcher_or_path_style_started_it() {
    let root = proc_with(
        "games",
        &[
            p(10, r"Z:\home\u\steamapps\common\GAME\game.exe"),
            p(11, "/home/u/.steam/steamapps/common/Game/game.x86_64"),
            p(12, "/nix/store/x-gamescope/bin/gamescope -- ./play"),
            p(13, "/nix/store/x-proton-ge/files/bin/wineserver -f"),
            p(14, "/home/u/other/launcher.exe --fullscreen"),
            p(15, "/usr/bin/reaper SteamLaunch AppId=570940 -- ./game"),
            p(16, "/x/pressure-vessel/bin/pv-bwrap --bind / /"),
        ],
    );
    let found = scan(&root, &[]);
    assert_eq!(found.competitors.len(), 7, "{found:#?}");
    assert!(found.incomplete.is_empty(), "{found:#?}");
}

#[test]
fn ordinary_desktop_and_stutter_processes_are_not_competitors() {
    let root = proc_with(
        "ordinary",
        &[
            p(20, "/run/current-system/sw/bin/firefox --name firefox"),
            p(21, "/nix/store/x-vscode/lib/vscode/code --type=zygote"),
            p(
                22,
                "/nix/store/x-stutter autotune --config /etc/stutter/x.toml --mode apply-medium-risk",
            ),
            p(
                23,
                "cargo run -q -p xtask -- production-smoke --binary-sha256 abc --out out",
            ),
            p(24, "/nix/store/x-hyprland/bin/Hyprland --watchdog-fd 4"),
        ],
    );
    assert_eq!(scan(&root, &[]), Scan::default());
}

#[test]
fn a_fixture_is_exempt_only_by_exact_identity_never_by_its_command_line() {
    let dir = "/home/u/Development/stutter/.llm-runs/smoke/steamapps/common/StutterAcceptanceGame";
    let game = format!("{dir}/StutterAcceptanceGame live-workload --wine-helper");
    let helper = format!("{dir}/wineserver live-workload");
    let root = proc_with(
        "identity",
        &[
            // The fixture itself, a child it started, and a process that only has the same path.
            Proc {
                pid: 60,
                ppid: 1,
                ticks: 500,
                cmdline: &game,
            },
            Proc {
                pid: 61,
                ppid: 60,
                ticks: 510,
                cmdline: &helper,
            },
            Proc {
                pid: 62,
                ppid: 1,
                ticks: 520,
                cmdline: &game,
            },
            // The fixture's pid reused by another process after it exited.
            Proc {
                pid: 63,
                ppid: 1,
                ticks: 999,
                cmdline: &game,
            },
        ],
    );
    let fixtures = [fixture(60, 500), fixture(63, 600)];
    let found = scan(&root, &fixtures);
    let pids: Vec<&str> = found
        .competitors
        .iter()
        .map(|c| c.split(':').next().unwrap())
        .collect();
    assert_eq!(pids, ["pid 62", "pid 63"], "{found:#?}");
    assert!(found.incomplete.is_empty());
}

#[test]
fn an_uninspectable_host_blocks_instead_of_looking_empty() {
    // /proc itself cannot be listed.
    let missing = std::env::temp_dir().join(format!("stutter-iso-nowhere-{}", std::process::id()));
    let _ = fs::remove_dir_all(&missing);
    let found = scan(&missing, &[]);
    assert!(found.competitors.is_empty());
    assert_eq!(found.incomplete.len(), 1, "{found:#?}");
    assert!(found.incomplete[0].contains("cannot be enumerated"));
    assert!(!reasons_at(&missing, &[], Ok(1.0)).is_empty());

    // A command line that stays unreadable (here: a directory where the file should be).
    let root = proc_with("unreadable", &[p(30, "/usr/bin/true")]);
    fs::remove_file(root.join("30/cmdline")).unwrap();
    fs::create_dir(root.join("30/cmdline")).unwrap();
    let found = scan(&root, &[]);
    assert_eq!(found.incomplete.len(), 1, "{found:#?}");
    assert!(found.incomplete[0].contains("pid 30"));

    // A command line that is missing while its process directory is still there.
    let root = proc_with("half-gone", &[p(31, "/usr/bin/true")]);
    fs::remove_file(root.join("31/cmdline")).unwrap();
    assert_eq!(scan(&root, &[]).incomplete.len(), 1);

    // A candidate whose identity (stat) cannot be read cannot be told from a stranger.
    let root = proc_with("no-stat", &[p(32, "/x/steamapps/common/G/g.exe")]);
    fs::remove_file(root.join("32/stat")).unwrap();
    let found = scan(&root, &[]);
    assert_eq!(found.incomplete.len(), 1, "{found:#?}");
    assert!(found.competitors.is_empty());
}

#[test]
fn a_process_that_truly_disappeared_is_skipped_not_blocked() {
    let root = proc_with("gone", &[p(40, "/usr/bin/true")]);
    fs::remove_dir_all(root.join("40")).unwrap();
    assert_eq!(read_cmdline(&root, 40), Ok(None));
    assert_eq!(scan(&root, &[]), Scan::default());
}

#[test]
fn a_cpu_load_that_cannot_be_trusted_blocks_exactly_like_a_busy_host() {
    let root = proc_with("busy", &[p(50, "/usr/bin/true")]);
    assert!(reasons_at(&root, &[], Ok(0.0)).is_empty());
    assert!(reasons_at(&root, &[], Ok(MAX_HOST_BUSY_PERCENT)).is_empty());
    let busy = reasons_at(&root, &[], Ok(MAX_HOST_BUSY_PERCENT + 1.0));
    assert_eq!(busy.len(), 1);
    assert!(busy[0].contains("busy"), "{busy:?}");
    // No sample, a malformed one, no elapsed time, and numbers that are not percentages.
    for bad in [
        Err("stat unreadable".to_owned()),
        Err("its cpu line is malformed".to_owned()),
        Ok(f64::NAN),
        Ok(f64::INFINITY),
        Ok(-5.0),
    ] {
        let reasons = reasons_at(&root, &[], bad.clone());
        assert_eq!(reasons.len(), 1, "{bad:?}: {reasons:?}");
        assert!(reasons[0].contains("inspection incomplete"), "{reasons:?}");
    }
}

#[test]
fn cpu_busy_share_is_computed_from_two_proc_stat_samples_and_bad_input_is_no_reading() {
    let first = "cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 1 1 1 1\n";
    let second = "cpu  200 0 200 750 150 0 0 0 0 0\n";
    let (a, b) = (
        parse_cpu_line(first).unwrap(),
        parse_cpu_line(second).unwrap(),
    );
    assert_eq!(a, (200, 1000));
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
fn a_fixture_is_registered_by_its_exact_identity_or_not_at_all() {
    let root = proc_with(
        "register",
        &[Proc {
            pid: 70,
            ppid: 1,
            ticks: 777,
            cmdline: "x",
        }],
    );
    record_fixture(&root, 70).unwrap();
    assert!(fixtures().contains(&fixture(70, 777)));
    // No stat, no identity: refused.
    fs::remove_file(root.join("70/stat")).unwrap();
    assert!(record_fixture(&root, 70).is_err());
    // An empty or missing comm: refused.
    let root = proc_with("register2", &[p(71, "x")]);
    fs::write(root.join("71/comm"), "\n").unwrap();
    assert!(record_fixture(&root, 71).is_err());
    fs::remove_file(root.join("71/comm")).unwrap();
    assert!(record_fixture(&root, 71).is_err());
}

fn audit_of(records: Vec<serde_json::Value>) -> HistoryAudit {
    HistoryAudit {
        records,
        ..Default::default()
    }
}

fn record(decision: &str, target: serde_json::Value) -> serde_json::Value {
    json!({"decision": {"decision": decision}, "target": target})
}

fn target(
    pid: serde_json::Value,
    comm: serde_json::Value,
    ticks: serde_json::Value,
) -> serde_json::Value {
    json!({"root_pid": pid, "process_comm": comm, "process_starttime_ticks": ticks})
}

#[test]
fn every_state_change_must_be_on_a_fixture_started_by_this_run() {
    let fixtures = [fixture(100, 5000), fixture(101, 5100)];
    let good = audit_of(vec![
        record(
            "observed",
            target(json!(5555), json!("firefox"), json!(null)),
        ),
        record(
            "candidate_applied",
            target(json!(100), json!("StutterAcceptan"), json!(5000)),
        ),
        record(
            "candidate_kept",
            target(json!(101), json!("StutterAcceptan"), json!(5100)),
        ),
        record(
            "cooldown_entered",
            target(json!(777), json!("x"), json!(null)),
        ),
    ]);
    let check = only_fixture_targets(&good, &fixtures);
    assert!(check.passed, "{}", check.detail);

    let unticked = audit_of(vec![record(
        "candidate_applied",
        target(json!(100), json!("StutterAcceptan"), json!(null)),
    )]);
    let check = only_fixture_targets(&unticked, &fixtures);
    assert!(
        !check.passed,
        "a record with no start time is not exact identity"
    );
    assert!(
        check.detail.contains("carries no process start time"),
        "{}",
        check.detail
    );

    let stranger = audit_of(vec![record(
        "candidate_applied",
        target(json!(4242), json!("StutterAcceptan"), json!(null)),
    )]);
    let check = only_fixture_targets(&stranger, &fixtures);
    assert!(!check.passed);
    assert!(check.detail.contains("4242"), "{}", check.detail);
    assert!(
        !only_fixture_targets(
            &audit_of(vec![record(
                "restored",
                target(json!(100), json!("StutterAcceptan"), json!(null))
            )]),
            &[]
        )
        .passed
    );
    assert!(only_fixture_targets(&audit_of(Vec::new()), &[]).passed);
}

#[test]
fn a_state_change_that_cannot_be_attributed_fails_instead_of_disappearing() {
    let fixtures = [fixture(100, 5000)];
    let cases = [
        // No target at all, a null pid, a string pid, a missing pid.
        record("candidate_applied", json!(null)),
        record(
            "candidate_applied",
            target(json!(null), json!("StutterAcceptan"), json!(null)),
        ),
        record(
            "candidate_applied",
            target(json!("100"), json!("StutterAcceptan"), json!(null)),
        ),
        record(
            "candidate_applied",
            json!({"process_comm": "StutterAcceptan"}),
        ),
        // No comm, the wrong comm.
        record(
            "candidate_applied",
            target(json!(100), json!(null), json!(null)),
        ),
        record(
            "candidate_applied",
            target(json!(100), json!("firefox"), json!(null)),
        ),
        // The right pid but a start time that is not the fixture's: a reused pid.
        record(
            "candidate_applied",
            target(json!(100), json!("StutterAcceptan"), json!(1234)),
        ),
    ];
    for case in cases {
        let check = only_fixture_targets(&audit_of(vec![case.clone()]), &fixtures);
        assert!(!check.passed, "{case}: {}", check.detail);
    }
}

#[test]
fn an_incomplete_history_fails_the_audit_whatever_it_contains() {
    let fixtures = [fixture(100, 5000)];
    let ok = record(
        "candidate_applied",
        target(json!(100), json!("StutterAcceptan"), json!(null)),
    );
    let mut audit = audit_of(vec![ok]);
    audit.shrank = true;
    assert!(
        only_fixture_targets(&audit, &fixtures)
            .detail
            .contains("rotated or replaced")
    );
    let mut audit = audit_of(Vec::new());
    audit.unparsable = 2;
    assert!(!only_fixture_targets(&audit, &fixtures).passed);
    let mut audit = audit_of(Vec::new());
    audit.unreadable = Some("denied".into());
    assert!(!only_fixture_targets(&audit, &fixtures).passed);
}

#[test]
fn the_history_is_read_strictly_from_where_the_smoke_began() {
    let path = std::env::temp_dir().join(format!("stutter-iso-history-{}", std::process::id()));
    let _ = fs::remove_file(&path);
    let first = "{\"before\":1}\n";
    fs::write(&path, first).unwrap();
    let start = first.len() as u64;
    // Two records, one corrupt line, and an append still in flight.
    fs::write(
        &path,
        format!("{first}{{\"a\":1}}\nnot json\n{{\"b\":2}}\n{{\"partial\":"),
    )
    .unwrap();
    let audit = audit_history(&path, start);
    assert_eq!(audit.records.len(), 2);
    assert_eq!(audit.unparsable, 1);
    assert!(!audit.shrank && audit.unreadable.is_none());
    // The file was rotated to something shorter.
    fs::write(&path, "x\n").unwrap();
    assert!(audit_history(&path, start).shrank);
    // Missing: fine if it never existed, unreadable if it had content when the smoke began.
    fs::remove_file(&path).unwrap();
    assert!(audit_history(&path, 0).unreadable.is_none());
    assert!(audit_history(&path, start).unreadable.is_some());
}

#[test]
fn the_fence_the_smoke_writes_has_exactly_the_fields_the_worker_reads_and_nothing_else() {
    let body = fence_json(1000, 4242, 777, "boot-1\n", 9_999);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    // The worker refuses unknown fields, so any drift here makes every mutation refused.
    assert_eq!(
        keys,
        [
            "boot_id",
            "expires_unix",
            "root_pid",
            "start_ticks",
            "uid",
            "version"
        ]
    );
    assert_eq!(value["version"], 1);
    assert_eq!(value["uid"], 1000);
    assert_eq!(value["root_pid"], 4242);
    assert_eq!(value["start_ticks"], 777);
    assert_eq!(value["boot_id"], "boot-1", "the boot id is trimmed");
    assert_eq!(value["expires_unix"], 9_999);
}

#[test]
fn an_unarmed_fence_refuses_instead_of_silently_running_unfenced() {
    // Not armed in this test process: a mutation case must not proceed, and no sudo call is made.
    assert!(write_fence(1, 60).is_err());
    assert!(deny_all(60).is_err());
}
