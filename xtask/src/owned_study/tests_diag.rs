//! E13/E14 support: the output-hash rule and the observation-only diagnostics.
use std::{cell::Cell, fs};

use super::{
    diagnostics::{parse_perf_csv, parse_psi, summarize, wrap_with_perf, write_slot_diag},
    host::HostState,
    journal::{self, Mode},
    run::{Completed, SlotRunner, hash_output, run_with},
    tests::{manifest, schedule_records, scratch},
};

const CTX: &str = "ctx-1";

struct Hashing {
    ran: Cell<usize>,
    hash: Box<dyn Fn(usize) -> Option<String>>,
}

impl SlotRunner for Hashing {
    fn prepare(&self, _: &super::manifest::Slot) -> anyhow::Result<()> {
        Ok(())
    }
    fn run_slot(&self, _: &super::manifest::Slot) -> anyhow::Result<Result<Completed, String>> {
        let n = self.ran.get();
        self.ran.set(n + 1);
        Ok(Ok(Completed {
            elapsed_ns: 10,
            output_sha256: (self.hash)(n),
        }))
    }
}

fn safe() -> HostState {
    HostState::Safe
}

#[test]
fn the_output_artifact_must_be_a_plain_relative_path() {
    let dir = scratch("diag-path");
    let mut m = manifest(&dir);
    m.output_artifact = Some("release/libx.rlib".into());
    m.validate(&super::tests::allowed()).expect("plain path");
    for bad in [
        "",
        "/etc/passwd",
        "../x",
        "a/../b",
        "a//b",
        "a/./b",
        "has space",
        "x\ny",
    ] {
        m.output_artifact = Some(bad.into());
        assert!(m.validate(&super::tests::allowed()).is_err(), "{bad:?}");
    }
    m.output_artifact = None;
    let digest_without = m.digest();
    m.diagnostics = false;
    assert_eq!(
        m.digest(),
        digest_without,
        "unused options never change a manifest digest (old studies still verify)"
    );
    m.diagnostics = true;
    assert_ne!(m.digest(), digest_without);
}

#[test]
fn every_arm_must_build_the_same_output_or_the_slot_is_invalid() {
    let dir = scratch("diag-hash");
    let mut m = manifest(&dir);
    m.output_artifact = Some("release/libx.rlib".into());
    let runner = Hashing {
        ran: Cell::new(0),
        hash: Box::new(|n| {
            Some(if n == 7 {
                "other".into()
            } else {
                "same".into()
            })
        }),
    };
    run_with(&m, &dir, CTX, &safe, &runner).expect("run");
    let records = journal::load(&dir, &m, &Mode::Resume(CTX.into()), false)
        .expect("verified")
        .records;
    let bad: Vec<_> = records.iter().filter(|r| !r.valid).collect();
    assert_eq!(bad.len(), 1);
    assert!(
        bad[0]
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("differs"))
    );
    assert_eq!(
        bad[0].output_sha256.as_deref(),
        Some("other"),
        "the divergent hash is kept as evidence"
    );
    assert!(
        records
            .iter()
            .filter(|r| r.valid)
            .all(|r| r.output_sha256.as_deref() == Some("same"))
    );
}

#[test]
fn a_missing_hash_is_invalid_and_a_hash_without_an_artifact_is_refused() {
    let dir = scratch("diag-nohash");
    let mut m = manifest(&dir);
    m.output_artifact = Some("release/libx.rlib".into());
    let runner = Hashing {
        ran: Cell::new(0),
        hash: Box::new(|_| None),
    };
    run_with(&m, &dir, CTX, &safe, &runner).expect("run");
    let records = journal::load(&dir, &m, &Mode::Resume(CTX.into()), false)
        .expect("verified")
        .records;
    assert!(records.iter().all(|r| !r.valid));
    let plain = manifest(&scratch("diag-plain"));
    let mut forged = schedule_records(&plain, Some(CTX), |_, _| Some(5));
    forged[0].output_sha256 = Some("x".into());
    assert!(journal::verify_records(&plain, forged, &Mode::Analyze).is_err());
    let mut mixed = schedule_records(&m, Some(CTX), |_, _| Some(5));
    for r in &mut mixed[..2] {
        r.output_sha256 = Some("a".into());
    }
    mixed[1].output_sha256 = Some("b".into());
    assert!(
        journal::verify_records(&m, mixed[..2].to_vec(), &Mode::Analyze).is_err(),
        "two outputs among valid slots"
    );
}

#[test]
fn the_output_hash_reads_only_a_regular_file_inside_the_owned_target() {
    let dir = scratch("diag-hashfile");
    let target = dir.join("target");
    fs::create_dir_all(target.join("release")).expect("dirs");
    fs::write(target.join("release/lib.rlib"), "bits").expect("file");
    let first = hash_output(&target, "release/lib.rlib").expect("hash");
    assert_eq!(
        first,
        hash_output(&target, "release/lib.rlib").expect("stable")
    );
    assert!(hash_output(&target, "release/missing.rlib").is_err());
    assert!(
        hash_output(&target, "release").is_err(),
        "a directory is not an artifact"
    );
    fs::write(dir.join("outside"), "secret").expect("outside");
    std::os::unix::fs::symlink(dir.join("outside"), target.join("release/link")).expect("link");
    assert!(
        hash_output(&target, "release/link").is_err(),
        "a symlink out of the target is refused"
    );
}

