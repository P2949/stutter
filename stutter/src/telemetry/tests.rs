use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

use super::{
    frame_source::*,
    identity::*,
    quality::*,
    session::{ConfinedDir, ConfinementError},
};
use crate::autotune::observation::WorkloadIdentity;

fn identity(pid: u32, ticks: Option<u64>) -> WorkloadIdentity {
    WorkloadIdentity {
        root_pid: pid,
        process_starttime_ticks: ticks,
        exe_dev: Some(1),
        exe_ino: Some(2),
        cgroup_path: Some("/user.slice/game".to_owned()),
        focus_kind: None,
        class_distribution: BTreeMap::new(),
        stable_hash: "h".to_owned(),
    }
}

fn key(pid: u32, ticks: u64) -> WorkloadKey {
    WorkloadKey::from_identity(&identity(pid, Some(ticks)), "boot", 1000).unwrap()
}

fn claim(pid: u32, ticks: u64) -> SourceClaim {
    SourceClaim {
        stage: MeasurementStage::ApplicationFrame,
        producer_pid: Some(pid),
        producer_start_ticks: Some(ticks),
        file_owner_uid: Some(1000),
        launch_id_matched: false,
    }
}

fn observed(pid: u32, ticks: u64) -> SourceObservation {
    SourceObservation {
        target: key(100, 5),
        target_focused: true,
        producer: Some(LiveProcess {
            start_ticks: ticks,
            uid: 1000,
            ancestors: vec![(pid.saturating_sub(1), 4), (100, 5)],
        }),
        competing_targets: 0,
        source_age_ms: 10,
    }
}

fn verdict(claim: &SourceClaim, observed: &SourceObservation) -> Result<Attribution, SourceDenial> {
    attribute(claim, MeasurementStage::ApplicationFrame, observed)
}

#[test]
fn a_renderer_under_the_launch_root_with_verified_ownership_is_validated_direct() {
    // The launcher is the root; the renderer is a Proton descendant that re-executed.
    assert_eq!(
        verdict(&claim(200, 9), &observed(200, 9)),
        Ok(Attribution::ValidatedDirect)
    );
}

#[test]
fn identity_is_missing_rather_than_assumed() {
    assert_eq!(
        WorkloadKey::from_identity(&identity(1, None), "boot", 1000),
        Err(IdentityDenial::MissingStartTicks)
    );
    assert_eq!(
        WorkloadKey::from_identity(&identity(1, Some(1)), " ", 1000),
        Err(IdentityDenial::EmptyBootId)
    );
}

#[test]
fn a_reused_pid_never_matches_the_old_workload() {
    assert_ne!(key(100, 5), key(100, 6));
    let mut live = observed(200, 9);
    live.producer.as_mut().unwrap().start_ticks = 10;
    assert_eq!(verdict(&claim(200, 9), &live), Err(SourceDenial::PidReused));
}

#[test]
fn every_mismatch_is_a_typed_denial() {
    let ok = observed(200, 9);
    let mut wrong_user = ok.clone();
    wrong_user.producer.as_mut().unwrap().uid = 0;
    assert_eq!(
        verdict(&claim(200, 9), &wrong_user),
        Err(SourceDenial::WrongUser)
    );

    let mut foreign_file = claim(200, 9);
    foreign_file.file_owner_uid = Some(0);
    assert_eq!(verdict(&foreign_file, &ok), Err(SourceDenial::WrongUser));

    let mut gone = ok.clone();
    gone.producer = None;
    assert_eq!(
        verdict(&claim(200, 9), &gone),
        Err(SourceDenial::ProducerGone)
    );

    // Same-UID unrelated log: live, right user, but outside the target's tree.
    let mut unrelated = ok.clone();
    unrelated.producer.as_mut().unwrap().ancestors = vec![(1, 1)];
    assert_eq!(
        verdict(&claim(200, 9), &unrelated),
        Err(SourceDenial::NotInTargetTree)
    );

    let mut two_games = ok.clone();
    two_games.competing_targets = 1;
    assert_eq!(
        verdict(&claim(200, 9), &two_games),
        Err(SourceDenial::AmbiguousTargets)
    );

    let mut swapped = ok.clone();
    swapped.target_focused = false;
    assert_eq!(
        verdict(&claim(200, 9), &swapped),
        Err(SourceDenial::TargetNotFocused)
    );

    let mut stale = ok.clone();
    stale.source_age_ms = MAX_SOURCE_AGE_MS + 1;
    assert_eq!(
        verdict(&claim(200, 9), &stale),
        Err(SourceDenial::StaleSource)
    );

    let mut wrong_stage = claim(200, 9);
    wrong_stage.stage = MeasurementStage::KmsFlip;
    assert_eq!(verdict(&wrong_stage, &ok), Err(SourceDenial::StageMismatch));
}

