//! A feature study that was not qualified is replayed offline: its reason must follow from its sealed
//! records and the per-attempt observation it recorded, with the treatments checked against the intent's
//! own schedule, never from a live boolean.

use std::sync::{Arc, Mutex};

use super::*;
use crate::autotune::research::{
    ResearchFactor, Treatment,
    child_ledger::Disposition,
    child_study::{Stage, StudyResult},
    study_executor::judge,
};

struct Run {
    root: tempfile::TempDir,
    parent: ResearchIntentV1,
    study: ChildStudy,
    reason: String,
}

fn run_feature() -> Run {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut parent = parent();
    parent.factors = vec![ResearchFactor::NtSync];
    let study = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap();
    let schedule: Arc<Mutex<Vec<Treatment>>> = Arc::default();
    let mut host = Host::finishing();
    host.process = Some(crate::autotune::research::action_child::ChildTarget {
        pid: stutter_core::ids::Pid::new(4242),
        start_ticks: 99,
        uid: crate::syscall::geteuid(),
    });
    host.schedule_out = Some(schedule.clone());
    // The game's own environment shows each attempt's treatment, in the order the attempts launched:
    // NTSync off is the disabling variable, on is its absence.
    let calls = Arc::new(Mutex::new(0usize));
    let env = {
        let schedule = schedule.clone();
        move |_: u32| {
            let mut calls = calls.lock().unwrap();
            let schedule = schedule.lock().unwrap();
            let treatment = schedule[*calls % schedule.len()];
            *calls += 1;
            Some(match treatment {
                Treatment::Off => b"HOME=/h\0PROTON_NO_NTSYNC=1\0".to_vec(),
                Treatment::On => b"HOME=/h\0".to_vec(),
            })
        }
    };
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
    .with_environ_reader(Box::new(env));
    let outcome = run_to_end(&mut runner, &study);
    let live = judge(&study, &outcome);
    assert_eq!(live.result, StudyResult::Unqualified, "{live:?}");
    Run {
        root,
        parent,
        study,
        reason: live.reason.to_owned(),
    }
}

fn replay(run: &Run) -> anyhow::Result<()> {
    let dir = File::open(
        run.root
            .path()
            .join("studies")
            .join(&run.study.study_sha256[..16]),
    )
    .unwrap();
    let uid = crate::syscall::geteuid();
    let report = serde_json::to_value(crate::autotune::research::report_store::load_and_verify(
        &dir, uid,
    )?)?;
    let disposition = Disposition {
        study_sha256: run.study.study_sha256.clone(),
        profile: run.study.profile,
        stage: run.study.stage,
        result: Some(StudyResult::Unqualified),
        reason: run.reason.clone(),
    };
    crate::autotune::research::child_judgement::verify_qualification(
        &dir,
        uid,
        &run.parent,
        &disposition,
        &report,
        false,
        false,
    )
}

#[test]
fn an_unqualified_feature_reason_is_replayed_from_its_recorded_observation() {
    let run = run_feature();
    // The treatments reached the game and both ran; the game reports nothing for this factor.
    assert_eq!(run.reason, "FeatureNotObservable");
    replay(&run).unwrap();
}

#[test]
fn a_receipt_that_is_missing_or_contradicts_the_schedule_makes_the_replay_disagree() {
    let run = run_feature();
    let receipt = run
        .root
        .path()
        .join("studies")
        .join(&run.study.study_sha256[..16])
        .join(crate::autotune::research::init_receipt::FEATURE_RECEIPT);
    let original = std::fs::read(&receipt).unwrap();
    // One attempt claims a treatment its schedule did not give it.
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["attempts"][0]["treatment"] = serde_json::json!("on");
    value["attempts"][1]["treatment"] = serde_json::json!("on");
    std::fs::set_permissions(&receipt, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(replay(&run).is_err());
    // No receipt at all proves nothing.
    std::fs::remove_file(&receipt).unwrap();
    assert!(replay(&run).is_err());
}
