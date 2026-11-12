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
    std::fs::write(dir.join("summary.json"), summary).unwrap();
    std::fs::write(dir.join("frames.csv"), "frame,time\n1,0\n").unwrap();
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
        fn begin(&mut self) -> anyhow::Result<()> {
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
    probe.begin().unwrap();
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
    probe.begin().unwrap();
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
        fn begin(&mut self) -> anyhow::Result<()> {
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
    probe.begin().unwrap();
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
