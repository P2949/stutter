use super::*;

#[test]
fn a_feature_study_enables_exactly_its_factor_and_the_next_study_starts_from_baseline() {
    use crate::autotune::research::{ResearchAction, ResearchFactor, child_study::Stage};
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut parent = parent();
    parent.factors = vec![ResearchFactor::NtSync];
    parent.actions = vec![ResearchAction::CpuAffinityProfile];
    let plan = plan(&parent).unwrap();
    let feature = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap()
        .clone();
    let confirm = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Confirm))
        .unwrap()
        .clone();
    let host = Host::finishing();
    let seen = host.seen_factors.clone();
    let mut now = parent.issued_unix_seconds + 1;
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    );
    run_to_end(&mut runner, &feature);
    run_to_end(&mut runner, &confirm);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![vec![ResearchFactor::NtSync], Vec::<ResearchFactor>::new()],
        "the feature study's factor is enabled for it alone"
    );
    // A feature study the campaign never authorized is refused before any directory exists.
    let mut unauthorized = parent.clone();
    unauthorized.factors.clear();
    let root2 = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root2.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut now2 = parent.issued_unix_seconds + 1;
    let mut refusing = DriverStudyAttempts::new(
        File::open(root2.path()).unwrap(),
        unauthorized,
        Box::new(Host::finishing()),
        Box::new(move || {
            now2 += 1;
            now2
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    );
    assert!(refusing.poll(&feature, 100).is_err());
    assert!(!root2.path().join("studies").exists());
}

mod activation {
    use super::*;
    use crate::autotune::research::Treatment::{Off, On};

    fn runner() -> (tempfile::TempDir, DriverStudyAttempts, ChildStudy) {
        let (root, runner, study, _) = attempts(Host::finishing());
        (root, runner, study)
    }

    #[test]
    fn every_completed_attempt_must_show_its_own_treatment_and_both_treatments_must_run() {
        let (_root, mut runner, _study) = runner();
        let mut with_factor = parent();
        with_factor.factors = vec![crate::autotune::research::ResearchFactor::NtSync];
        let feature = plan(&with_factor)
            .unwrap()
            .into_iter()
            .find(|s| {
                matches!(
                    s.stage,
                    crate::autotune::research::child_study::Stage::Feature(_)
                )
            })
            .expect("a feature study");
        // Not a feature study (or nothing observed): no claim either way.
        assert_eq!(
            runner.feature_activation(&plan(&parent()).unwrap()[0], 4),
            None
        );
        runner.feature_observed.insert(0, (Off, true));
        runner.feature_observed.insert(1, (On, true));
        assert_eq!(runner.feature_activation(&feature, 2), Some(true));
        // An attempt that did not show its treatment, a missing observation, or one treatment only.
        runner.feature_observed.insert(1, (On, false));
        assert_eq!(runner.feature_activation(&feature, 2), Some(false));
        runner.feature_observed.insert(1, (On, true));
        assert_eq!(
            runner.feature_activation(&feature, 3),
            Some(false),
            "an attempt unobserved"
        );
        runner.feature_observed.remove(&0);
        runner.feature_observed.insert(0, (On, true));
        assert_eq!(
            runner.feature_activation(&feature, 2),
            Some(false),
            "never off"
        );
    }
}

#[test]
fn a_feature_study_records_per_attempt_whether_its_treatment_reached_the_game_process() {
    use std::os::unix::fs::PermissionsExt;

    use crate::autotune::research::{ResearchFactor, Treatment, child_study::Stage};
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut parent = parent();
    parent.factors = vec![ResearchFactor::NtSync];
    let feature = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap();
    let mut host = Host::finishing();
    host.process = Some(crate::autotune::research::action_child::ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: crate::syscall::geteuid(),
    });
    host.treatment = Some(Treatment::On);
    let mut now = parent.issued_unix_seconds + 1;
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    )
    // NTSync on is the absence of the disabling variable: this environment shows it reached.
    .with_environ_reader(Box::new(|pid| {
        assert_eq!(pid, 4242, "the sighted game process is the one read");
        Some(b"HOME=/h\0".to_vec())
    }));
    let outcome = run_to_end(&mut runner, &feature);
    assert!(!runner.feature_observed.is_empty());
    assert!(
        runner
            .feature_observed
            .values()
            .all(|(treatment, reached)| *treatment == Treatment::On && *reached)
    );
    // Only the On treatment ever ran in this fixture, so activation is not shown for the study.
    assert_eq!(outcome.activation, Some(false));
}

#[test]
fn each_attempts_telemetry_receipt_is_kept_beside_its_study() {
    use std::os::unix::fs::PermissionsExt;

    use crate::autotune::research::telemetry_observer::{SharedTelemetry, SourceSnapshot};
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = parent();
    let study = plan(&parent).unwrap().remove(0);
    let mut host = Host::finishing();
    let target = crate::autotune::research::action_child::ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: crate::syscall::geteuid(),
    };
    host.process = Some(target);
    let telemetry: SharedTelemetry = Default::default();
    let mut now = parent.issued_unix_seconds + 1;
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    )
    .with_telemetry(telemetry.clone());
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    // The runtime reports what it sees of the attested game while the attempt runs.
    {
        let mut observer = telemetry.lock().unwrap();
        assert!(
            observer.is_observing(),
            "the sighted process is being observed"
        );
        for _ in 0..2_400 {
            observer.count_frame();
        }
        observer.observe_source(SourceSnapshot {
            producer_pid: Some(4242),
            producer_start_ticks: Some(99),
            dropped_frames: 0,
            unexplained_gaps: 0,
            healthy: true,
            competing_targets: 0,
            alignment_uncertainty_ms: 0.5,
            frame_age: Some(std::time::Duration::from_millis(5)),
        });
    }
    run_to_end(&mut runner, &study);
    let file = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("telemetry-receipts.json");
    let receipts: serde_json::Value =
        serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(receipts[0]["pid"], 4242);
    assert_eq!(
        receipts[0]["attempt"], 0,
        "the receipt names its journal slot"
    );
    assert_eq!(receipts[0]["frames"], 2_400);
    assert_eq!(receipts[0]["source_bound_to_game"], true);
    assert_eq!(receipts[0]["frame_verdict"], "Eligible");
}

#[test]
fn a_telemetry_receipt_that_cannot_be_recorded_is_an_evidence_failure_not_a_silent_omission() {
    use std::os::unix::fs::PermissionsExt;

    use crate::autotune::research::telemetry_observer::SharedTelemetry;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = parent();
    let study = plan(&parent).unwrap().remove(0);
    let mut host = Host::finishing();
    host.process = Some(crate::autotune::research::action_child::ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: crate::syscall::geteuid(),
    });
    let telemetry: SharedTelemetry = Default::default();
    let mut now = parent.issued_unix_seconds + 1;
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(host),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    )
    .with_telemetry(telemetry);
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    // Something already holds the receipts file, so this attempt's receipt cannot be written.
    let file = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("telemetry-receipts.json");
    std::fs::write(&file, b"[]").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut failure = None;
    for _ in 0..200 {
        match runner.poll(&study, 100) {
            Err(error) => {
                failure = Some(error.to_string());
                break;
            }
            Ok(Some(_)) => break,
            Ok(None) => {}
        }
    }
    let message = failure.expect("the study fails instead of dropping the receipt");
    assert!(
        message.contains("telemetry receipts not recorded"),
        "{message}"
    );
}