#[test]
fn psi_and_perf_files_are_parsed_and_unavailable_counters_are_never_zero() {
    assert_eq!(
        parse_psi(
            "some avg10=1.08 avg60=6.39 avg300=5.41 total=3710180351\nfull avg10=0.00 total=0\n"
        ),
        Some(3_710_180_351)
    );
    assert_eq!(parse_psi("garbage"), None);
    let csv = "# started on x\n941.5,msec,task-clock:u,941958,100.00,,\n12,,context-switches:u,941958,100.00,,\n<not counted>,,cycles:u,0,0.00,,\n<not supported>,,instructions:u,0,0.00,,\n";
    let parsed = parse_perf_csv(csv);
    assert_eq!(parsed["task-clock:u"].0, Some(941.5));
    assert_eq!(parsed["cycles:u"].0, None, "not counted stays absent");
    assert_eq!(parsed["instructions:u"].0, None);
    let argv = wrap_with_perf(
        vec!["xtask".into(), "fixed-work".into()],
        std::path::Path::new("/x/y.csv"),
    );
    assert_eq!(&argv[..4], &["perf", "stat", "-x,", "-e"]);
    assert_eq!(&argv[argv.len() - 3..], &["--", "xtask", "fixed-work"]);
}

#[test]
fn a_slot_diag_file_labels_scope_and_reports_unavailable_without_inventing_values() {
    let dir = scratch("diag-write");
    let csv = dir.join("a.perf.csv");
    fs::write(
        &csv,
        "100,msec,task-clock:u,1,100.00,,\n<not counted>,,cycles:u,0,0.00,,\n",
    )
    .expect("csv");
    write_slot_diag(&dir.join("a.diag.json"), &csv, Some(10), Some(2_510)).expect("diag");
    let v: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("a.diag.json")).expect("read")).expect("json");
    assert_eq!(v["psi_cpu_some_delta_us"], 2500);
    assert_eq!(v["perf"]["events"]["cycles:u"], "Unavailable");
    assert!(
        v["perf"]["scope"]
            .as_str()
            .is_some_and(|s| s.contains("user-space"))
    );
    assert!(
        v["psi_scope"]
            .as_str()
            .is_some_and(|s| s.contains("system-wide"))
    );
    write_slot_diag(
        &dir.join("b.diag.json"),
        &dir.join("absent.csv"),
        None,
        Some(5),
    )
    .expect("diag");
    let b: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("b.diag.json")).expect("read")).expect("json");
    assert!(
        b["perf"]["status"]
            .as_str()
            .is_some_and(|s| s.starts_with("Unavailable"))
    );
    assert!(
        b["psi_cpu_some_delta_us"].is_null(),
        "a missing PSI start is not a zero delta"
    );
}

#[test]
fn the_diagnostics_summary_covers_only_valid_slots_and_is_null_when_off() {
    let dir = scratch("diag-summary");
    let mut m = manifest(&dir);
    let records = schedule_records(&m, Some(CTX), |_, _| Some(4_000_000_000));
    assert!(summarize(&m, &dir, &records).is_null());
    m.diagnostics = true;
    for r in records.iter().filter(|r| r.arm == "baseline") {
        let csv = dir.join(format!("{}.perf.csv", r.slot));
        fs::write(
            &csv,
            "8000,msec,task-clock:u,1,100.00,,\n400,,context-switches:u,1,100.00,,\n",
        )
        .expect("csv");
        write_slot_diag(
            &dir.join(format!("{}.diag.json", r.slot)),
            &csv,
            Some(0),
            Some(1_000_000),
        )
        .expect("diag");
    }
    let summary = summarize(&m, &dir, &records);
    let base = &summary["arms"]["baseline"];
    assert_eq!(base["slots_with_diagnostics"], 6);
    assert_eq!(base["means"]["task_clock_ms"], 8000.0);
    assert_eq!(base["means"]["cpu_seconds_per_wall_second"], 2.0);
    assert_eq!(base["means"]["psi_cpu_some_delta_ms"], 1000.0);
    assert!(
        base["means"].get("context_switches").is_none(),
        "an invalid kernel-side counter is never averaged"
    );
    assert!(
        summary["invalid_signals"]
            .as_str()
            .is_some_and(|s| s.contains("InvalidSignal"))
    );
    assert!(
        base["means"].get("ipc_user").is_none(),
        "no counters, no IPC"
    );
    assert_eq!(summary["arms"]["split"]["slots_with_diagnostics"], 0);
    assert!(
        summary["limits"]
            .as_str()
            .is_some_and(|l| l.contains("no runnable-delay"))
    );
}
