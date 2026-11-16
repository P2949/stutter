use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::autotune::research::{
    cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkFile, NativeBenchmarkMetrics,
    },
    report::AttemptEvidenceV1,
};

/// Seal an evidence record the way the campaign directory holds it (private, canonical name).
fn write_evidence(dir: &std::fs::File, item: &AttemptEvidenceV1) {
    use std::os::unix::fs::OpenOptionsExt;
    let path = format!(
        "/proc/self/fd/{}/{:04}.json",
        std::os::fd::AsRawFd::as_raw_fd(dir),
        item.slot_index
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    std::io::Write::write_all(&mut file, &serde_json::to_vec(item).unwrap()).unwrap();
}

fn completion(dir: &std::path::Path, uid: u32, fps: f64) -> CyberpunkBenchmarkCompletion {
    // A real result folder: summary and frame file agree.
    let run = dir.join("run");
    std::fs::create_dir(&run).unwrap();
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).unwrap();
    let summary = format!(
        r#"{{"RootType":"worldBenchmarkSummary","Data":{{"averageFps":{fps},"frameNumber":20,"gameVersion":"2.5","presetName":"P","gpuName":"G","upscalingType":2,"rayTracingEnabled":true,"frameGenerationType":0}}}}"#
    );
    let mut csv = String::from("Frame index, Frame time (ms)\n");
    for index in 0..20 {
        csv.push_str(&format!("{index}, {:.4}\n", 1000.0 / fps));
    }
    std::fs::write(run.join("summary.json"), &summary).unwrap();
    std::fs::write(run.join("frames.csv"), &csv).unwrap();
    let context = crate::autotune::research::native_context::verify_frames(
        summary.as_bytes(),
        csv.as_bytes(),
    )
    .unwrap();
    let file = |name: &str, bytes: &[u8]| NativeBenchmarkFile {
        path: format!("run/{name}"),
        size_bytes: bytes.len() as u64,
        sha256: stutter_core::hash::sha256_hex(bytes),
    };
    CyberpunkBenchmarkCompletion {
        schema_version: 1,
        app_id: 1_091_500,
        build_id: 42,
        scene: CyberpunkScenario::BuiltinBenchmark,
        settings_sha256: "c".repeat(64),
        proton_source: ProtonSource::Ge,
        proton_identity_sha256: "d".repeat(64),
        process: CyberpunkProcessIdentity {
            pid: stutter_core::ids::Pid::new(1234),
            start_ticks: 1,
            uid,
            app_id: 1_091_500,
            build_id: 42,
            executable_sha256: "b".repeat(64),
        },
        started_unix_nanos: 1,
        completed_unix_nanos: 2,
        result_directory: "run".into(),
        metrics: NativeBenchmarkMetrics {
            average_fps: fps,
            minimum_fps: None,
            maximum_fps: None,
        },
        context: Some(context),
        loaded_runtime: None,
        runtime_binding: None,
        files: vec![
            file("frames.csv", csv.as_bytes()),
            file("summary.json", summary.as_bytes()),
        ],
    }
}

fn evidence_dir(root: &std::path::Path) -> std::fs::File {
    let dir = root.join("evidence");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::File::open(dir).unwrap()
}

#[test]
fn raw_files_replay_and_any_tampering_is_reported_per_attempt() {
    let uid = crate::syscall::geteuid();
    let root = tempfile::tempdir().unwrap();
    let results = root.path().join("results");
    std::fs::create_dir(&results).unwrap();
    std::fs::set_permissions(&results, std::fs::Permissions::from_mode(0o700)).unwrap();
    let sealed = completion(&results, uid, 60.0);
    let evidence = evidence_dir(root.path());
    write_evidence(
        &evidence,
        &AttemptEvidenceV1 {
            schema_version: 1,
            slot_index: 0,
            average_fps: 60.0,
            native_completion: Some(Box::new(sealed)),
        },
    );
    write_evidence(
        &evidence,
        &AttemptEvidenceV1 {
            schema_version: 1,
            slot_index: 1,
            average_fps: 50.0,
            native_completion: None,
        },
    );
    let lookup = |_: ProtonSource| results.clone();
    let clean = verify_raw_evidence(&evidence, uid, &lookup).unwrap();
    assert!(clean.passed());
    assert_eq!((clean.verified, clean.without_native_result), (1, 1));
    // Tamper with the frame file: the same record no longer replays.
    std::fs::write(
        results.join("run/frames.csv"),
        b"Frame index, Frame time (ms)\n0, 9.0\n",
    )
    .unwrap();
    let tampered = verify_raw_evidence(&evidence, uid, &lookup).unwrap();
    assert!(!tampered.passed());
    assert_eq!(tampered.problems.len(), 1);
    assert_eq!(tampered.problems[0].slot_index, 0);
    // A results directory that is gone is a problem, never a pass.
    let gone = verify_raw_evidence(&evidence, uid, &|_| root.path().join("absent")).unwrap();
    assert!(!gone.passed());
}

#[test]
fn an_attempt_whose_fps_disagrees_with_its_sealed_native_metrics_does_not_replay() {
    let uid = crate::syscall::geteuid();
    let root = tempfile::tempdir().unwrap();
    let results = root.path().join("results");
    std::fs::create_dir(&results).unwrap();
    std::fs::set_permissions(&results, std::fs::Permissions::from_mode(0o700)).unwrap();
    let sealed = completion(&results, uid, 60.0);
    let evidence = evidence_dir(root.path());
    write_evidence(
        &evidence,
        &AttemptEvidenceV1 {
            schema_version: 1,
            slot_index: 0,
            average_fps: 75.0,
            native_completion: Some(Box::new(sealed)),
        },
    );
    let outcome = verify_raw_evidence(&evidence, uid, &|_| results.clone()).unwrap();
    assert!(!outcome.passed());
}

/// Read-only replay of a real, preserved campaign:
/// `STUTTER_REAL_CAMPAIGN=<campaign dir> STUTTER_REAL_RESULTS=<benchmarkResults dir> cargo test
/// real_campaign_replay -- --ignored --nocapture`. It never writes.
#[test]
#[ignore = "needs a preserved real campaign directory"]
fn real_campaign_replay() {
    use std::os::fd::AsFd;
    let (Some(campaign), Some(results)) = (
        std::env::var_os("STUTTER_REAL_CAMPAIGN"),
        std::env::var_os("STUTTER_REAL_RESULTS"),
    ) else {
        panic!("set STUTTER_REAL_CAMPAIGN and STUTTER_REAL_RESULTS");
    };
    let uid = crate::syscall::geteuid();
    let dir = std::fs::File::from(
        crate::syscall::open_dir_no_symlinks(std::path::Path::new(&campaign)).unwrap(),
    );
    let evidence =
        std::fs::File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence").unwrap());
    let results = std::path::PathBuf::from(results);
    let outcome = verify_raw_evidence(&evidence, uid, &|_| results.clone()).unwrap();
    println!("{}", serde_json::to_string_pretty(&outcome).unwrap());
    assert!(outcome.passed(), "{outcome:?}");
}
