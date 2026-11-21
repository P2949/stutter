use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::autotune::research::{
    cyberpunk::{
        CyberpunkBenchmarkCompletion, CyberpunkProcessIdentity, CyberpunkScenario,
        NativeBenchmarkFile, NativeBenchmarkMetrics,
    },
    journal::{AttemptOutcome, AttemptRecord},
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

fn slot(index: u32) -> crate::autotune::research::journal::AttemptSlot {
    crate::autotune::research::journal::AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index,
        arm: ProtonSource::Ge,
    }
}

/// A completed record sealing the evidence's digest (what the driver writes).
fn completed(item: &AttemptEvidenceV1) -> AttemptRecord {
    AttemptRecord {
        slot: slot(item.slot_index),
        outcome: AttemptOutcome::Completed {
            evidence_sha256: item.digest().unwrap(),
        },
    }
}

fn failed(index: u32) -> AttemptRecord {
    AttemptRecord {
        slot: slot(index),
        outcome: AttemptOutcome::Failed {
            code: "GpuReset".into(),
        },
    }
}

fn item(index: u32, fps: f64, native: Option<CyberpunkBenchmarkCompletion>) -> AttemptEvidenceV1 {
    AttemptEvidenceV1 {
        thermal: None,
        long_session: None,
        schema_version: 1,
        slot_index: index,
        average_fps: fps,
        native_completion: native.map(Box::new),
    }
}

fn results_dir(root: &std::path::Path) -> std::path::PathBuf {
    let results = root.join("results");
    std::fs::create_dir(&results).unwrap();
    std::fs::set_permissions(&results, std::fs::Permissions::from_mode(0o700)).unwrap();
    results
}

#[test]
fn raw_files_replay_and_any_tampering_is_reported_per_attempt() {
    let uid = crate::syscall::geteuid();
    let root = tempfile::tempdir().unwrap();
    let results = results_dir(root.path());
    let first = item(0, 60.0, Some(completion(&results, uid, 60.0)));
    let evidence = evidence_dir(root.path());
    write_evidence(&evidence, &first);
    let records = [completed(&first), failed(1)];
    let lookup = |_: ProtonSource| results.clone();
    let clean = verify_raw_evidence(&evidence, uid, &records, &lookup).unwrap();
    assert!(clean.passed(), "{clean:?}");
    assert_eq!(
        (clean.completed, clean.verified, clean.failed_attempts),
        (1, 1, 1)
    );
    // Tamper with the frame file: the same record no longer replays.
    std::fs::write(
        results.join("run/frames.csv"),
        b"Frame index, Frame time (ms)\n0, 9.0\n",
    )
    .unwrap();
    let tampered = verify_raw_evidence(&evidence, uid, &records, &lookup).unwrap();
    assert!(!tampered.passed());
    assert_eq!(tampered.problems.len(), 1);
    assert_eq!(tampered.problems[0].slot_index, 0);
    // A results directory that is gone is a problem, never a pass.
    let gone =
        verify_raw_evidence(&evidence, uid, &records, &|_| root.path().join("absent")).unwrap();
    assert!(!gone.passed());
}

