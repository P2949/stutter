use std::{fs, path::Path};

use super::*;
use crate::telemetry::quality::QualityDenial;

const RENDERER: u32 = 4242;
const OBSERVER: u32 = 100;
const OWNER: u32 = 1000;
const T0: u64 = 1_000_000_000;

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

fn accuracy(error_ns: u64) -> StampAccuracy {
    StampAccuracy {
        max_error_ns: error_ns,
        samples: MIN_ACCURACY_SAMPLES,
        reference: "photodiode on a controlled display".into(),
    }
}

#[test]
fn an_authenticated_producer_with_both_bounds_is_eligible_under_the_unchanged_gate() {
    let root = proc_tree("ok");
    let mut source = source(&root);
    source.bind(PEER, &hello(1, 0)).unwrap();
    let now = feed(&mut source, 0, 1500, 100_000);
    assert!(source.set_stamp_accuracy(accuracy(400_000)));
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
    assert!(source.set_stamp_accuracy(accuracy(2_500_000)));
    assert_eq!(
        source.quality_at(now).frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
}

#[test]
fn an_accuracy_claim_needs_enough_samples_and_a_named_reference() {
    let root = proc_tree("accuracy");
    let mut source = source(&root);
    let few = StampAccuracy {
        samples: MIN_ACCURACY_SAMPLES - 1,
        ..accuracy(1)
    };
    let unnamed = StampAccuracy {
        reference: "  ".into(),
        ..accuracy(1)
    };
    assert!(!source.set_stamp_accuracy(few));
    assert!(!source.set_stamp_accuracy(unnamed));
    assert!(source.set_stamp_accuracy(accuracy(1)));
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
    assert!(source.set_stamp_accuracy(accuracy(100_000)));
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
    assert!(source.set_stamp_accuracy(accuracy(100_000)));
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
    assert!(source.set_stamp_accuracy(accuracy(0)));
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
    assert_eq!(source.stream().rejected(), 1);
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
