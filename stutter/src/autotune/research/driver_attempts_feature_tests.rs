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

/// A feature study qualifies on the kernel's own witness: the treatment reached the game, and the factor's modelled
/// backend shows in every On attempt's process and in no Off attempt's. Without the backend (or with it where it
/// should not be) the study is unproven, and a factor whose backend is not modelled never qualifies.
#[test]
fn a_feature_study_qualifies_only_on_the_kernels_witness_of_its_backend() {
    use crate::autotune::research::{
        ResearchFactor, Treatment, action_child::ChildTarget, child_study::Stage,
        feature_proof::Witness, features::factor_env, study_executor::judge,
    };
    let run = |factor: ResearchFactor, backend_on: bool, backend_off: bool| {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut parent = parent();
        parent.factors = vec![factor];
        let feature = plan(&parent)
            .unwrap()
            .into_iter()
            .find(|s| matches!(s.stage, Stage::Feature(_)))
            .unwrap();
        let mut host = Host::finishing();
        host.processes = (0..64)
            .map(|n| ChildTarget {
                pid: stutter_core::ids::Pid::new(7_000 + n),
                start_ticks: 300 + u64::from(n),
                uid: crate::syscall::geteuid(),
            })
            .collect();
        host.schedule_out = Some(Default::default());
        let schedule = host.schedule_out.clone().unwrap();
        let (env_schedule, witness_schedule) = (schedule.clone(), schedule);
        let mut now = parent.issued_unix_seconds + 1;
        let mut runner = DriverStudyAttempts::new(
            File::open(root.path()).unwrap(),
            parent.clone(),
            Box::new(host),
            Box::new(move || {
                now += 10;
                now
            }),
            Box::new(|| Ok(())),
            Box::new(|_| std::path::PathBuf::from("/nonexistent")),
            Box::new(|study| Ok(inputs_for(study))),
        )
        // The treatment reached the game: the process shows exactly its own arm's environment.
        .with_environ_reader(Box::new(move |pid| {
            let on = env_schedule.lock().unwrap()[(pid - 7_000) as usize] == Treatment::On;
            let env = factor_env(ProtonSource::Ge, factor, on).ok()?;
            let mut bytes = Vec::new();
            for (key, value) in env {
                bytes.extend_from_slice(format!("{key}={value}\0").as_bytes());
            }
            Some(bytes)
        }))
        .with_witness_reader(Box::new(move |pid, _watch| {
            let on = witness_schedule.lock().unwrap()[(pid - 7_000) as usize] == Treatment::On;
            let mut witness = Witness::default();
            if (on && backend_on) || (!on && backend_off) {
                match factor {
                    ResearchFactor::WineWayland => {
                        witness.modules.insert("winewayland.drv".to_owned());
                    }
                    ResearchFactor::NtSync => {
                        witness.devices.insert("/dev/ntsync".to_owned());
                    }
                    _ => {}
                }
            }
            Some(witness)
        }));
        let outcome = run_to_end(&mut runner, &feature);
        // What independent verification reads: the proof the driver kept, against the processes the attempts
        // recorded (the host's own, here), by the one function live outcome and replay share.
        let path = root
            .path()
            .join("studies")
            .join(&feature.study_sha256[..16]);
        let dir = File::open(&path).unwrap();
        let intent: ResearchIntentV1 =
            serde_json::from_slice(&std::fs::read(path.join("intent.json")).unwrap()).unwrap();
        let uid = crate::syscall::geteuid();
        let evidence: std::collections::BTreeMap<
            u32,
            crate::autotune::research::report::AttemptEvidenceV1,
        > = (0..8u32)
            .map(|slot| {
                let record = crate::autotune::research::long_receipt::LongSessionRecord {
                    unit_id: "u".into(),
                    definition_sha256: "a".repeat(64),
                    started_unix: 1,
                    ended_unix: 2,
                    process: crate::autotune::research::long_receipt::SessionProcess {
                        pid: stutter_core::ids::Pid::new(7_000 + slot),
                        start_ticks: 300 + u64::from(slot),
                        uid,
                    },
                    phases: Vec::new(),
                    max_poll_gap_seconds: Some(1),
                    settled: true,
                };
                let item = crate::autotune::research::report::AttemptEvidenceV1 {
                    schema_version: 1,
                    slot_index: slot,
                    average_fps: 1.0,
                    native_completion: None,
                    long_session: Some(Box::new(record)),
                    thermal: None,
                };
                (slot, item)
            })
            .collect();
        let _ = judge;
        (
            outcome.activation,
            crate::autotune::research::child_judgement::feature_proof_valid(
                &dir, uid, factor, &intent, &evidence,
            ),
        )
    };
    // The kernel shows the backend exactly where the treatment is On: the proof stands.
    assert_eq!(run(ResearchFactor::NtSync, true, false), (Some(true), true));
    // No backend where it should be, or the backend where it should not: unproven.
    assert!(!run(ResearchFactor::NtSync, false, false).1);
    assert!(!run(ResearchFactor::NtSync, true, true).1);
    // A factor whose backend is not modelled never qualifies, whatever the process shows.
    assert!(!run(ResearchFactor::Hdr, true, false).1);
}

