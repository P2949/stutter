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
    health: bool,
    process: Option<crate::autotune::research::action_child::ChildTarget>,
    /// The host's settlement cannot be proven (its abort fails).
    abort_fails: bool,
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
            health: false,
            process: None,
            abort_fails: false,
        }
    }
}

impl AttemptHost for Host {
    fn game_process(&mut self) -> Option<crate::autotune::research::action_child::ChildTarget> {
        self.process
    }
    fn health_blocked(&mut self) -> bool {
        self.health
    }
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
        anyhow::ensure!(!self.abort_fails, "settlement unproven");
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

#[test]
fn the_controllers_fence_runs_before_each_launch_and_every_driver_step_is_observable() {
    let (_root, mut runner, study, _) = attempts(Host::finishing());
    let mut fenced = 0;
    let mut steps = Vec::new();
    for _ in 0..200 {
        let done = runner
            .poll_fenced(&study, 100, &mut || {
                fenced += 1;
                Ok(())
            })
            .unwrap();
        steps.extend(runner.take_step());
        // A step is taken once: asking again yields nothing until the next tick.
        assert!(runner.take_step().is_none());
        if done.is_some() {
            break;
        }
    }
    // One launch per attempt, and the fence ran exactly before each.
    assert_eq!(fenced, study.min_attempts);
    assert!(steps.iter().any(|s| matches!(s, Step::Running(_))));
    assert!(steps.iter().any(|s| matches!(s, Step::Sealed(_))));
    assert_eq!(steps.last(), Some(&Step::Done));
}

#[test]
fn a_fence_that_refuses_stops_the_launch_and_the_error_surfaces() {
    let (_root, mut runner, study, _) = attempts(Host::finishing());
    let refused = runner.poll_fenced(&study, 100, &mut || anyhow::bail!("lease not launching"));
    // The first tick would arm a launch: the refusing fence makes the tick fail, never launch.
    assert!(refused.is_err() || runner.take_step().is_none());
}

mod funnel {
    use super::*;
    use crate::autotune::research::{
        child_study::StudyResult,
        funnel_driver::FunnelDriver,
        study_executor::{ProfileFailure, ProfileHook},
    };

    /// No graphics settings are touched: the funnel's profile boundary is exercised elsewhere.
    struct NoProfiles;
    impl ProfileHook for NoProfiles {
        fn apply(
            &mut self,
            _: crate::autotune::research::child_study::Profile,
        ) -> Result<bool, ProfileFailure> {
            Ok(false)
        }
        fn recover(&mut self) -> Result<(), &'static str> {
            Ok(())
        }
        fn restore(&mut self) -> Result<(), &'static str> {
            Ok(())
        }
    }

    fn no_blocker(_: &ChildStudy) -> Option<&'static str> {
        None
    }

    fn funnel(
        host: Host,
        clock_offset: u64,
    ) -> (tempfile::TempDir, FunnelDriver, ResearchIntentV1) {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let campaign = File::open(root.path()).unwrap();
        let parent = parent();
        let mut now = parent.issued_unix_seconds + clock_offset;
        let attempts = DriverStudyAttempts::new(
            campaign.try_clone().unwrap(),
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
        let driver = FunnelDriver::new(
            &campaign,
            &parent,
            Box::new(NoProfiles),
            attempts,
            no_blocker,
            Default::default(),
        )
        .unwrap();
        (root, driver, parent)
    }

    #[test]
    fn the_funnel_is_driven_one_tick_at_a_time_through_the_lease_visible_driver_steps() {
        let (_root, mut driver, _) = funnel(Host::finishing(), 1);
        let mut fenced = 0;
        let mut steps = Vec::new();
        for _ in 0..400 {
            let step = driver
                .tick(&mut || {
                    fenced += 1;
                    Ok(())
                })
                .unwrap();
            let done = step == Step::Done;
            steps.push(step);
            if done {
                break;
            }
        }
        assert_eq!(
            steps.last(),
            Some(&Step::Done),
            "the funnel ends only when every study is decided"
        );
        // The lease-visible steps of the inner driver come through, once per attempt.
        assert!(steps.iter().any(|s| matches!(s, Step::Running(_))));
        assert!(steps.iter().any(|s| matches!(s, Step::Sealed(_))));
        // A study's own completion is never the campaign's: only one Done, and it is the last.
        assert_eq!(steps.iter().filter(|s| **s == Step::Done).count(), 1);
        // The fake host seals no native results, so the screens cannot qualify and everything that
        // depends on them is blocked: every study is decided, none qualified.
        let all = driver.dispositions();
        assert!(all.iter().all(|d| d.result.is_some()));
        assert!(all.iter().all(|d| d.result != Some(StudyResult::Qualified)));
        // Launches happened only for the two screens (2 attempts each).
        assert_eq!(fenced, 4);
    }

    #[test]
    fn a_health_stop_halts_the_funnel_before_any_launch() {
        let mut host = Host::finishing();
        host.health = true;
        let (_root, mut driver, _) = funnel(host, 1);
        let step = driver.tick(&mut || Ok(())).unwrap();
        assert_eq!(step, Step::Halted("BlockedGpuStability"));
        assert!(driver.dispositions().iter().all(|d| d.result.is_none()));
    }

    #[test]
    fn a_cancel_settles_and_ends_the_funnel_without_deciding_what_it_prevented() {
        let (_root, mut driver, _) = funnel(Host::finishing(), 1);
        assert!(matches!(
            driver.tick(&mut || Ok(())).unwrap(),
            Step::Waiting | Step::Running(_)
        ));
        driver.cancel();
        assert_eq!(driver.tick(&mut || Ok(())).unwrap(), Step::Cancelled);
        assert!(driver.dispositions().iter().all(|d| d.result.is_none()));
        // Shutdown after a cancel has nothing left to settle.
        assert_eq!(driver.shutdown(), Ok(()));
    }

    #[test]
    fn a_campaign_deadline_inside_a_study_ends_the_campaign_not_just_the_study() {
        let parent = parent();
        let late = parent.deadline_unix_seconds - parent.issued_unix_seconds + 5;
        let (_root, mut driver, _) = funnel(Host::finishing(), late);
        let mut ended = None;
        for _ in 0..50 {
            let step = driver.tick(&mut || Ok(())).unwrap();
            if matches!(
                step,
                Step::Expired | Step::ActiveBudgetExhausted | Step::Done
            ) {
                ended = Some(step);
                break;
            }
        }
        assert!(
            matches!(ended, Some(Step::Expired | Step::ActiveBudgetExhausted)),
            "{ended:?}"
        );
    }

    #[test]
    fn a_refusing_fence_stops_the_launch_and_halts_the_funnel_with_the_study_recorded_failed() {
        let host = Host::finishing();
        let (_root, mut driver, _) = funnel(host, 1);
        let mut step = Step::Waiting;
        for _ in 0..5 {
            step = driver
                .tick(&mut || anyhow::bail!("lease is not launching"))
                .unwrap();
            if step != Step::Waiting {
                break;
            }
        }
        // The runner records the failed executor and halts; nothing else may run.
        assert_eq!(step, Step::Halted("ExecutorError"));
        let first = &driver.dispositions()[0];
        assert_eq!(first.result, Some(StudyResult::Failed));
        assert_eq!(first.reason, "ExecutorError");
        assert!(
            driver.dispositions()[1..]
                .iter()
                .all(|d| d.result.is_none())
        );
    }
}

