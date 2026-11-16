use std::{
    os::unix::fs::{PermissionsExt, symlink},
    time::Duration,
};

use super::*;

fn installation() -> CyberpunkInstallation {
    CyberpunkInstallation {
        app_id: 1_091_500,
        build_id: 42,
        installation_directory: "Cyberpunk 2077".into(),
        manifest_update_status: super::super::SteamUpdateStatus::NoPendingUpdateIndicated,
        manifest_sha256: "a".repeat(64),
        game_executable_sha256: Some("b".repeat(64)),
        launcher_executable_sha256: None,
        entitlement_verified: true,
        native_benchmark_verified: false,
    }
}

fn process() -> CyberpunkProcessIdentity {
    CyberpunkProcessIdentity {
        pid: Pid::new(1234),
        start_ticks: 56,
        uid: crate::syscall::geteuid(),
        app_id: 1_091_500,
        build_id: 42,
        executable_sha256: "b".repeat(64),
    }
}

fn attempt(
    installation: CyberpunkInstallation,
    process: CyberpunkProcessIdentity,
    started_unix_nanos: u128,
    completed_unix_nanos: u128,
) -> CyberpunkBenchmarkAttempt {
    CyberpunkBenchmarkAttempt {
        installation,
        process,
        scene: CyberpunkScenario::BuiltinBenchmark,
        settings_sha256: "c".repeat(64),
        proton: super::super::ResearchTool {
            source: ProtonSource::Ge,
            identity_sha256: "d".repeat(64),
        },
        started_unix_nanos,
        completed_unix_nanos,
        loaded_runtime: None,
    }
}

fn prepare_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn complete(
    root: &Path,
    snapshot: &BenchmarkDirectorySnapshot,
) -> anyhow::Result<CyberpunkBenchmarkCompletion> {
    let process = process();
    let started = snapshot.captured_unix_nanos + 1;
    std::thread::sleep(Duration::from_millis(2));
    let completed = now_unix_nanos().unwrap();
    verify_benchmark_completion(
        root,
        snapshot,
        &attempt(installation(), process, started, completed),
    )
}

fn write_result(root: &Path, name: &str, summary: &str) {
    let dir = root.join(name);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    // The real summary carries `frameNumber`; fixtures with a usable average get the fixture
    // frame file's count (kept verbatim otherwise, so malformed summaries stay malformed).
    let mut text = summary.to_owned();
    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(summary)
        && let Some(root_object) = value.as_object_mut()
    {
        let target = if root_object
            .get("Data")
            .is_some_and(serde_json::Value::is_object)
        {
            root_object
                .get_mut("Data")
                .and_then(|data| data.as_object_mut())
        } else {
            Some(root_object)
        };
        if let Some(target) = target
            && target
                .get("averageFps")
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|fps| fps > 0.0)
            && !target.contains_key("frameNumber")
        {
            target.insert("frameNumber".into(), 50.into());
            text = value.to_string();
        }
    }
    std::fs::write(dir.join("summary.json"), text).unwrap();
    // The game derives its average from the frame times, so the fixture frame file is coherent with
    // its summary whenever the summary has a usable average.
    let frames = serde_json::from_str::<serde_json::Value>(summary)
        .ok()
        .and_then(|value| value["averageFps"].as_f64())
        .filter(|fps| *fps > 0.0)
        .map_or_else(
            || "frame,time\n1,0\n".to_owned(),
            |fps| {
                let mut csv = String::from("Frame index, Frame time (ms)\n");
                for index in 0..50 {
                    csv.push_str(&format!("{index}, {:.4}\n", 1000.0 / fps));
                }
                csv
            },
        );
    std::fs::write(dir.join("frames.csv"), frames).unwrap();
}

#[test]
fn fresh_native_result_is_bound_to_build_process_settings_and_raw_hashes() {
    let root = prepare_root();
    let before = snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).unwrap();
    write_result(
        root.path(),
        "benchmark_2026-11-11_20-00-00",
        r#"{"averageFps": 92.5, "minFps": 62, "maxFps": 121}"#,
    );
    let evidence = complete(root.path(), &before).unwrap();
    assert_eq!(evidence.app_id, 1_091_500);
    assert_eq!(evidence.build_id, 42);
    assert_eq!(evidence.metrics.average_fps, 92.5);
    assert_eq!(evidence.metrics.minimum_fps, Some(62.0));
    assert_eq!(evidence.files.len(), 2);
    assert!(evidence.files.iter().all(|file| file.sha256.len() == 64));
    assert!(
        serde_json::from_slice::<CyberpunkBenchmarkCompletion>(
            &serde_json::to_vec(&evidence).unwrap()
        )
        .is_ok()
    );
}

