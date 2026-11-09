use std::{fs, path::Path};

use super::*;
use crate::telemetry::quality::QualityDenial;

const RENDERER: u32 = 4242;
const OBSERVER: u32 = 100;
const OWNER: u32 = 1000;
const T0: u64 = 1_000_000_000;
const EXE_BYTES: &[u8] = b"instrumented-renderer-build-1";
const BOOT: &str = "boot-id-1";

/// A fake `/proc`: a renderer started at tick 777 and an observer, sharing one time namespace.
fn proc_tree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-stamped-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for pid in [RENDERER, OBSERVER] {
        fs::create_dir_all(root.join(format!("{pid}/ns"))).unwrap();
    }
    set_start(&root, RENDERER, 777);
    set_start(&root, OBSERVER, 5);
    fs::write(root.join("shared-time"), "").unwrap();
    fs::write(root.join(format!("{RENDERER}/exe")), EXE_BYTES).unwrap();
    fs::create_dir_all(root.join("sys/kernel/random")).unwrap();
    fs::write(root.join("sys/kernel/random/boot_id"), format!("{BOOT}\n")).unwrap();
    for pid in [RENDERER, OBSERVER] {
        std::os::unix::fs::symlink(
            root.join("shared-time"),
            root.join(format!("{pid}/ns/time")),
        )
        .unwrap();
    }
    root
}

fn set_start(root: &Path, pid: u32, ticks: u64) {
    fs::write(
        root.join(format!("{pid}/stat")),
        format!("{pid} (app) S 1 {pid} {pid} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0 0\n"),
    )
    .unwrap();
}

fn source(root: &Path) -> StampedSource {
    StampedSource::new(SourcePolicy {
        owner_uid: OWNER,
        renderer_pid: RENDERER,
        renderer_start_ticks: 777,
        observer_pid: OBSERVER,
        proc_root: root.to_path_buf(),
    })
}

const PEER: PeerCredentials = PeerCredentials {
    pid: RENDERER,
    uid: OWNER,
};

fn hello(epoch: u64, first_seq: u64) -> String {
    format!(
        r#"{{"version":1,"epoch":{epoch},"first_seq":{first_seq},"event":"present","clock":"CLOCK_MONOTONIC"}}"#
    )
}

