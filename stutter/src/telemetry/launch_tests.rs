use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use super::{
    frame_source::Attribution,
    launch_discovery::{LaunchRejection, discover, verify_launch},
    live_source::{TargetView, evaluate},
    managed_source::{Drain, ManagedFrameSource},
};
use crate::{autotune::observation::WorkloadIdentity, game_run::LaunchRecord};

/// A process whose address space maps MangoHud: a renderer.
const HUD: &str = "7f00 r-xp /usr/lib/libMangoHud.so\n";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-launch-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn process(proc_root: &Path, pid: u32, ppid: u32, start: u64) {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("stat"),
        format!("{pid} (game) S {ppid} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {start} 0 0"),
    )
    .unwrap();
    // A process whose address space is readable and maps nothing of interest: a launcher or helper.
    fs::write(dir.join("maps"), "7f00 r-xp /usr/lib/libc.so.6\n").unwrap();
}

fn capture(root: &Path, name: &str, record: &LaunchRecord) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        dir.join("launch.json"),
        serde_json::to_string(record).unwrap(),
    )
    .unwrap();
    dir
}

fn record(pid: u32, start: u64, uid: u32) -> LaunchRecord {
    LaunchRecord {
        launch_id: "a".repeat(32),
        boot_id: "boot".into(),
        uid,
        pid,
        start_ticks: start,
        argv: vec!["game".into()],
        mangohud_requested: true,
    }
}

fn uid() -> u32 {
    crate::syscall::geteuid()
}