#[test]
fn a_launch_id_binds_but_never_validates() {
    let mut unverified_owner = claim(200, 9);
    unverified_owner.file_owner_uid = None;
    unverified_owner.launch_id_matched = true;
    assert_eq!(
        verdict(&unverified_owner, &observed(200, 9)),
        Ok(Attribution::LaunchBound)
    );
    unverified_owner.launch_id_matched = false;
    assert_eq!(
        verdict(&unverified_owner, &observed(200, 9)),
        Ok(Attribution::Correlated)
    );
    let anonymous = SourceClaim {
        producer_pid: None,
        ..claim(0, 0)
    };
    assert_eq!(
        verdict(&anonymous, &observed(200, 9)),
        Ok(Attribution::ObservedUnattributed)
    );
    assert!(Attribution::LaunchBound < Attribution::ValidatedDirect);
}

fn good_quality() -> StreamQuality {
    StreamQuality {
        attribution: Attribution::ValidatedDirect,
        stage: MeasurementStage::ApplicationFrame,
        unexplained_sequence_gaps: 0,
        dropped_frames: 0,
        alignment_uncertainty_ms: 0.5,
        source_healthy: true,
    }
}

#[test]
fn only_a_clean_validated_application_stream_is_frame_primary() {
    assert_eq!(good_quality().frame_primary_eligible(), Ok(()));
    let weaker = |attribution| StreamQuality {
        attribution,
        ..good_quality()
    };
    for attribution in [
        Attribution::LaunchBound,
        Attribution::Correlated,
        Attribution::Unavailable,
    ] {
        assert_eq!(
            weaker(attribution).frame_primary_eligible(),
            Err(QualityDenial::NotValidatedDirect)
        );
    }
    let kms = StreamQuality {
        stage: MeasurementStage::KmsFlip,
        ..good_quality()
    };
    assert_eq!(
        kms.frame_primary_eligible(),
        Err(QualityDenial::NotApplicationFrame)
    );
    let lossy = StreamQuality {
        dropped_frames: 1,
        ..good_quality()
    };
    assert_eq!(
        lossy.frame_primary_eligible(),
        Err(QualityDenial::UnexplainedLoss)
    );
    let gapped = StreamQuality {
        unexplained_sequence_gaps: 1,
        ..good_quality()
    };
    assert_eq!(
        gapped.frame_primary_eligible(),
        Err(QualityDenial::UnexplainedLoss)
    );
    let skewed = StreamQuality {
        alignment_uncertainty_ms: f64::NAN,
        ..good_quality()
    };
    assert_eq!(
        skewed.frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
    let sick = StreamQuality {
        source_healthy: false,
        ..good_quality()
    };
    assert_eq!(
        sick.frame_primary_eligible(),
        Err(QualityDenial::SourceUnhealthy)
    );
}

#[test]
fn a_rotated_source_starts_a_new_epoch() {
    let id = TelemetryStreamId::new(
        key(100, 5),
        MeasurementStage::ApplicationFrame,
        "c".into(),
        0,
    );
    assert!(id.same_epoch(&id.clone()));
    assert!(!id.same_epoch(&id.next_epoch()));
    assert_eq!(new_capture_id().unwrap().len(), 32);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-telemetry-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn my_uid() -> u32 {
    crate::syscall::geteuid()
}

#[test]
fn path_abuse_is_refused_on_the_opened_descriptor() {
    let dir = scratch("confine");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(dir.join("log.csv"), "a").unwrap();
    symlink("/etc/passwd", dir.join("link.csv")).unwrap();
    fs::create_dir(dir.join("sub")).unwrap();
    let confined = ConfinedDir::open(&dir, my_uid()).unwrap();

    let (_, first) = confined.open_file("log.csv").unwrap();
    assert_eq!(first.uid, my_uid());
    assert!(matches!(
        confined.open_file("../x"),
        Err(ConfinementError::BadName)
    ));
    assert!(matches!(
        confined.open_file("a/b"),
        Err(ConfinementError::BadName)
    ));
    assert!(matches!(
        confined.open_file(""),
        Err(ConfinementError::BadName)
    ));
    assert!(matches!(
        confined.open_file("link.csv"),
        Err(ConfinementError::Io(_))
    ));
    assert!(matches!(
        confined.open_file("sub"),
        Err(ConfinementError::NotRegularFile)
    ));

    // A replaced inode is a different source.
    fs::remove_file(dir.join("log.csv")).unwrap();
    fs::write(dir.join("log.csv"), "b").unwrap();
    let (_, second) = confined.open_file("log.csv").unwrap();
    assert!(second.ino != first.ino || second.dev != first.dev || first == second);
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_permissive_foreign_or_symlinked_directory_is_refused() {
    let dir = scratch("dirs");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        ConfinedDir::open(&dir, my_uid()),
        Err(ConfinementError::PermissiveMode)
    ));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        ConfinedDir::open(&dir, my_uid().wrapping_add(1)),
        Err(ConfinementError::WrongOwner)
    ));
    let link = dir.join("dirlink");
    symlink(&dir, &link).unwrap();
    assert!(matches!(
        ConfinedDir::open(&link, my_uid()),
        Err(ConfinementError::NotADirectory)
    ));
    fs::remove_dir_all(&dir).unwrap();
}