/// Feeds `count` frames from `from`, 4 ms apart, each seen `latency_ns` after its stamp.
fn feed(source: &mut StampedSource, from: u64, count: u64, latency_ns: u64) -> u64 {
    let mut rx = 0;
    for seq in from..from + count {
        let stamp = T0 + seq * 4_000_000;
        rx = stamp + latency_ns;
        source
            .ingest(&format!(r#"{{"seq":{seq},"monotonic_ns":{stamp}}}"#), rx)
            .unwrap();
    }
    rx
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Raw (stamp, reference) pairs in which every stamp is `error_ns` after its reference.
fn raw_pairs(count: usize, error_ns: u64) -> String {
    (0..count)
        .map(|i| {
            let reference = 10_000_000 + i as u64 * 16_600_000;
            format!("{} {reference}\n", reference + error_ns)
        })
        .collect()
}

fn exe_sha() -> String {
    stutter_core::hash::sha256_hex(EXE_BYTES)
}

/// A claim measured for the fixture renderer, its `present` event and this boot, valid for a day.
fn claim_with(
    count: usize,
    error_ns: u64,
    exe: &str,
    event: &str,
    boot: &str,
    valid_until: u64,
) -> Result<StampAccuracy, AccuracyRefusal> {
    StampAccuracy::from_raw_pairs(
        ReferenceKind::Photodiode,
        exe,
        event,
        boot,
        valid_until,
        unix_now(),
        &raw_pairs(count, error_ns),
    )
}

fn accuracy(error_ns: u64) -> StampAccuracy {
    claim_with(
        MIN_ACCURACY_SAMPLES,
        error_ns,
        &exe_sha(),
        "present",
        BOOT,
        unix_now() + 86_400,
    )
    .unwrap()
}

/// Sets a verified claim on a bound source.
fn set_accuracy(source: &mut StampedSource, error_ns: u64) {
    source
        .set_stamp_accuracy(accuracy(error_ns), unix_now())
        .unwrap();
}

#[test]
fn an_authenticated_producer_with_both_bounds_is_eligible_under_the_unchanged_gate() {
    let root = proc_tree("ok");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 1500, 100_000);
    set_accuracy(&mut source, 400_000);
    let quality = source.quality_at(now);
    // 0.1 ms delivery + 0.4 ms stamp error.
    assert!(
        (quality.alignment_uncertainty_ms - 0.5).abs() < 1e-9,
        "{quality:?}"
    );
    assert_eq!(quality.frame_primary_eligible(), Ok(()));
}

#[test]
fn fast_delivery_without_an_independent_stamp_accuracy_is_denied_not_trusted() {
    let root = proc_tree("no-accuracy");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 3000, 30_000);
    let quality = source.quality_at(now);
    assert!(quality.alignment_uncertainty_ms.is_nan());
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
    // A stamp error that takes the sum over the gate denies too.
    set_accuracy(&mut source, 2_500_000);
    assert_eq!(
        source.quality_at(now).frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
}

#[test]
fn a_claim_is_recomputed_from_raw_pairs_and_cannot_understate_the_measured_error() {
    // Errors of 0.3, -1.2 and 0.5 ms among otherwise exact pairs: the bound is the worst absolute one.
    let mut raw = raw_pairs(MIN_ACCURACY_SAMPLES, 0);
    raw.push_str("10300000 10000000\n9000000 10200000\n10500000 10000000\n");
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
    let altered = raw.replacen("10500000 10000000", "10500001 10000000", 1);
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
    // The renderer binary was replaced on disk after the measurement: no longer that renderer.
    fs::write(root.join(format!("{RENDERER}/exe")), b"rebuilt renderer").unwrap();
    assert_eq!(
        source.set_stamp_accuracy(accuracy(100), unix_now()),
        Err(AccuracyRefusal::WrongRenderer)
    );
    // And a matching claim is accepted once everything agrees.
    fs::write(root.join(format!("{RENDERER}/exe")), EXE_BYTES).unwrap();
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
fn the_producer_is_bound_by_user_process_start_time_and_time_namespace() {
    let root = proc_tree("identity");
    let wrong_user = PeerCredentials { uid: 0, ..PEER };
    assert_eq!(
        source(&root).bind(wrong_user, &hello(1, 0)),
        Err(BindDenial::WrongUser)
    );
    let wrong_pid = PeerCredentials { pid: 9999, ..PEER };
    assert_eq!(
        source(&root).bind(wrong_pid, &hello(1, 0)),
        Err(BindDenial::WrongProcess)
    );
    // The pid was reused: alive, but not the process that was recorded.
    set_start(&root, RENDERER, 888);
    assert_eq!(
        source(&root).bind(PEER, &hello(1, 0)),
        Err(BindDenial::IdentityChanged)
    );
    // The process is gone.
    fs::remove_dir_all(root.join(RENDERER.to_string())).unwrap();
    assert_eq!(
        source(&root).bind(PEER, &hello(1, 0)),
        Err(BindDenial::IdentityChanged)
    );

    // A producer in another time namespace.
    let other = proc_tree("namespace");
    fs::write(other.join("other-time"), "").unwrap();
    fs::remove_file(other.join(format!("{RENDERER}/ns/time"))).unwrap();
    std::os::unix::fs::symlink(
        other.join("other-time"),
        other.join(format!("{RENDERER}/ns/time")),
    )
    .unwrap();
    assert_eq!(
        source(&other).bind(PEER, &hello(1, 0)),
        Err(BindDenial::Namespace(
            ClockNamespaceDenial::DifferentNamespace
        ))
    );
}

#[test]
fn a_bad_unsupported_or_replayed_hello_is_refused_and_binds_nothing() {
    let root = proc_tree("hello");
    let mut source = source(&root);
    for bad in [
        "not json".to_owned(),
        hello(1, 0).replace("\"version\":1", "\"version\":2"),
        hello(1, 0).replace("CLOCK_MONOTONIC", "CLOCK_REALTIME"),
    ] {
        assert_eq!(source.bind(PEER, &bad), Err(BindDenial::BadHello), "{bad}");
    }
    assert_eq!(
        source.bind(PEER, &hello(1, 0).replace("present", "queue_submit")),
        Err(BindDenial::UnsupportedEvent)
    );
    assert!(!source.is_bound());
    assert_eq!(source.ingest("{}", 0), Err(IngestError::NotBound));
    // A real hello binds; the same epoch again, or an older one, is a replay.
    source.bind(PEER, &hello(5, 0)).unwrap();
    feed(&mut source, 0, 10, 1_000);
    set_start(&root, RENDERER, 888);
    source.revalidate();
    assert!(!source.is_bound());
    set_start(&root, RENDERER, 777);
    assert_eq!(
        source.bind(PEER, &hello(5, 0)),
        Err(BindDenial::ReplayedEpoch)
    );
    assert_eq!(
        source.bind(PEER, &hello(4, 0)),
        Err(BindDenial::ReplayedEpoch)
    );
    source.bind(PEER, &hello(6, 0)).unwrap();
}

#[test]
fn a_second_producer_while_one_is_alive_is_refused_as_competing() {
    let root = proc_tree("competing");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    feed(&mut source, 0, 1200, 20_000);
    assert_eq!(
        source.bind(PEER, &hello(2, 0)),
        Err(BindDenial::CompetingProducer)
    );
    // The refusal leaves the bound producer's calibration intact.
    assert_eq!(source.stream().accepted(), 1200);
    assert_eq!(source.stream().epoch(), 1);
}

#[test]
fn a_restarted_producer_withdraws_the_source_and_a_new_epoch_must_calibrate_afresh() {
    let root = proc_tree("restart");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 1500, 20_000);
    set_accuracy(&mut source, 100_000);
    assert_eq!(source.quality_at(now).frame_primary_eligible(), Ok(()));
    // The renderer exits (its pid vanishes): the source is withdrawn immediately.
    fs::remove_dir_all(root.join(RENDERER.to_string())).unwrap();
    source.revalidate();
    assert!(!source.is_bound());
    assert_eq!(
        source.quality_at(now).frame_primary_eligible(),
        Err(QualityDenial::NotValidatedDirect)
    );
    assert_eq!(
        source.ingest(r#"{"seq":1500,"monotonic_ns":1}"#, 2),
        Err(IngestError::NotBound)
    );
    // The same renderer comes back (same recorded start time) under a higher epoch: a fresh
    // calibration and a fresh accuracy measurement are both required.
    fs::create_dir_all(root.join(format!("{RENDERER}/ns"))).unwrap();
    fs::write(root.join(format!("{RENDERER}/exe")), EXE_BYTES).unwrap();
    set_start(&root, RENDERER, 777);
    std::os::unix::fs::symlink(
        root.join("shared-time"),
        root.join(format!("{RENDERER}/ns/time")),
    )
    .unwrap();
    source.bind(PEER, &hello(2, 0)).unwrap();
    assert_eq!(source.stream().accepted(), 0);
    assert!(source.stream().calibration().is_none());
    let later = feed(&mut source, 0, 999, 20_000);
    assert!(
        source.quality_at(later).alignment_uncertainty_ms.is_nan(),
        "one frame short"
    );
    let later = feed(&mut source, 999, 1, 20_000).max(later);
    // Calibrated again, but the stamp-accuracy measurement described the earlier process and was
    // dropped when it went away: still unknown, so still denied until it is established afresh.
    assert_eq!(
        source.quality_at(later).frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
    set_accuracy(&mut source, 100_000);
    assert_eq!(source.quality_at(later).frame_primary_eligible(), Ok(()));
}

#[test]
fn a_late_first_frame_in_an_epoch_is_counted_as_unexplained_loss_and_denies() {
    let root = proc_tree("late-start");
    let mut source = source(&root);
    // The hello says the epoch starts at sequence 0; the first frame carries 7.
    source.bind(PEER, &hello(1, 0)).unwrap();
    let stamp = T0;
    source
        .ingest(
            &format!(r#"{{"seq":7,"monotonic_ns":{stamp}}}"#),
            stamp + 1_000,
        )
        .unwrap();
    let now = feed(&mut source, 8, 1100, 20_000);
    set_accuracy(&mut source, 0);
    assert_eq!(source.stream().unexplained_gaps(), 7);
    assert_eq!(
        source.quality_at(now).frame_primary_eligible(),
        Err(QualityDenial::UnexplainedLoss)
    );
}

#[test]
fn malformed_and_impossible_frames_are_rejected_not_absorbed() {
    let root = proc_tree("frames");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    assert_eq!(source.ingest("garbage", 10), Err(IngestError::Malformed));
    assert_eq!(
        source.ingest(r#"{"seq":0,"monotonic_ns":9000}"#, 5000),
        Err(IngestError::Rejected(StampError::FromTheFuture))
    );
    // The malformed line counts as loss too, exactly like the impossible frame.
    assert_eq!(source.stream().rejected(), 2);
}

/// The real kernel's peer credentials for a real socket pair: this very process is the peer, so
/// the same binding succeeds only when the policy names it, and a different expected pid is refused.
#[test]
fn real_socket_peer_credentials_bind_this_process_and_refuse_another() {
    let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    let me = std::process::id();
    let uid = crate::syscall::geteuid();
    let proc_root = Path::new("/proc");
    let live = live_process(proc_root, me).expect("this process is live");
    let mut bound = StampedSource::new(SourcePolicy {
        owner_uid: uid,
        renderer_pid: me,
        renderer_start_ticks: live.start_ticks,
        observer_pid: me,
        proc_root: proc_root.to_path_buf(),
    });
    bound
        .accept(&ours, &hello(1, 0))
        .expect("a real peer must bind");
    assert!(bound.is_bound());

    let mut other = StampedSource::new(SourcePolicy {
        owner_uid: uid,
        renderer_pid: me.wrapping_add(1),
        renderer_start_ticks: live.start_ticks,
        observer_pid: me,
        proc_root: proc_root.to_path_buf(),
    });
    assert_eq!(
        other.accept(&ours, &hello(1, 0)),
        Err(BindDenial::WrongProcess)
    );
}

#[test]
fn a_malformed_truncated_or_oversize_line_after_a_complete_calibration_is_loss_and_denies() {
    let root = proc_tree("invalid-lines");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 1500, 30_000);
    set_accuracy(&mut source, 50_000);
    assert_eq!(source.quality_at(now).frame_primary_eligible(), Ok(()));
    // Truncated JSON, not JSON, and a line far longer than any frame record.
    assert_eq!(
        source.ingest(r#"{"seq":1500,"monotonic_n"#, now),
        Err(IngestError::Malformed)
    );
    let quality = source.quality_at(now);
    assert_eq!(quality.dropped_frames, 1);
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(QualityDenial::UnexplainedLoss)
    );
    assert_eq!(source.ingest("not json", now), Err(IngestError::Malformed));
    let long = format!(
        r#"{{"seq":1501,"monotonic_ns":1,"pad":"{}"}}"#,
        "x".repeat(MAX_STAMPED_LINE_BYTES)
    );
    assert_eq!(source.ingest(&long, now), Err(IngestError::Oversize));
    assert_eq!(source.stream().rejected(), 3);
}

#[test]
fn the_maximum_sequence_number_from_an_authenticated_producer_is_refused_not_a_panic() {
    let root = proc_tree("max-seq");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 10, 10_000);
    let line = format!(r#"{{"seq":{},"monotonic_ns":{}}}"#, u64::MAX, now);
    assert_eq!(
        source.ingest(&line, now + 1),
        Err(IngestError::Rejected(StampError::SequenceOverflow))
    );
    assert_eq!(source.stream().rejected(), 1);
    assert_eq!(source.stream().accepted(), 10);
}

#[test]
fn a_renderer_that_exits_or_has_its_pid_reused_right_before_the_decision_is_no_longer_trusted() {
    let root = proc_tree("dead-at-decision");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 1500, 30_000);
    set_accuracy(&mut source, 50_000);
    assert_eq!(source.quality_at(now).frame_primary_eligible(), Ok(()));
    // The pid is reused by another process: alive, but not the renderer. No ingest happens first.
    set_start(&root, RENDERER, 4040);
    let quality = source.quality_at(now);
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(QualityDenial::NotValidatedDirect)
    );
    assert!(!source.is_bound());

    // And a renderer that simply exited.
    let root = proc_tree("exited-at-decision");
    let mut source = self::source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 1500, 30_000);
    set_accuracy(&mut source, 50_000);
    assert_eq!(source.quality_at(now).frame_primary_eligible(), Ok(()));
    fs::remove_dir_all(root.join(RENDERER.to_string())).unwrap();
    assert_eq!(
        source.quality_at(now).frame_primary_eligible(),
        Err(QualityDenial::NotValidatedDirect)
    );
}