#[test]
fn a_launch_is_trusted_only_after_every_check_passes() {
    let base = scratch("verify");
    let proc_root = base.join("proc");
    process(&proc_root, 77, 1, 9);
    let ok = capture(&base, "ok", &record(77, 9, uid()));
    assert!(verify_launch(&ok, uid(), "boot", &proc_root).is_ok());
    assert_eq!(
        verify_launch(&ok, uid(), "other-boot", &proc_root).unwrap_err(),
        LaunchRejection::OtherBoot
    );
    let reused = capture(&base, "reused", &record(77, 10, uid()));
    assert_eq!(
        verify_launch(&reused, uid(), "boot", &proc_root).unwrap_err(),
        LaunchRejection::ProcessReplaced
    );
    let gone = capture(&base, "gone", &record(78, 9, uid()));
    assert_eq!(
        verify_launch(&gone, uid(), "boot", &proc_root).unwrap_err(),
        LaunchRejection::ProcessGone
    );
    let foreign = capture(&base, "foreign", &record(77, 9, uid().wrapping_add(1)));
    assert_eq!(
        verify_launch(&foreign, uid(), "boot", &proc_root).unwrap_err(),
        LaunchRejection::OtherUser
    );
    let open = capture(&base, "open", &record(77, 9, uid()));
    fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        verify_launch(&open, uid(), "boot", &proc_root).unwrap_err(),
        LaunchRejection::UnsafeDirectory
    );
    // Discovery returns only the verified one, and ignores a symlinked directory.
    std::os::unix::fs::symlink(&ok, base.join("link")).unwrap();
    let found = discover(&base, uid(), "boot", &proc_root);
    assert_eq!(found.len(), 1, "{found:?}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn frames_flow_from_a_launch_log_and_trust_follows_the_live_target() {
    let base = scratch("flow");
    let proc_root = base.join("proc");
    // The launcher (77) is the wrapper's child; the renderer (100) runs under it.
    process(&proc_root, 77, 1, 9);
    process(&proc_root, 100, 77, 5);
    fs::write(proc_root.join("100/maps"), HUD).unwrap();
    let dir = capture(&base, "run", &record(77, 9, uid()));
    let csv = "fps,frametime,elapsed\n60,16.6,1000000\n60,16.7,17000000\n";
    fs::write(dir.join("game_2026.csv"), csv).unwrap();
    let launch = verify_launch(&dir, uid(), "boot", &proc_root).unwrap();
    let mut source = ManagedFrameSource::new(launch, uid());
    let now = Instant::now();
    let first = source.poll(&proc_root, now, 100);
    assert!(!first.gone);
    let second = source.poll(&proc_root, now, 100);
    assert_eq!(first.frames.len() + second.frames.len(), 2);

    let identity = WorkloadIdentity {
        root_pid: 100,
        process_starttime_ticks: Some(5),
        exe_dev: Some(1),
        exe_ino: Some(2),
        cgroup_path: Some("/c".into()),
        focus_kind: None,
        class_distribution: Default::default(),
        stable_hash: "h".into(),
    };
    // The runtime measures alignment and frame age on every tick; a source that has not been
    // measured is unknown, not fresh and not aligned.
    source.record_alignment(0.5, 0);
    source.record_frame_age(now, Some(Duration::from_millis(5)));
    let status = source.shared_status().lock().unwrap().clone();
    assert_eq!(status.claim.file_owner_uid, Some(uid()));
    let view = |focused| TargetView {
        identity: &identity,
        boot_id: "boot",
        uid: uid(),
        focused,
    };
    let quality = evaluate(&status, &view(true), &proc_root, now);
    assert_eq!(quality.attribution, Attribution::ValidatedDirect);
    assert_eq!(quality.frame_primary_eligible(), Ok(()));
    assert_ne!(
        evaluate(&status, &view(false), &proc_root, now).attribution,
        Attribution::ValidatedDirect
    );

    // The launched process exits: the source reports itself gone and trust cannot continue.
    fs::remove_dir_all(proc_root.join("77")).unwrap();
    assert!(source.poll(&proc_root, now, 100).gone);
    assert!(!source.shared_status().lock().unwrap().healthy);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn frames_produced_before_a_phase_boundary_are_discarded_and_later_ones_still_flow() {
    let base = scratch("phase");
    let proc_root = base.join("proc");
    process(&proc_root, 77, 1, 9);
    let dir = capture(&base, "run", &record(77, 9, uid()));
    let log = dir.join("game_2026.csv");
    let mut csv = String::from("fps,frametime,elapsed\n");
    for index in 1..=10u64 {
        csv.push_str(&format!("60,16.6,{}\n", index * 17_000_000));
    }
    fs::write(&log, &csv).unwrap();
    let launch = verify_launch(&dir, uid(), "boot", &proc_root).unwrap();
    let mut source = ManagedFrameSource::new(launch, uid());
    let now = Instant::now();
    // Ten frames from the previous phase are waiting; the boundary throws them away.
    assert_eq!(
        source.discard_pending(&proc_root, now),
        Drain {
            discarded: 10,
            proven: true
        }
    );
    assert!(source.poll(&proc_root, now, 100).frames.is_empty());
    // Frames written after the boundary are delivered normally.
    let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
    std::io::Write::write_all(&mut file, b"60,16.6,200000000\n60,16.6,217000000\n").unwrap();
    assert_eq!(source.poll(&proc_root, now, 100).frames.len(), 2);
    // A source that is gone discards nothing and does not panic.
    fs::remove_dir_all(proc_root.join("77")).unwrap();
    assert_eq!(
        source.discard_pending(&proc_root, now),
        Drain {
            discarded: 0,
            proven: false
        }
    );
    fs::remove_dir_all(&base).unwrap();
}

fn identity_for(pid: u32, ticks: u64) -> WorkloadIdentity {
    WorkloadIdentity {
        root_pid: pid,
        process_starttime_ticks: Some(ticks),
        exe_dev: Some(1),
        exe_ino: Some(2),
        cgroup_path: Some("/c".into()),
        focus_kind: None,
        class_distribution: Default::default(),
        stable_hash: "h".into(),
    }
}

fn quality_of(
    source: &ManagedFrameSource,
    proc_root: &Path,
    now: Instant,
) -> crate::telemetry::quality::StreamQuality {
    let status = source.shared_status().lock().unwrap().clone();
    let identity = identity_for(100, 5);
    evaluate(
        &status,
        &TargetView {
            identity: &identity,
            boot_id: "boot",
            uid: uid(),
            focused: true,
        },
        proc_root,
        now,
    )
}

/// A launch with a launcher (77) and one renderer (100), a log, and a polled source.
fn launched(name: &str, csv: &str) -> (PathBuf, PathBuf, PathBuf, ManagedFrameSource) {
    let base = scratch(name);
    let proc_root = base.join("proc");
    process(&proc_root, 77, 1, 9);
    process(&proc_root, 100, 77, 5);
    fs::write(proc_root.join("100/maps"), HUD).unwrap();
    let dir = capture(&base, "run", &record(77, 9, uid()));
    fs::write(dir.join("game_2026.csv"), csv).unwrap();
    let launch = verify_launch(&dir, uid(), "boot", &proc_root).unwrap();
    (base, proc_root, dir, ManagedFrameSource::new(launch, uid()))
}

#[test]
fn two_renderers_under_one_launcher_or_none_at_all_deny_direct_attribution() {
    let csv = "fps,frametime,elapsed\n60,16.6,1000000\n";
    let (base, proc_root, _dir, mut source) = launched("renderers", csv);
    let now = Instant::now();
    source.poll(&proc_root, now, 100);
    source.record_frame_age(now, Some(Duration::from_millis(5)));
    source.record_alignment(0.5, 0);
    assert_eq!(
        quality_of(&source, &proc_root, now).attribution,
        Attribution::ValidatedDirect
    );

    // A second MangoHud renderer appears under the same launcher: which one wrote the rows?
    process(&proc_root, 101, 77, 6);
    fs::write(proc_root.join("101/maps"), HUD).unwrap();
    // A new producer must withdraw the earlier grant inside the old five-second cache period.
    let later = now + Duration::from_millis(1);
    source.poll(&proc_root, later, 100);
    assert_ne!(
        quality_of(&source, &proc_root, later).attribution,
        Attribution::ValidatedDirect
    );
    // A competitor exiting cannot retroactively attest the rows collected while it existed.
    fs::remove_dir_all(proc_root.join("101")).unwrap();
    let recovered = later + Duration::from_millis(1);
    source.poll(&proc_root, recovered, 100);
    let quality = quality_of(&source, &proc_root, recovered);
    assert_eq!(quality.attribution, Attribution::ValidatedDirect);
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(super::quality::QualityDenial::UnexplainedLoss)
    );
    assert!(source.discard_pending(&proc_root, recovered).proven);
    source.record_frame_age(recovered, Some(Duration::from_millis(1)));
    assert!(
        quality_of(&source, &proc_root, recovered)
            .frame_primary_eligible()
            .is_ok()
    );
    fs::remove_dir_all(&base).unwrap();

    // No renderer with MangoHud mapped: the producer cannot be established.
    let (base, proc_root, _dir, mut none) = launched("no-renderer", csv);
    fs::remove_file(proc_root.join("100/maps")).unwrap();
    none.poll(&proc_root, now, 100);
    none.record_frame_age(now, Some(Duration::from_millis(5)));
    none.record_alignment(0.5, 0);
    assert_ne!(
        quality_of(&none, &proc_root, now).attribution,
        Attribution::ValidatedDirect
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn two_live_logs_in_one_capture_directory_are_ambiguous() {
    let csv = "fps,frametime,elapsed\n60,16.6,1000000\n";
    let (base, proc_root, dir, mut source) = launched("two-logs", csv);
    let now = Instant::now();
    source.poll(&proc_root, now, 100);
    source.record_frame_age(now, Some(Duration::from_millis(5)));
    source.record_alignment(0.5, 0);
    assert_eq!(
        quality_of(&source, &proc_root, now).attribution,
        Attribution::ValidatedDirect
    );
    fs::write(dir.join("game_other.csv"), csv).unwrap();
    let later = now + Duration::from_millis(1);
    source.poll(&proc_root, later, 100);
    assert_ne!(
        quality_of(&source, &proc_root, later).attribution,
        Attribution::ValidatedDirect
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn rejected_rows_and_missing_timestamps_are_loss_and_a_reset_clears_them() {
    // One good row, one malformed row, one row far beyond any plausible frame time.
    let csv = "fps,frametime,elapsed\n60,16.6,1000000\nnot,a,row\n60,99999999,17000000\n";
    let (base, proc_root, _dir, mut source) = launched("loss", csv);
    let now = Instant::now();
    let out = source.poll(&proc_root, now, 100);
    assert_eq!(out.frames.len(), 1);
    source.record_frame_age(now, Some(Duration::from_millis(5)));
    source.record_alignment(0.5, 0);
    let status = source.shared_status().lock().unwrap().clone();
    assert!(
        status.dropped_frames >= 1,
        "rejected rows are published as loss"
    );
    assert_eq!(
        quality_of(&source, &proc_root, now).frame_primary_eligible(),
        Err(crate::telemetry::quality::QualityDenial::UnexplainedLoss)
    );
    // The loss belonged to the window that ended: a fresh window starts clean.
    source.reset_loss();
    assert_eq!(source.shared_status().lock().unwrap().dropped_frames, 0);
    fs::remove_dir_all(&base).unwrap();

    // No elapsed column at all: no frame can be placed on any clock, so none is delivered.
    let (base, proc_root, _dir, mut untimed) =
        launched("untimed", "fps,frametime\n60,16.6\n60,16.7\n");
    let out = untimed.poll(&proc_root, now, 100);
    assert!(out.frames.is_empty());
    assert!(untimed.shared_status().lock().unwrap().dropped_frames >= 2);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn an_unfinished_drain_at_a_phase_boundary_is_not_a_clean_boundary() {
    // More frames waiting than the bounded drain can read. The bound is shrunk to 2 rounds of 8
    // frames so the case is proven with a small log; the production bound is 64 rounds of 4096.
    let mut csv = String::from("fps,frametime,elapsed\n");
    for index in 1..=(2 * 8 + 5u64) {
        csv.push_str(&format!("60,16.6,{}\n", index * 1_000_000));
    }
    let (base, proc_root, _dir, mut source) = launched("backlog", &csv);
    source.set_drain_bound_for_test(2, 8);
    let now = Instant::now();
    source.record_alignment(0.5, 0);
    source.record_frame_age(now, Some(Duration::from_millis(5)));
    let first = source.discard_pending(&proc_root, now);
    assert!(!first.proven, "the bound ran out before the end of the log");
    assert_eq!(first.discarded, 2 * 8);
    let status = source.shared_status().lock().unwrap().clone();
    assert_eq!(status.unexplained_gaps, 1);
    assert_eq!(
        quality_of(&source, &proc_root, now).frame_primary_eligible(),
        Err(crate::telemetry::quality::QualityDenial::UnexplainedLoss)
    );
    // Draining the remainder reaches the end of the log: the boundary is proven and loss resets.
    let second = source.discard_pending(&proc_root, now);
    assert!(second.proven);
    assert_eq!(source.shared_status().lock().unwrap().unexplained_gaps, 0);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_new_log_of_the_same_launch_is_a_new_generation_that_never_reuses_an_epoch() {
    let csv = "fps,frametime,elapsed\n60,16.6,1000000\n";
    let (base, proc_root, dir, mut source) = launched("generations", csv);
    let now = Instant::now();
    let first = source.poll(&proc_root, now, 100);
    assert_eq!(first.frames.len(), 1);
    let before = first.frames[0].epoch;
    // The game restarts under the same launch and MangoHud opens a newer log.
    std::thread::sleep(Duration::from_millis(20));
    fs::write(
        dir.join("game_restart.csv"),
        "fps,frametime,elapsed\n60,16.6,2000000\n",
    )
    .unwrap();
    let second = source.poll(&proc_root, now, 100);
    assert_eq!(second.frames.len(), 1);
    assert_ne!(
        second.frames[0].epoch, before,
        "an epoch number is never reused across logs"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn with_one_renderer_the_producer_is_that_renderer_so_a_sibling_helper_is_not_the_target() {
    let csv = "fps,frametime,elapsed\n60,16.6,1000000\n";
    let (base, proc_root, _dir, mut source) = launched("sole-renderer", csv);
    // A helper process under the same launcher that never draws.
    process(&proc_root, 150, 77, 11);
    let now = Instant::now();
    source.poll(&proc_root, now, 100);
    source.record_frame_age(now, Some(Duration::from_millis(5)));
    source.record_alignment(0.5, 0);
    let status = source.shared_status().lock().unwrap().clone();
    assert_eq!(
        status.claim.producer_pid,
        Some(100),
        "the producer is the renderer, not the launcher"
    );

    let against = |pid: u32, ticks: u64| {
        let identity = identity_for(pid, ticks);
        evaluate(
            &status,
            &TargetView {
                identity: &identity,
                boot_id: "boot",
                uid: uid(),
                focused: true,
            },
            &proc_root,
            now,
        )
        .attribution
    };
    assert_eq!(
        against(100, 5),
        Attribution::ValidatedDirect,
        "the renderer itself"
    );
    assert_ne!(
        against(150, 11),
        Attribution::ValidatedDirect,
        "a sibling helper is not the producer's tree"
    );
    fs::remove_dir_all(&base).unwrap();
}
