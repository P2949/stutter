use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use super::{
    frames::{metrics, read_frametimes},
    health::fault_lines,
    manifest::*,
    outcome::Outcome,
    run::{Candidate, execution_argv, parse_candidate, run},
};
use crate::evidence_bundle::sha256::sha256_hex;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-rw-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn frame_logs_are_trimmed_by_warmup_and_damaged_data_is_refused() {
    let csv =
        "os,cpu\nlinux,x\nfps,frametime,elapsed\n60,10,1000000\n60,12,5000000\n60,14,9000000\n";
    // 5 ms of warmup drops the first frame by elapsed time.
    assert_eq!(read_frametimes(csv, 5.0).unwrap(), vec![12.0, 14.0]);
    assert_eq!(metrics(csv, 0.0).unwrap().frames, 3);
    assert!(
        read_frametimes("fps,frametime\n60,abc\n", 0.0).is_none(),
        "malformed row"
    );
    assert!(
        read_frametimes("fps,frametime\n60,-1\n", 0.0).is_none(),
        "non-positive frametime"
    );
    assert!(read_frametimes("a,b\n1,2\n", 0.0).is_none(), "no header");
    assert!(
        read_frametimes("fps,frametime\n", 0.0).is_none(),
        "no frames"
    );
    // Without an elapsed column the cumulative frametime stands in.
    assert_eq!(
        read_frametimes("frametime\n10\n10\n10\n", 15.0).unwrap(),
        vec![10.0, 10.0]
    );
}

#[test]
fn only_a_known_unprivileged_treatment_is_parsed_and_arms_differ_only_by_it() {
    assert_eq!(
        parse_candidate("taskset:2-3,6"),
        Some(Candidate::Taskset("2-3,6".into()))
    );
    for bad in ["taskset:", "taskset:1;rm", "nice:5", "cpu_affinity_profile"] {
        assert_eq!(parse_candidate(bad), None, "{bad}");
    }
    let manifest = tiny_manifest(
        "a".repeat(64),
        "taskset:0",
        vec!["game".into(), "--x".into()],
    );
    let candidate = Candidate::Taskset("0".into());
    let base = execution_argv(
        Path::new("/s"),
        Path::new("/c"),
        &manifest,
        Arm::Baseline,
        &candidate,
    );
    let treat = execution_argv(
        Path::new("/s"),
        Path::new("/c"),
        &manifest,
        Arm::Treatment,
        &candidate,
    );
    assert!(base.ends_with(&["--".to_owned(), "game".into(), "--x".into()]));
    assert!(treat.ends_with(&[
        "taskset".to_owned(),
        "-c".into(),
        "0".into(),
        "game".into(),
        "--x".into()
    ]));
    assert!(base.contains(&"--mangohud".to_owned()) && treat.contains(&"--mangohud".to_owned()));
}

#[test]
fn only_real_gpu_faults_in_the_kernel_log_count() {
    let log = "amdgpu 0000:03:00.0: amdgpu: ring gfx_0.0.0 timeout, signaled seq=1\nusb 1-1: new device\n[drm] GPU reset begin!\nsystem is during a string of events";
    assert_eq!(fault_lines(log).len(), 2);
    assert!(fault_lines("nothing wrong here").is_empty());
}

fn tiny_manifest(hash: String, candidate: &str, command: Vec<String>) -> Manifest {
    let metric = |name: &str| Metric {
        name: name.into(),
        unit: "ms".into(),
        direction: Direction::LowerIsBetter,
    };
    Manifest {
        schema_version: 1,
        evidence_kind: EvidenceKind::ControlledReal,
        source_commit: "a".repeat(40),
        binary_sha256: hash,
        workload: "fake".into(),
        launch_mode: LaunchMode::Native,
        work_unit: "50 frames".into(),
        command,
        time_boxed: false,
        instrumentation: vec!["mangohud".into()],
        context: Default::default(),
        warmup_seconds: 0,
        measure_seconds: 1,
        cooldown_seconds: 0,
        timeout_seconds: 30,
        max_pairs: 4,
        candidate: candidate.into(),
        primary: metric("frame_p99_ms"),
        min_effect_percent: 10.0,
        equivalence_margin_percent: 3.0,
        guardrails: vec![Guardrail {
            metric: metric("frame_mean_ms"),
            max_regression_percent: 5.0,
        }],
        abort_rules: vec!["amdgpu reset".into()],
    }
}