#[test]
fn one_valid_result_cannot_vouch_for_an_incomplete_campaign() {
    let uid = crate::syscall::geteuid();
    let root = tempfile::tempdir().unwrap();
    let results = results_dir(root.path());
    let good = item(0, 60.0, Some(completion(&results, uid, 60.0)));
    let evidence = evidence_dir(root.path());
    write_evidence(&evidence, &good);
    let lookup = |_: ProtonSource| results.clone();
    // Slot 1 is sealed Completed but has no native result at all.
    let bare = item(1, 50.0, None);
    write_evidence(&evidence, &bare);
    let outcome = verify_raw_evidence(
        &evidence,
        uid,
        &[completed(&good), completed(&bare)],
        &lookup,
    )
    .unwrap();
    assert!(!outcome.passed());
    assert_eq!((outcome.completed, outcome.verified), (2, 1));
    assert_eq!(outcome.problems[0].slot_index, 1);
    // A Completed record whose evidence file is missing.
    let missing = verify_raw_evidence(
        &evidence,
        uid,
        &[completed(&good), completed(&item(2, 1.0, None))],
        &lookup,
    )
    .unwrap();
    assert!(!missing.passed());
    // Evidence with no sealed record, and evidence on a failed attempt, are problems.
    let orphan = verify_raw_evidence(&evidence, uid, &[completed(&good)], &lookup).unwrap();
    assert!(!orphan.passed());
    let on_failed =
        verify_raw_evidence(&evidence, uid, &[completed(&good), failed(1)], &lookup).unwrap();
    assert!(!on_failed.passed());
    // No completed attempt at all is not a pass.
    assert!(
        !verify_raw_evidence(&evidence_dir_empty(root.path()), uid, &[failed(0)], &lookup)
            .unwrap()
            .passed()
    );
}

fn evidence_dir_empty(root: &std::path::Path) -> std::fs::File {
    let dir = root.join("evidence-empty");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::File::open(dir).unwrap()
}

#[test]
fn a_record_whose_evidence_digest_or_arm_or_fps_differs_does_not_replay() {
    let uid = crate::syscall::geteuid();
    let root = tempfile::tempdir().unwrap();
    let results = results_dir(root.path());
    let sealed = completion(&results, uid, 60.0);
    let evidence = evidence_dir(root.path());
    let genuine = item(0, 75.0, Some(sealed.clone()));
    write_evidence(&evidence, &genuine);
    let lookup = |_: ProtonSource| results.clone();
    // FPS differs from the sealed native metrics.
    assert!(
        !verify_raw_evidence(&evidence, uid, &[completed(&genuine)], &lookup)
            .unwrap()
            .passed()
    );
    // The journal sealed a different digest than the evidence now has.
    let mut forged = completed(&genuine);
    forged.outcome = AttemptOutcome::Completed {
        evidence_sha256: "0".repeat(64),
    };
    assert!(
        !verify_raw_evidence(&evidence, uid, &[forged], &lookup)
            .unwrap()
            .passed()
    );
    // The native result names another Proton arm than the slot's.
    let root2 = tempfile::tempdir().unwrap();
    let results2 = results_dir(root2.path());
    let mut wrong_arm = completion(&results2, uid, 60.0);
    wrong_arm.proton_source = ProtonSource::CachyOs;
    let evidence2 = evidence_dir(root2.path());
    let other = item(0, 60.0, Some(wrong_arm));
    write_evidence(&evidence2, &other);
    assert!(
        !verify_raw_evidence(&evidence2, uid, &[completed(&other)], &|_| results2.clone())
            .unwrap()
            .passed()
    );
}

/// Read-only replay of a real, preserved campaign:
/// `STUTTER_REAL_CAMPAIGN=<campaign dir> STUTTER_REAL_RESULTS=<benchmarkResults dir> cargo test
/// real_campaign_replay -- --ignored --nocapture`. It never writes.
#[test]
#[ignore = "needs a preserved real campaign directory"]
fn real_campaign_replay() {
    use std::os::fd::AsFd;

    use crate::autotune::research::{
        ResearchIntentV1, driver::build_schedule, journal::ResearchJournal,
        report_store::read_private,
    };
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
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&read_private(&dir, "intent.json", uid).unwrap()).unwrap();
    let journal = ResearchJournal::open(
        crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal").unwrap(),
        uid,
        build_schedule(&intent).unwrap(),
    )
    .unwrap();
    let records = journal.verify().unwrap();
    let evidence =
        std::fs::File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence").unwrap());
    let results = std::path::PathBuf::from(results);
    let outcome = verify_raw_evidence(&evidence, uid, &records, &|_| results.clone()).unwrap();
    println!("{}", serde_json::to_string_pretty(&outcome).unwrap());
    assert!(outcome.passed(), "{outcome:?}");
}
