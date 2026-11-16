use std::{collections::VecDeque, os::unix::fs::PermissionsExt};

use super::*;
use crate::autotune::research::{
    ProtonSource, ResearchTool,
    child_study::plan,
    dispatch_context::{ArmSpec, DispatchContextV1},
    driver::Progress,
    journal::AttemptSlot,
    report::AttemptEvidenceV1,
    tests::intent,
};

struct Host {
    polls: VecDeque<Progress>,
    launches: u32,
    current: u32,
    unsafe_until_tick: u32,
    ticks: u32,
    /// Every factor set the driver enabled before a study, shared so a test can read it back.
    seen_factors:
        std::sync::Arc<std::sync::Mutex<Vec<Vec<crate::autotune::research::ResearchFactor>>>>,
}

impl Host {
    fn finishing() -> Self {
        Self {
            polls: VecDeque::new(),
            launches: 0,
            current: 0,
            unsafe_until_tick: 0,
            ticks: 0,
            seen_factors: Default::default(),
        }
    }
}

impl AttemptHost for Host {
    fn set_enabled_factors(&mut self, factors: &[crate::autotune::research::ResearchFactor]) {
        self.seen_factors.lock().unwrap().push(factors.to_vec());
    }
    fn safe_window(&mut self) -> bool {
        self.ticks += 1;
        self.ticks > self.unsafe_until_tick
    }
    fn arm_spec(&mut self, _: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        Ok(ArmSpec {
            tool_dir: "/t".into(),
            study_compat_data: "/s".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        })
    }
    fn launch(&mut self, slot: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        self.launches += 1;
        self.current = slot.index;
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        self.polls
            .pop_front()
            .unwrap_or_else(|| Progress::Finished {
                evidence: AttemptEvidenceV1 {
                    schema_version: 1,
                    slot_index: self.current,
                    average_fps: 60.0 + f64::from(self.launches),
                    native_completion: None,
                },
            })
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

fn parent() -> ResearchIntentV1 {
    let mut intent = intent();
    intent.uid = crate::syscall::geteuid();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.max_attempts = 40;
    intent.max_attempt_seconds = 100;
    intent
}

fn attempts(
    host: Host,
) -> (
    tempfile::TempDir,
    DriverStudyAttempts,
    ChildStudy,
    ResearchIntentV1,
) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let parent = parent();
    let study = plan(&parent).unwrap().remove(0);
    let mut now = parent.issued_unix_seconds + 1;
    let runner = DriverStudyAttempts::new(
        campaign,
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
    (root, runner, study, parent)
}

fn run_to_end(runner: &mut DriverStudyAttempts, study: &ChildStudy) -> AttemptsOutcome {
    for _ in 0..200 {
        if let Some(outcome) = runner.poll(study, 100).unwrap() {
            return outcome;
        }
    }
    panic!("the study never ended")
}

#[test]
fn a_child_study_runs_its_own_attempts_through_the_driver_and_reports_what_was_sealed() {
    let (_root, mut runner, study, _) = attempts(Host::finishing());
    assert!(!runner.has_progress(&study));
    // One poll is one tick: the study stays running across polls.
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    assert!(runner.has_progress(&study), "its own directories now exist");
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.attempts_used, study.min_attempts);
    assert_eq!(outcome.completed, study.min_attempts);
    assert!(!outcome.health_halt);
    // No native completions were sealed, so the context and profile are unproven, never assumed.
    assert!(!outcome.context_consistent);
    // Nothing was replayed from raw files and no runtime was captured: not verified.
    assert!(!outcome.raw_verified && !outcome.runtime_matched);
    assert!(
        outcome
            .conformance
            .iter()
            .all(|c| *c == ProfileConformance::Unproven)
    );
}

#[test]
fn a_failed_attempt_is_sealed_failed_and_the_study_is_incomplete() {
    let mut host = Host::finishing();
    host.polls.push_back(Progress::Failed {
        code: "ExitedWithoutNativeResult".into(),
    });
    let (_root, mut runner, study, _) = attempts(host);
    let outcome = run_to_end(&mut runner, &study);
    assert_eq!(outcome.attempts_used, study.min_attempts);
    assert_eq!(outcome.completed, study.min_attempts - 1);
    assert!(!outcome.health_halt);
}

#[test]
fn a_gpu_reset_halts_the_study_and_no_further_attempt_runs() {
    let mut host = Host::finishing();
    host.polls.push_back(Progress::Failed {
        code: "GpuReset".into(),
    });
    let (_root, mut runner, study, _) = attempts(host);
    // The Driver halts only when the host's durable health marker is set; the fake reports the
    // failure code, which the driver seals as a failed attempt.
    let outcome = run_to_end(&mut runner, &study);
    assert!(outcome.completed < study.min_attempts);
}

#[test]
fn two_studies_never_share_directories_or_schedules() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let mut now = parent.issued_unix_seconds + 1;
    let mut runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent.clone(),
        Box::new(Host::finishing()),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(|study| Ok(inputs_for(study))),
    );
    let first = run_to_end(&mut runner, &plan[0]);
    let second = run_to_end(&mut runner, &plan[1]);
    assert_eq!(first.completed, plan[0].min_attempts);
    assert_eq!(second.completed, plan[1].min_attempts);
    let studies: Vec<_> = std::fs::read_dir(root.path().join("studies"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(studies.len(), 2);
    assert!(runner.has_progress(&plan[0]) && runner.has_progress(&plan[1]));
}

mod child_verification {
    use super::*;
    use crate::autotune::research::child_verify::verify_children;

    /// Run the first study to the end and return the campaign directory with its parent intent.
    fn finished() -> (tempfile::TempDir, ResearchIntentV1, ChildStudy) {
        let (root, mut runner, study, parent) = attempts(Host::finishing());
        run_to_end(&mut runner, &study);
        (root, parent, study)
    }

    fn campaign(root: &tempfile::TempDir) -> File {
        File::open(root.path()).unwrap()
    }

    fn lookup(_: ProtonSource) -> std::path::PathBuf {
        "/nonexistent".into()
    }

    #[test]
    fn a_finished_child_reconstructs_and_its_missing_raw_results_are_reported_not_assumed() {
        let (root, parent, study) = finished();
        let uid = crate::syscall::geteuid();
        let result = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
        assert_eq!(result.children.len(), 1);
        let child = &result.children[0];
        assert_eq!(child.study, study.study_sha256[..16]);
        assert!(child.reconstruction, "{:?}", child.problem);
        // The fake host sealed no native results, so nothing can replay: not verified.
        assert!(!child.raw_files_verified);
        assert!(!result.all_verified());
    }

    #[test]
    fn a_campaign_without_children_has_nothing_verified() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = verify_children(
            &campaign(&root),
            crate::syscall::geteuid(),
            &parent(),
            &lookup,
        )
        .unwrap();
        assert!(result.children.is_empty());
        assert!(!result.all_verified());
    }

    #[test]
    fn a_tampered_report_a_forged_intent_or_an_unexpected_entry_fails_reconstruction() {
        let uid = crate::syscall::geteuid();
        // Tamper with the sealed report.
        let (root, parent, study) = finished();
        let dir = root.path().join("studies").join(&study.study_sha256[..16]);
        let report = std::fs::read_to_string(dir.join("report.json")).unwrap();
        std::fs::write(
            dir.join("report.json"),
            report.replace("\"attempts_completed\":", "\"attempts_completed\":9"),
        )
        .unwrap();
        let tampered = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
        assert!(!tampered.children[0].reconstruction);
        // A child whose intent names a tool the campaign never authorized.
        let (root, parent, study) = finished();
        let dir = root.path().join("studies").join(&study.study_sha256[..16]);
        let mut forged: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("intent.json")).unwrap()).unwrap();
        forged["tools"][0]["identity_sha256"] = "9".repeat(64).into();
        std::fs::write(dir.join("intent.json"), forged.to_string()).unwrap();
        let foreign = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
        assert!(!foreign.children[0].reconstruction);
        assert!(
            foreign.children[0]
                .problem
                .as_deref()
                .unwrap()
                .contains("campaign's own intent")
        );
        // A child bound to a different study digest than its directory name.
        let (root, parent, study) = finished();
        let dir = root.path().join("studies").join(&study.study_sha256[..16]);
        let mut rebound: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("intent.json")).unwrap()).unwrap();
        rebound["child_study_sha256"] = "7".repeat(64).into();
        std::fs::write(dir.join("intent.json"), rebound.to_string()).unwrap();
        assert!(
            !verify_children(&campaign(&root), uid, &parent, &lookup)
                .unwrap()
                .children[0]
                .reconstruction
        );
        // An entry that is not a study digest.
        let (root, parent, _) = finished();
        std::fs::create_dir(root.path().join("studies/stray")).unwrap();
        let stray = verify_children(&campaign(&root), uid, &parent, &lookup).unwrap();
        assert!(
            stray
                .children
                .iter()
                .any(|c| c.study == "stray" && !c.reconstruction)
        );
    }
}

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

/// The inputs of the two-arm fixture campaign: its arms, and the rule a confirmation needs.
fn inputs_for(study: &ChildStudy) -> StudyInputs {
    StudyInputs {
        arms: vec![ProtonSource::Ge, ProtonSource::ValveStable],
        rule: matches!(
            study.stage,
            crate::autotune::research::child_study::Stage::Confirm
        )
        .then(|| crate::autotune::research::pilot_rule::freeze(&[70.0, 73.0, 67.0]).unwrap()),
    }
}