#[test]
fn native_probe_uses_the_verifier_and_rejects_stale_results() {
    use std::sync::{Arc, Mutex};

    use super::super::{
        native_probe::{NativeObservationSource, NativeResultProbe, Observation},
        steam_host::GameState,
        steam_port::ResultProbe,
    };
    struct Source(Arc<Mutex<Option<CyberpunkBenchmarkAttempt>>>);
    impl NativeObservationSource for Source {
        fn begin(&mut self, _: super::super::ProtonSource) -> anyhow::Result<()> {
            Ok(())
        }
        fn poll(&mut self) -> Observation {
            self.0
                .lock()
                .unwrap()
                .take()
                .map_or(Observation::Running, |attempt| {
                    Observation::Completed(Box::new(attempt))
                })
        }
    }
    let root = prepare_root();
    let observed = Arc::new(Mutex::new(None));
    let mut probe = NativeResultProbe::new(
        root.path().to_path_buf(),
        crate::syscall::geteuid(),
        Source(observed.clone()),
    );
    probe.begin(super::super::ProtonSource::Ge).unwrap();
    let started = now_unix_nanos().unwrap();
    std::thread::sleep(Duration::from_millis(2));
    write_result(root.path(), "new", r#"{"averageFps": 92.5}"#);
    let completed = now_unix_nanos().unwrap();
    *observed.lock().unwrap() = Some(attempt(installation(), process(), started, completed));
    let GameState::Result { completion } = probe.poll() else {
        panic!("fresh native result was not verified")
    };
    assert_eq!(completion.metrics.average_fps, 92.5);
    assert!(!completion.files.is_empty());
    probe.begin(super::super::ProtonSource::Ge).unwrap();
    let started = now_unix_nanos().unwrap();
    std::thread::sleep(Duration::from_millis(2));
    *observed.lock().unwrap() = Some(attempt(
        installation(),
        process(),
        started,
        now_unix_nanos().unwrap(),
    ));
    assert_eq!(probe.poll(), GameState::InvalidNativeResult);
}

#[test]
fn native_probe_preserves_process_observation_outcomes() {
    use super::super::{
        native_probe::{NativeObservationSource, NativeResultProbe, Observation},
        steam_host::GameState,
        steam_port::ResultProbe,
    };
    struct Source(std::collections::VecDeque<Observation>);
    impl NativeObservationSource for Source {
        fn begin(&mut self, _: super::super::ProtonSource) -> anyhow::Result<()> {
            Ok(())
        }
        fn poll(&mut self) -> Observation {
            self.0.pop_front().unwrap()
        }
    }
    let root = prepare_root();
    let source = Source(
        [
            Observation::Starting,
            Observation::Running,
            Observation::ExitedWithoutResult,
            Observation::IdentityMismatch,
        ]
        .into(),
    );
    let mut probe =
        NativeResultProbe::new(root.path().to_path_buf(), crate::syscall::geteuid(), source);
    probe.begin(super::super::ProtonSource::Ge).unwrap();
    assert_eq!(probe.poll(), GameState::Starting);
    assert_eq!(probe.poll(), GameState::Running);
    assert_eq!(probe.poll(), GameState::ExitedWithoutResult);
    assert_eq!(probe.poll(), GameState::IdentityMismatch);
}

#[test]
fn stale_or_ambiguous_native_results_are_not_accepted() {
    let root = prepare_root();
    write_result(root.path(), "old", r#"{"averageFps": 60}"#);
    let before = snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).unwrap();
    assert!(complete(root.path(), &before).is_err());
    write_result(root.path(), "new-one", r#"{"averageFps": 60}"#);
    write_result(root.path(), "new-two", r#"{"averageFps": 61}"#);
    assert!(complete(root.path(), &before).is_err());
}

#[test]
fn malformed_summary_and_invalid_metrics_are_rejected() {
    for summary in [
        "not-json",
        "[]",
        r#"{"averageFps": 0}"#,
        r#"{"averageFps": "fast"}"#,
        r#"{"averageFps": 50, "minFps": 51}"#,
        r#"{"averageFps": 50, "maxFps": 49}"#,
    ] {
        let root = prepare_root();
        let before = snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).unwrap();
        write_result(root.path(), "one", summary);
        assert!(
            complete(root.path(), &before).is_err(),
            "accepted {summary}"
        );
    }
}

#[test]
fn symlinks_wrong_process_identity_and_bad_digests_fail_closed() {
    let root = prepare_root();
    let target = root.path().join("target");
    std::fs::create_dir(&target).unwrap();
    symlink(&target, root.path().join("alias")).unwrap();
    assert!(snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).is_err());

    let root = prepare_root();
    let before = snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).unwrap();
    write_result(root.path(), "one", r#"{"averageFps": 50}"#);
    let mut bad_process = process();
    bad_process.start_ticks = 0;
    let now = now_unix_nanos().unwrap();
    assert!(
        verify_benchmark_completion(
            root.path(),
            &before,
            &attempt(
                installation(),
                bad_process,
                before.captured_unix_nanos + 1,
                now
            ),
        )
        .is_err()
    );
    let mut bad_digest = attempt(
        installation(),
        process(),
        before.captured_unix_nanos + 1,
        now,
    );
    bad_digest.settings_sha256 = "bad".into();
    assert!(verify_benchmark_completion(root.path(), &before, &bad_digest).is_err());
}

#[test]
fn installation_without_owned_steam_entitlement_or_frozen_update_is_rejected() {
    let root = prepare_root();
    let before = snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).unwrap();
    write_result(root.path(), "one", r#"{"averageFps": 50}"#);
    let now = now_unix_nanos().unwrap();
    let mut unowned = installation();
    unowned.entitlement_verified = false;
    assert!(
        verify_benchmark_completion(
            root.path(),
            &before,
            &attempt(unowned, process(), before.captured_unix_nanos + 1, now),
        )
        .is_err()
    );

    let mut updating = installation();
    updating.manifest_update_status = super::super::SteamUpdateStatus::Updating;
    assert!(
        verify_benchmark_completion(
            root.path(),
            &before,
            &attempt(updating, process(), before.captured_unix_nanos + 1, now),
        )
        .is_err()
    );
}

#[test]
fn unsafe_or_unbounded_native_result_trees_are_rejected() {
    let root = prepare_root();
    let before = snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).unwrap();
    let result = root.path().join("one");
    std::fs::create_dir(&result).unwrap();
    std::fs::set_permissions(&result, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = root.path().join("outside.json");
    std::fs::write(&target, r#"{"averageFps": 50}"#).unwrap();
    symlink(&target, result.join("summary.json")).unwrap();
    assert!(complete(root.path(), &before).is_err());

    let root = prepare_root();
    let before = snapshot_benchmark_results(root.path(), crate::syscall::geteuid()).unwrap();
    let mut dir = root.path().join("deep");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    for depth in 0..=MAX_RESULT_DEPTH {
        dir.push(format!("d{depth}"));
        std::fs::create_dir(&dir).unwrap();
    }
    std::fs::write(dir.join("summary.json"), r#"{"averageFps": 50}"#).unwrap();
    assert!(complete(root.path(), &before).is_err());
}

#[test]
fn an_absent_results_directory_is_an_empty_baseline_that_still_needs_a_fresh_result() {
    let parent = prepare_root();
    let results = parent.path().join("benchmarkResults");
    let uid = crate::syscall::geteuid();
    let before = snapshot_benchmark_results_or_absent(&results, uid).unwrap();
    assert!(before.directories.is_empty());
    // Nothing was created in the prefix by taking the baseline.
    assert!(!results.exists());
    // If the game never produced the directory there is no result to accept.
    assert!(complete(&results, &before).is_err());
    // Once the game creates it with one fresh folder, that folder is the result.
    std::fs::create_dir(&results).unwrap();
    std::fs::set_permissions(&results, std::fs::Permissions::from_mode(0o700)).unwrap();
    write_result(&results, "new", r#"{"averageFps": 61.0}"#);
    let completion = complete(&results, &before).unwrap();
    assert_eq!(completion.metrics.average_fps, 61.0);
    assert!(!completion.files.is_empty());
}

#[test]
fn only_a_genuinely_absent_path_is_an_empty_baseline() {
    let uid = crate::syscall::geteuid();
    let parent = prepare_root();
    // A symlinked component is an error, never an absent directory.
    let real = parent.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, parent.path().join("link")).unwrap();
    assert!(
        snapshot_benchmark_results_or_absent(&parent.path().join("link/benchmarkResults"), uid)
            .is_err()
    );
    // A non-directory where the directory should be is an error too.
    std::fs::write(parent.path().join("benchmarkResults"), b"x").unwrap();
    assert!(
        snapshot_benchmark_results_or_absent(&parent.path().join("benchmarkResults"), uid).is_err()
    );
    // The strict snapshot still refuses an absent directory.
    assert!(snapshot_benchmark_results(&parent.path().join("missing"), uid).is_err());
    // An unprivileged owner is still required.
    assert!(snapshot_benchmark_results_or_absent(&parent.path().join("missing"), 0).is_err());
}

#[test]
fn the_games_wrapped_summary_schema_is_parsed_and_other_roots_are_refused() {
    let real = br#"{"RootType":"worldBenchmarkSummary","Data":{"gameVersion":"2.5","presetName":"RayTracingUltra","averageFps":68.34229278564453,"minFps":53.859867095947269,"maxFps":81.63131713867188}}"#;
    let metrics = super::parse_summary(real).unwrap();
    assert!((metrics.average_fps - 68.342).abs() < 0.001);
    assert!(metrics.minimum_fps.is_some() && metrics.maximum_fps.is_some());
    let other = br#"{"RootType":"somethingElse","Data":{"averageFps":60.0}}"#;
    assert!(super::parse_summary(other).is_err());
    let wrapped_without_root = br#"{"Data":{"averageFps":60.0}}"#;
    assert!(super::parse_summary(wrapped_without_root).is_err());
    assert!(super::parse_summary(br#"{"averageFps":60.0}"#).is_ok());
    assert!(
        super::parse_summary(br#"{"RootType":"worldBenchmarkSummary","Data":{"averageFps":0.0}}"#)
            .is_err()
    );
}

fn sealed(root: &Path) -> CyberpunkBenchmarkCompletion {
    let before = snapshot_benchmark_results(root, crate::syscall::geteuid()).unwrap();
    write_result(
        root,
        "run",
        r#"{"averageFps": 60.0, "minFps": 40.0, "maxFps": 80.0}"#,
    );
    complete(root, &before).unwrap()
}

#[test]
fn a_sealed_completion_replays_against_its_untouched_files() {
    let root = prepare_root();
    let completion = sealed(root.path());
    reverify_completion(root.path(), &completion).unwrap();
}

#[test]
fn an_edited_missing_added_or_substituted_native_file_fails_the_replay() {
    let edits: [fn(&Path); 5] = [
        |dir| {
            std::fs::write(
                dir.join("frames.csv"),
                b"Frame index, Frame time (ms)\n0, 1.0\n",
            )
            .unwrap()
        },
        |dir| std::fs::remove_file(dir.join("frames.csv")).unwrap(),
        |dir| std::fs::write(dir.join("extra.txt"), b"x").unwrap(),
        |dir| {
            let text = std::fs::read_to_string(dir.join("summary.json")).unwrap();
            std::fs::write(dir.join("summary.json"), text.replace("60", "61")).unwrap();
        },
        |dir| {
            std::fs::remove_file(dir.join("summary.json")).unwrap();
            symlink("frames.csv", dir.join("summary.json")).unwrap();
        },
    ];
    for edit in edits {
        let root = prepare_root();
        let completion = sealed(root.path());
        edit(&root.path().join("run"));
        assert!(
            reverify_completion(root.path(), &completion).is_err(),
            "tampering must not replay"
        );
    }
}

#[test]
fn a_replay_refuses_a_sealed_record_that_was_altered_after_the_fact() {
    let root = prepare_root();
    let good = sealed(root.path());
    let mut metrics = good.clone();
    metrics.metrics.average_fps += 1.0;
    assert!(reverify_completion(root.path(), &metrics).is_err());
    let mut context = good.clone();
    context.context.as_mut().unwrap().frame_count += 1;
    assert!(reverify_completion(root.path(), &context).is_err());
    let mut traversal = good.clone();
    traversal.result_directory = "../run".into();
    assert!(reverify_completion(root.path(), &traversal).is_err());
    let mut hash = good;
    hash.files[0].sha256 = "0".repeat(64);
    assert!(reverify_completion(root.path(), &hash).is_err());
}
