use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::json;

use super::{
    tests::{
        BOOT, EXE_BYTES, PEER, RENDERER, accuracy, claim_with, exe_sha, feed, hello, proc_tree,
        raw_pairs, source, unix_now,
    },
    *,
};
use crate::telemetry::quality::QualityDenial;

#[test]
fn a_claim_is_recomputed_from_raw_pairs_and_cannot_understate_the_measured_error() {
    // Errors of 0.3, -1.2 and 0.5 ms among otherwise exact pairs: the bound is the worst absolute one.
    let mut raw = raw_pairs(MIN_ACCURACY_SAMPLES, 0);
    // Three more events after the last reference, 0.3 ms late, 1.2 ms early and 0.5 ms late.
    let last = 10_000_000 + (MIN_ACCURACY_SAMPLES as u64 - 1) * 16_600_000;
    for (step, offset) in [(1_u64, 300_000_i64), (2, -1_200_000), (3, 500_000)] {
        let reference = last + step * 16_600_000;
        raw.push_str(&format!("{} {reference}\n", reference as i64 + offset));
    }
    let claim = StampAccuracy::from_raw_pairs(
        ReferenceKind::HighSpeedCamera,
        &exe_sha(),
        "present",
        BOOT,
        unix_now() + 60,
        unix_now(),
        &raw,
    )
    .unwrap();
    assert_eq!(claim.max_error_ns(), 1_200_000);
    assert_eq!(claim.samples(), MIN_ACCURACY_SAMPLES + 3);
    assert_eq!(claim.reference(), ReferenceKind::HighSpeedCamera);
    assert_eq!(
        claim.evidence_sha256(),
        stutter_core::hash::sha256_hex(raw.as_bytes())
    );
    // The bias is recorded, not subtracted: an unmodelled offset is error.
    assert!(claim.bias_ns().abs() < 1_000);
    // Changing one pair changes the digest, so a claim names the exact raw evidence it came from.
    let altered = raw.replacen("10000000 10000000", "10000001 10000000", 1);
    assert_ne!(altered, raw, "the altered row must exist");
    let other = StampAccuracy::from_raw_pairs(
        ReferenceKind::HighSpeedCamera,
        &exe_sha(),
        "present",
        BOOT,
        unix_now() + 60,
        unix_now(),
        &altered,
    )
    .unwrap();
    assert_ne!(other.evidence_sha256(), claim.evidence_sha256());
}

#[test]
fn a_claim_with_too_few_malformed_or_expired_evidence_is_refused() {
    let far = unix_now() + 86_400;
    let build = |raw: &str, valid_until: u64| {
        StampAccuracy::from_raw_pairs(
            ReferenceKind::Photodiode,
            &exe_sha(),
            "present",
            BOOT,
            valid_until,
            unix_now(),
            raw,
        )
    };
    assert_eq!(
        claim_with(
            MIN_ACCURACY_SAMPLES - 1,
            100,
            &exe_sha(),
            "present",
            BOOT,
            far
        ),
        Err(AccuracyRefusal::TooFewSamples)
    );
    assert_eq!(build("", far), Err(AccuracyRefusal::TooFewSamples));
    for bad in ["1 2 3\n", "x y\n", "5\n", "-1 2\n", "1.5 2\n"] {
        let raw = format!("{}{bad}", raw_pairs(MIN_ACCURACY_SAMPLES, 10));
        assert_eq!(build(&raw, far), Err(AccuracyRefusal::Malformed), "{bad:?}");
    }
    assert_eq!(
        claim_with(
            MIN_ACCURACY_SAMPLES,
            100,
            &exe_sha(),
            "present",
            BOOT,
            unix_now()
        ),
        Err(AccuracyRefusal::Expired)
    );
    let huge = raw_pairs(MAX_ACCURACY_SAMPLES + 1, 5);
    assert_eq!(build(&huge, far), Err(AccuracyRefusal::TooManySamples));
}