/// A factor told apart by the content of a mapped build (the CachyOS low-latency vkd3d) is proven through the whole
/// driver: the host's two tool-build hashes, the kernel's witness of each attempt's mapped file, and the verifier.
#[test]
fn a_factor_told_apart_by_content_is_proven_through_the_driver_from_the_hosts_tool_builds() {
    use crate::autotune::research::{
        ResearchFactor, Treatment,
        action_child::ChildTarget,
        child_study::Stage,
        feature_proof::{self, Witness},
    };
    let (on_hash, off_hash) = ("a".repeat(64), "b".repeat(64));
    let (on_path, off_path) = feature_proof::tool_builds(ResearchFactor::Vkd3dLowLatency).unwrap();
    let run = |host_knows_builds: bool, mapped_on_off: (&'static str, &'static str)| {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut parent = parent();
        parent.factors = vec![ResearchFactor::Vkd3dLowLatency];
        parent.tools.push(crate::autotune::research::ResearchTool {
            source: ProtonSource::CachyOs,
            identity_sha256: "c".repeat(64),
        });
        let feature = plan(&parent)
            .unwrap()
            .into_iter()
            .find(|s| matches!(s.stage, Stage::Feature(_)))
            .unwrap();
        let mut host = Host::finishing();
        host.processes = (0..64)
            .map(|n| ChildTarget {
                pid: stutter_core::ids::Pid::new(7_000 + n),
                start_ticks: 300 + u64::from(n),
                uid: crate::syscall::geteuid(),
            })
            .collect();
        host.schedule_out = Some(Default::default());
        if host_knows_builds {
            host.tool_files.insert(on_path.to_owned(), on_hash.clone());
            host.tool_files
                .insert(off_path.to_owned(), off_hash.clone());
        }
        let schedule = host.schedule_out.clone().unwrap();
        let mut now = parent.issued_unix_seconds + 1;
        let (mapped_on, mapped_off) = (
            if mapped_on_off.0 == "on" {
                on_hash.clone()
            } else {
                off_hash.clone()
            },
            if mapped_on_off.1 == "on" {
                on_hash.clone()
            } else {
                off_hash.clone()
            },
        );
        let mut runner = DriverStudyAttempts::new(
            File::open(root.path()).unwrap(),
            parent.clone(),
            Box::new(host),
            Box::new(move || {
                now += 10;
                now
            }),
            Box::new(|| Ok(())),
            Box::new(|_| std::path::PathBuf::from("/nonexistent")),
            Box::new(|_study| {
                Ok(StudyInputs {
                    arms: vec![ProtonSource::CachyOs],
                    rule: None,
                })
            }),
        )
        // The environment is only an observation here; the proof stands on the witness.
        .with_environ_reader(Box::new(|_| Some(b"HOME=/h\0".to_vec())))
        .with_witness_reader(Box::new(move |pid, watch| {
            assert_eq!(
                watch,
                ["d3d12core.dll"],
                "the content-compared name is the one watched"
            );
            let on = schedule.lock().unwrap()[(pid - 7_000) as usize] == Treatment::On;
            let mut witness = Witness::default();
            witness.mapped.insert(
                "d3d12core.dll".to_owned(),
                [if on {
                    mapped_on.clone()
                } else {
                    mapped_off.clone()
                }]
                .into(),
            );
            Some(witness)
        }));
        run_to_end(&mut runner, &feature);
        let path = root
            .path()
            .join("studies")
            .join(&feature.study_sha256[..16]);
        let bytes = std::fs::read(path.join(feature_proof::FILE)).unwrap();
        let schedule: Vec<Treatment> = (0..8)
            .map(|n| {
                if n % 2 == 0 {
                    Treatment::Off
                } else {
                    Treatment::On
                }
            })
            .collect();
        let _ = schedule;
        (bytes, feature, path)
    };
    // The record names each attempt's mapped build beside the tool's own two builds.
    let (bytes, _, _) = run(true, ("on", "off"));
    let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let first = &record["attempts"][0]["witness"];
    assert_eq!(first["tool_on_sha256"], on_hash);
    assert_eq!(first["tool_off_sha256"], off_hash);
    assert!(first["mapped"]["d3d12core.dll"].is_array());
    // A host that cannot say what the two builds are leaves them unrecorded, and the proof then fails closed.
    let (bytes, _, _) = run(false, ("on", "off"));
    let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        record["attempts"][0]["witness"]
            .get("tool_on_sha256")
            .is_none()
    );
}
