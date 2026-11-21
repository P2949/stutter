use super::*;

fn read_spent(
    root: &tempfile::TempDir,
    study: &ChildStudy,
) -> crate::autotune::research::campaign_budget::Spent {
    let path = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("budget.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_report_conflict_keeps_the_finished_driver_until_accounting_can_be_retried() {
    let (root, mut runner, study, _) = attempts(Host::finishing());
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    let path = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("report.json");
    std::fs::write(&path, b"conflicting report").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut failed = false;
    for _ in 0..200 {
        match runner.poll(&study, 100) {
            Err(_) => {
                failed = true;
                break;
            }
            Ok(None) => {}
            Ok(Some(_)) => panic!("a conflicting report was accepted"),
        }
    }
    assert!(failed);
    assert!(runner.has_running(), "the settled driver remains owned");
    assert!(path.with_file_name("budget.json").is_file());
    std::fs::remove_file(&path).unwrap();
    let outcome = run_to_end(&mut runner, &study);
    assert!(outcome.attempts_used > 0);
    assert!(!runner.has_running());
}

#[test]
fn telemetry_is_retained_when_its_durable_receipt_conflicts() {
    use std::sync::{Arc, Mutex};
    let (root, mut runner, study, parent) = attempts(Host::finishing());
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    let observer = Arc::new(Mutex::new(
        crate::autotune::research::telemetry_observer::TelemetryObserver::default(),
    ));
    observer.lock().unwrap().arm(
        crate::autotune::research::action_child::ChildTarget {
            pid: stutter_core::ids::Pid::new(4242),
            start_ticks: 99,
            uid: parent.uid,
        },
        Some(0),
    );
    runner.telemetry = Some(observer.clone());
    let path = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("telemetry-receipts.json");
    std::fs::write(&path, b"conflicting telemetry").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(runner.record_telemetry(&study).is_err());
    assert_eq!(observer.lock().unwrap().freeze_receipts().len(), 1);
    std::fs::remove_file(&path).unwrap();
    runner.record_telemetry(&study).unwrap();
    assert!(observer.lock().unwrap().freeze_receipts().is_empty());
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(saved.as_array().unwrap().len(), 1);
}

#[test]
fn a_second_child_starts_from_what_the_first_already_spent_of_the_campaign() {
    let (root, mut runner, first, parent) = attempts(Host::finishing());
    let second = plan(&parent).unwrap().remove(1);
    run_to_end(&mut runner, &first);
    let spent = read_spent(&root, &first);
    assert!(
        spent.active_seconds > 0,
        "the first child's time was charged"
    );
    // The second child's driver does not start from zero: the campaign's budget is shared.
    assert_eq!(runner.poll(&second, 100).unwrap(), None);
    let (_, driver) = runner.running.as_ref().unwrap();
    assert_eq!(driver.active_seconds(), spent.active_seconds);
}

#[test]
fn three_children_record_only_their_own_active_time() {
    let (root, mut runner, _, parent) = attempts(Host::finishing());
    let studies = plan(&parent).unwrap();
    let mut total = 0;
    for study in studies.iter().take(3) {
        assert_eq!(runner.poll(study, 100).unwrap(), None);
        let driver = &runner.running.as_ref().unwrap().1;
        assert_eq!(driver.active_seconds(), total);
        assert_eq!(driver.child_active_seconds(), 0);
        run_to_end(&mut runner, study);
        let child = read_spent(&root, study).active_seconds;
        assert!(child > 0);
        total += child;
    }
    let studies_dir = File::open(root.path().join("studies")).unwrap();
    assert_eq!(
        crate::autotune::research::campaign_budget::spent(
            &studies_dir,
            parent.uid,
            u64::from(parent.max_active_seconds)
        )
        .unwrap()
        .active_seconds,
        total
    );
    assert_eq!(runner.poll(&studies[3], 100).unwrap(), None);
    assert_eq!(runner.running.as_ref().unwrap().1.active_seconds(), total);
}

#[test]
fn a_child_that_would_start_over_an_exhausted_campaign_budget_is_stopped_at_once() {
    let (root, mut runner, first, parent) = attempts(Host::finishing());
    let second = plan(&parent).unwrap().remove(1);
    run_to_end(&mut runner, &first);
    // Pretend the first child used the whole authorization (its record is rewritten before the
    // second begins, as a campaign that had spent that much would have recorded).
    let path = root
        .path()
        .join("studies")
        .join(&first.study_sha256[..16])
        .join("budget.json");
    let mut spent = read_spent(&root, &first);
    spent.active_seconds = u64::from(parent.max_active_seconds);
    std::fs::write(&path, serde_json::to_vec(&spent).unwrap()).unwrap();
    let outcome = run_to_end(&mut runner, &second);
    assert_eq!(
        outcome.attempts_used, 0,
        "no attempt may start over an exhausted active budget"
    );
}

#[test]
fn actual_viable_arm_count_sets_the_pilot_cost_before_any_study_directory_exists() {
    use crate::autotune::research::{child_study::Stage, study_executor::StudyAttempts};
    let root = tempfile::tempdir().unwrap();
    let mut intent = parent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::Em,
        identity_sha256: "c".repeat(64),
    });
    intent.max_attempts = 8;
    let study = plan(&intent)
        .unwrap()
        .into_iter()
        .find(|s| s.stage == Stage::Pilot)
        .unwrap();
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        intent,
        Box::new(Host::finishing()),
        Box::new(|| 11),
        Box::new(|| Ok(())),
        Box::new(|_| "/nonexistent".into()),
        Box::new(|_| {
            Ok(StudyInputs {
                arms: vec![
                    ProtonSource::Ge,
                    ProtonSource::ValveStable,
                    ProtonSource::Em,
                ],
                rule: None,
            })
        }),
    );
    assert_eq!(runner.required_attempts(&study).unwrap(), 9);
    assert!(runner.poll(&study, 8).is_err());
    assert!(!root.path().join("studies").exists());
}