/// A stand-in for `stutter game-run` that makes a capture directory and runs the command.
fn fake_stutter(dir: &Path) -> PathBuf {
    let path = dir.join("fake-stutter");
    fs::write(
        &path,
        "#!/bin/sh\nroot=$3\nshift 5\nd=\"$root/$(date +%s%N)\"\nmkdir -p \"$d\"\nchmod 700 \"$d\"\nSTUTTER_CAPTURE_DIR=\"$d\" exec \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A workload whose frametime really depends on the CPU affinity it was started with.
const WORKLOAD: &str = "cpus=$(awk '/Cpus_allowed_list/{print $2}' /proc/self/status); if [ \"$cpus\" = \"0\" ]; then ft=8; else ft=10; fi; { echo fps,frametime,elapsed; i=0; while [ $i -lt 50 ]; do i=$((i+1)); echo \"100,$ft,$((i*10000000))\"; done; } > \"$STUTTER_CAPTURE_DIR/log.csv\"";

fn manifest_file(dir: &Path, manifest: &Manifest) -> PathBuf {
    let path = dir.join("manifest.json");
    fs::write(&path, serde_json::to_string(manifest).unwrap()).unwrap();
    path
}

#[test]
fn the_engine_runs_counterbalanced_arms_and_the_treatment_really_changes_the_workload() {
    let dir = scratch("engine");
    let stutter = fake_stutter(&dir);
    let hash = sha256_hex(&fs::read(&stutter).unwrap());
    let manifest = tiny_manifest(
        hash,
        "taskset:0",
        vec!["sh".into(), "-c".into(), WORKLOAD.into()],
    );
    let manifest_path = manifest_file(&dir, &manifest);
    let out = dir.join("out");
    let report = run(&manifest_path, &out, &stutter, false).unwrap();
    assert_eq!(report.executions, 8);
    assert_eq!(
        report.verdict.outcome,
        Outcome::PerformancePass,
        "{:?}",
        report.verdict
    );
    assert!((report.verdict.effect_percent - 20.0).abs() < 1e-6);
    assert!(report.verdict.engineering_pass);
    // Resuming repeats nothing and gives the same verdict.
    let again = run(&manifest_path, &out, &stutter, false).unwrap();
    assert_eq!(again.verdict, report.verdict);
    // A changed manifest in the same output is a different study and is refused.
    let mut changed = manifest.clone();
    changed.min_effect_percent = 11.0;
    let changed_path = manifest_file(&dir.join("elsewhere-manifest").tap_create(), &changed);
    assert!(run(&changed_path, &out, &stutter, false).is_err());
    // The frozen binary hash must match the binary actually used.
    let mut wrong = manifest;
    wrong.binary_sha256 = "b".repeat(64);
    let wrong_path = manifest_file(&dir.join("wrong").tap_create(), &wrong);
    assert!(run(&wrong_path, &dir.join("out2"), &stutter, false).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unsupported_treatment_is_a_blocked_cohort_not_a_guess() {
    let dir = scratch("blocked");
    let stutter = fake_stutter(&dir);
    let hash = sha256_hex(&fs::read(&stutter).unwrap());
    let manifest = tiny_manifest(hash, "cpu_affinity_profile", vec!["true".into()]);
    let report = run(
        &manifest_file(&dir, &manifest),
        &dir.join("out"),
        &stutter,
        false,
    )
    .unwrap();
    assert_eq!(report.verdict.outcome, Outcome::BlockedResource);
    assert_eq!(report.executions, 0);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_workload_that_writes_no_log_is_inconclusive_with_a_damaged_engineering_path() {
    let dir = scratch("nolog");
    let stutter = fake_stutter(&dir);
    let hash = sha256_hex(&fs::read(&stutter).unwrap());
    let manifest = tiny_manifest(hash, "taskset:0", vec!["true".into()]);
    let report = run(
        &manifest_file(&dir, &manifest),
        &dir.join("out"),
        &stutter,
        false,
    )
    .unwrap();
    assert_eq!(report.verdict.outcome, Outcome::Inconclusive);
    assert!(!report.verdict.engineering_pass);
    fs::remove_dir_all(&dir).unwrap();
}

trait TapCreate {
    fn tap_create(self) -> PathBuf;
}

impl TapCreate for PathBuf {
    fn tap_create(self) -> PathBuf {
        fs::create_dir_all(&self).unwrap();
        self
    }
}