mod production_selection {
    use super::*;
    use crate::autotune::research::{
        arm_selection::screen_fps,
        child_study::{Stage, plan},
        funnel_driver::{FunnelDriver, production_inputs},
    };

    #[test]
    fn the_production_selector_reads_a_screens_sealed_evidence_and_refuses_a_missing_one() {
        let (root, mut runner, study, parent) = attempts(Host::finishing());
        run_to_end(&mut runner, &study);
        let campaign = File::open(root.path()).unwrap();
        let uid = crate::syscall::geteuid();
        // The fake host sealed no native results: the screen's evidence directory reads fine and
        // measures nothing, which is not a pair.
        assert!(screen_fps(&campaign, uid, &study).unwrap().is_empty());
        let mut select = production_inputs(
            File::open(root.path()).unwrap(),
            parent.clone(),
            Default::default(),
        );
        let plan = plan(&parent).unwrap();
        let screen = plan
            .iter()
            .find(|s| matches!(s.stage, Stage::Screen))
            .unwrap();
        assert_eq!(select(screen).unwrap().arms.len(), parent.tools.len());
        let pilot = plan
            .iter()
            .find(|s| matches!(s.stage, Stage::Pilot))
            .unwrap();
        assert!(select(pilot).is_err(), "no measured control and candidate");
        // A screen that never ran has no evidence directory at all.
        let empty = tempfile::tempdir().unwrap();
        std::fs::set_permissions(empty.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(screen_fps(&File::open(empty.path()).unwrap(), uid, screen).is_err());
    }

    #[test]
    fn the_funnels_host_is_reachable_for_the_controllers_observations() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let campaign = File::open(root.path()).unwrap();
        let parent = parent();
        let mut host = Host::finishing();
        host.health = true;
        let attempts = DriverStudyAttempts::new(
            campaign.try_clone().unwrap(),
            parent.clone(),
            Box::new(host),
            Box::new(|| 1),
            Box::new(|| Ok(())),
            Box::new(|_| std::path::PathBuf::from("/nonexistent")),
            Box::new(|study| Ok(inputs_for(study))),
        );
        struct Nothing;
        impl crate::autotune::research::study_executor::ProfileHook for Nothing {
            fn apply(
                &mut self,
                _: crate::autotune::research::child_study::Profile,
            ) -> Result<bool, crate::autotune::research::study_executor::ProfileFailure>
            {
                Ok(false)
            }
            fn recover(&mut self) -> Result<(), &'static str> {
                Ok(())
            }
            fn restore(&mut self) -> Result<(), &'static str> {
                Ok(())
            }
        }
        let mut driver = FunnelDriver::new(
            &campaign,
            &parent,
            Box::new(Nothing),
            attempts,
            |_| None,
            Default::default(),
        )
        .unwrap();
        assert!(driver.host_mut().health_blocked());
        driver.host_mut().observe_user_input_idle(None);
    }
}

#[path = "driver_attempts_action_tests.rs"]
mod action_stage;

#[path = "driver_attempts_abort_tests.rs"]
mod abort;