#[test]
fn a_claim_is_accepted_only_for_the_bound_renderer_event_and_boot() {
    let root = proc_tree("claim-binding");
    let far = unix_now() + 86_400;
    let mut source = source(&root);
    // Nothing bound: nothing for a claim to describe.
    assert_eq!(
        source.set_stamp_accuracy(accuracy(100), unix_now()),
        Err(AccuracyRefusal::NotBound)
    );
    source.bind(PEER, &hello(1, 0)).unwrap();
    let refused = |source: &mut StampedSource, claim: Result<StampAccuracy, AccuracyRefusal>| {
        source.set_stamp_accuracy(claim.unwrap(), unix_now())
    };
    let other_exe = stutter_core::hash::sha256_hex(b"some other binary");
    assert_eq!(
        refused(
            &mut source,
            claim_with(1000, 100, &other_exe, "present", BOOT, far)
        ),
        Err(AccuracyRefusal::WrongRenderer)
    );
    assert_eq!(
        refused(
            &mut source,
            claim_with(1000, 100, &exe_sha(), "swapchain", BOOT, far)
        ),
        Err(AccuracyRefusal::WrongEvent)
    );
    assert_eq!(
        refused(
            &mut source,
            claim_with(1000, 100, &exe_sha(), "present", "another-boot", far)
        ),
        Err(AccuracyRefusal::WrongBoot)
    );
    // The window ended between measuring and applying.
    assert_eq!(
        source.set_stamp_accuracy(accuracy(100), far + 1),
        Err(AccuracyRefusal::Expired)
    );
    // The renderer binary was rewritten after binding: the source is retired outright, so a claim
    // for it cannot be applied at all (stronger than refusing the claim as the wrong renderer).
    fs::write(root.join(format!("{RENDERER}/exe")), b"rebuilt renderer").unwrap();
    assert_eq!(
        source.set_stamp_accuracy(accuracy(100), unix_now()),
        Err(AccuracyRefusal::NotBound)
    );
    assert!(!source.is_bound());
    // With the original binary back and a fresh bind under a new epoch, a matching claim is accepted.
    fs::write(root.join(format!("{RENDERER}/exe")), EXE_BYTES).unwrap();
    source.bind(PEER, &hello(2, 0)).unwrap();
    source
        .set_stamp_accuracy(accuracy(100), unix_now())
        .unwrap();
}

#[test]
fn an_accuracy_claim_that_expires_while_in_use_stops_counting_at_the_next_decision() {
    let root = proc_tree("claim-expiry");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 1500, 30_000);
    let short = claim_with(1000, 50_000, &exe_sha(), "present", BOOT, unix_now() + 1).unwrap();
    source.set_stamp_accuracy(short, unix_now()).unwrap();
    assert_eq!(source.quality_at(now).frame_primary_eligible(), Ok(()));
    std::thread::sleep(std::time::Duration::from_millis(2_100));
    let quality = source.quality_at(now);
    assert!(quality.alignment_uncertainty_ms.is_nan(), "{quality:?}");
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
}

#[test]
fn duplicated_replayed_or_reordered_reference_rows_cannot_pad_a_measurement() {
    let far = unix_now() + 86_400;
    let build = |raw: &str| {
        StampAccuracy::from_raw_pairs(
            ReferenceKind::Photodiode,
            &exe_sha(),
            "present",
            BOOT,
            far,
            unix_now(),
            raw,
        )
    };
    let good = raw_pairs(MIN_ACCURACY_SAMPLES, 10);
    assert!(build(&good).is_ok());
    // One row repeated a thousand times is not a thousand measurements.
    let row = "5000000 4999990\n";
    assert_eq!(
        build(&row.repeat(MIN_ACCURACY_SAMPLES)),
        Err(AccuracyRefusal::NotMonotonic)
    );
    // A duplicated row, a repeated stamp or reference, and a reordered pair.
    let mut lines: Vec<&str> = good.lines().collect();
    lines.insert(500, lines[499]);
    assert_eq!(build(&lines.join("\n")), Err(AccuracyRefusal::NotMonotonic));
    let mut lines: Vec<&str> = good.lines().collect();
    lines.swap(10, 11);
    assert_eq!(build(&lines.join("\n")), Err(AccuracyRefusal::NotMonotonic));
    let same_reference = format!("{good}{} {}\n", u64::MAX / 2, 10_000_000);
    assert_eq!(build(&same_reference), Err(AccuracyRefusal::NotMonotonic));
}

