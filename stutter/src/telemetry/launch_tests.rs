use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Instant,
};

use super::{
    frame_source::Attribution,
    launch_discovery::{LaunchRejection, discover, verify_launch},
    live_source::{TargetView, evaluate},
    managed_source::ManagedFrameSource,
};
use crate::{autotune::observation::WorkloadIdentity, game_run::LaunchRecord};

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
    assert_eq!(source.discard_pending(&proc_root, now), 10);
    assert!(source.poll(&proc_root, now, 100).frames.is_empty());
    // Frames written after the boundary are delivered normally.
    let mut file = fs::OpenOptions::new().append(true).open(&log).unwrap();
    std::io::Write::write_all(&mut file, b"60,16.6,200000000\n60,16.6,217000000\n").unwrap();
    assert_eq!(source.poll(&proc_root, now, 100).frames.len(), 2);
    // A source that is gone discards nothing and does not panic.
    fs::remove_dir_all(proc_root.join("77")).unwrap();
    assert_eq!(source.discard_pending(&proc_root, now), 0);
    fs::remove_dir_all(&base).unwrap();
}