#[test]
fn only_a_capture_written_where_the_trusted_collector_alone_can_write_confers_a_claim() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = collector_tree("trust");
    let far = unix_now() + 86_400;
    let pairs = raw_pairs(MIN_ACCURACY_SAMPLES, 40_000);
    let path = write_capture(&root, &capture_json(&pairs, "photodiode", far));
    let load = |path: &Path, uid: u32| {
        StampAccuracy::from_trusted_capture_with(path, unix_now(), &root, uid)
    };
    // The collector's own capture is accepted, and is the same claim the raw pairs give.
    let claim = load(&path, me()).unwrap();
    assert_eq!(claim.max_error_ns(), 40_000);
    assert_eq!(claim.reference(), ReferenceKind::Photodiode);
    assert_eq!(
        claim.evidence_sha256(),
        stutter_core::hash::sha256_hex(pairs.as_bytes())
    );

    // Written by someone else, or not trusted to be: refused.
    assert_eq!(
        load(&path, me().wrapping_add(1)),
        Err(AccuracyRefusal::UntrustedCapture)
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();
    assert_eq!(load(&path, me()), Err(AccuracyRefusal::UntrustedCapture));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(root.join("captures"), fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(load(&path, me()), Err(AccuracyRefusal::UntrustedCapture));
    fs::set_permissions(root.join("captures"), fs::Permissions::from_mode(0o755)).unwrap();
    let link = root.join("captures/link.json");
    symlink(&path, &link).unwrap();
    assert_eq!(load(&link, me()), Err(AccuracyRefusal::UntrustedCapture));
    // The production entry point trusts root only, so a caller's own file is never enough.
    if me() != 0 {
        assert_eq!(
            StampAccuracy::from_trusted_capture(&path, unix_now()),
            Err(AccuracyRefusal::UntrustedCapture)
        );
    }
}

#[test]
fn a_trusted_capture_must_still_be_well_formed_fresh_and_a_real_event_series() {
    let root = collector_tree("content");
    let far = unix_now() + 86_400;
    let good = raw_pairs(MIN_ACCURACY_SAMPLES, 10);
    let load = |body: &str| {
        let path = write_capture(&root, body);
        StampAccuracy::from_trusted_capture_with(&path, unix_now(), &root, me())
    };
    assert!(load(&capture_json(&good, "display_hardware_timestamp", far)).is_ok());
    assert_eq!(load("not json"), Err(AccuracyRefusal::MalformedCapture));
    assert_eq!(
        load(&capture_json(&good, "self_reported", far)),
        Err(AccuracyRefusal::MalformedCapture)
    );
    let wrong_version =
        capture_json(&good, "photodiode", far).replace("\"version\":1", "\"version\":2");
    assert_eq!(load(&wrong_version), Err(AccuracyRefusal::MalformedCapture));
    let extra = capture_json(&good, "photodiode", far)
        .replace("\"version\":1", "\"version\":1,\"trusted\":true");
    assert_eq!(
        load(&extra),
        Err(AccuracyRefusal::MalformedCapture),
        "unknown fields are refused"
    );
    assert_eq!(
        load(&capture_json(&good, "photodiode", unix_now())),
        Err(AccuracyRefusal::Expired)
    );
    assert_eq!(
        load(&capture_json(
            &"1 1\n".repeat(MIN_ACCURACY_SAMPLES),
            "photodiode",
            far
        )),
        Err(AccuracyRefusal::NotMonotonic)
    );
    assert_eq!(
        load(&capture_json("5 4\n", "photodiode", far)),
        Err(AccuracyRefusal::TooFewSamples)
    );
    let missing = StampAccuracy::from_trusted_capture_with(
        &root.join("captures/absent.json"),
        unix_now(),
        &root,
        me(),
    );
    assert_eq!(missing, Err(AccuracyRefusal::UnreadableCapture));
}

#[test]
fn a_claim_from_a_trusted_capture_is_still_bound_to_the_renderer_event_and_boot_it_names() {
    let root = proc_tree("capture-binding");
    let tree = collector_tree("capture-bound");
    let far = unix_now() + 86_400;
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let pairs = raw_pairs(MIN_ACCURACY_SAMPLES, 25_000);
    let path = write_capture(&tree, &capture_json(&pairs, "high_speed_camera", far));
    let claim = StampAccuracy::from_trusted_capture_with(&path, unix_now(), &tree, me()).unwrap();
    source.set_stamp_accuracy(claim, unix_now()).unwrap();
    // A capture measured for another boot is refused when applied.
    let other_boot = capture_json(&pairs, "high_speed_camera", far).replace(BOOT, "other-boot");
    let path = write_capture(&tree, &other_boot);
    let claim = StampAccuracy::from_trusted_capture_with(&path, unix_now(), &tree, me()).unwrap();
    assert_eq!(
        source.set_stamp_accuracy(claim, unix_now()),
        Err(AccuracyRefusal::WrongBoot)
    );
}

fn capture_json(pairs: &str, reference: &str, valid_until: u64) -> String {
    json!({
        "version": 1,
        "reference": reference,
        "renderer_exe_sha256": exe_sha(),
        "event": "present",
        "boot_id": BOOT,
        "valid_until_unix": valid_until,
        "pairs": pairs,
    })
    .to_string()
}

/// A tree standing in for the root-owned collector output: owned by the test user, 0755.
fn collector_tree(name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("stutter-capture-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("captures")).unwrap();
    for dir in [root.clone(), root.join("captures")] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).unwrap();
    }
    root
}

fn write_capture(root: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join("captures/run.json");
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    path
}

fn me() -> u32 {
    crate::syscall::geteuid()
}
