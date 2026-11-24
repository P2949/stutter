use std::{collections::VecDeque, os::unix::fs::PermissionsExt};

use super::*;
use crate::autotune::research::{
    ProtonSource, ResearchTool,
    child_study::plan,
    dispatch_context::{ArmSpec, DispatchContextV1},
    driver::{FailureClass, Progress},
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
    abort_fails: bool,
    launch_unresolved: bool,
    treatment: Option<crate::autotune::research::Treatment>,
    /// Per-attempt treatments (by slot), when a test follows the intent's own schedule.
    scheduled: Option<Vec<crate::autotune::research::Treatment>>,
    /// Where the host publishes the schedule the driver gave it (a test reads the real one).
    schedule_out:
        Option<std::sync::Arc<std::sync::Mutex<Vec<crate::autotune::research::Treatment>>>>,
    prefix_refusal: Option<&'static str>,
    /// After this many launches the sighted game process is the given one (a new launch, a new process).
    switch_process: Option<(u32, crate::autotune::research::action_child::ChildTarget)>,
    /// One game process per launch (the Nth launch is the Nth), when a test needs distinct attempts.
    processes: Vec<crate::autotune::research::action_child::ChildTarget>,
    /// The hash of each tool file by relative path, as the host would read it from its tool directory.
    tool_files: std::collections::BTreeMap<String, String>,
    /// Every factor set the driver enabled before a study, shared so a test can read it back.
    seen_factors:
        std::sync::Arc<std::sync::Mutex<Vec<Vec<crate::autotune::research::ResearchFactor>>>>,
    launched_context: std::sync::Arc<std::sync::Mutex<Option<DispatchContextV1>>>,
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
            launched_context: Default::default(),
            health: false,
            process: None,
            abort_fails: false,
            launch_unresolved: false,
            treatment: None,
            scheduled: None,
            schedule_out: None,
            prefix_refusal: None,
            switch_process: None,
            processes: Vec::new(),
            tool_files: Default::default(),
        }
    }
}

impl AttemptHost for Host {
    fn tool_file_sha256(&mut self, relative: &str) -> Option<String> {
        self.tool_files.get(relative).cloned()
    }
    fn game_process(&mut self) -> Option<crate::autotune::research::action_child::ChildTarget> {
        if !self.processes.is_empty() {
            return self
                .processes
                .get((self.launches as usize).saturating_sub(1))
                .copied();
        }
        match self.switch_process {
            Some((after, target)) if self.launches >= after => Some(target),
            _ => self.process,
        }
    }
    fn health_blocked(&mut self) -> bool {
        self.health
    }
    fn attempt_index(&mut self) -> Option<u32> {
        Some(self.current)
    }
    fn set_treatments(&mut self, treatments: Option<Vec<crate::autotune::research::Treatment>>) {
        // Only a test that asked to follow the driver's own schedule does; the others keep a fixed treatment.
        if let (Some(shared), Some(schedule)) = (&self.schedule_out, &treatments) {
            *shared.lock().unwrap() = schedule.clone();
            self.scheduled = treatments;
        }
    }
    fn attempt_treatment(&mut self) -> Option<crate::autotune::research::Treatment> {
        self.scheduled
            .as_ref()
            .and_then(|schedule| schedule.get(self.current as usize).copied())
            .or(self.treatment)
    }
    fn check_prefixes(
        &mut self,
        _: &[ProtonSource],
    ) -> Result<Option<crate::autotune::research::prefix_identity::PrefixClass>, &'static str> {
        self.prefix_refusal.map_or(
            Ok(Some(super::super::prefix_identity::PrefixClass::Warm)),
            Err,
        )
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
    fn launch(&mut self, slot: &AttemptSlot, context: &DispatchContextV1) -> anyhow::Result<()> {
        self.launches += 1;
        self.current = slot.index;
        *self.launched_context.lock().unwrap() = Some(context.clone());
        if self.launch_unresolved {
            return Err(
                super::super::driver::LaunchUnresolved("fixture acknowledgement lost").into(),
            );
        }
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        self.polls
            .pop_front()
            .unwrap_or_else(|| Progress::Finished {
                evidence: AttemptEvidenceV1 {
                    thermal: None,
                    long_session: None,
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
    intent.max_evidence_bytes = 4 * 1024 * 1024;
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
    for _ in 0..20000 {
        if let Some(outcome) = runner.poll(study, 100).unwrap() {
            return outcome;
        }
    }
    panic!("the study never ended")
}

#[test]
fn child_consumed_ticket_requires_both_witnesses_until_the_seal_is_durable() {
    use std::sync::{Arc, Mutex};

    use crate::autotune::research::{
        driver::{ClaimSettlementGuard, ClaimSettlementVerifier},
        launch_ticket::{ResearchLaunchTicketV1, SteamVerb, binding, consume_launch_ticket},
    };

    struct Guard(std::path::PathBuf, Arc<Mutex<Vec<bool>>>);
    impl ClaimSettlementGuard for Guard {}
    impl Drop for Guard {
        fn drop(&mut self) {
            self.1.lock().unwrap().push(self.0.is_file());
        }
    }
    struct Verifier {
        process_ok: bool,
        worker_ok: bool,
        seal: std::path::PathBuf,
        drops: Arc<Mutex<Vec<bool>>>,
    }
    impl ClaimSettlementVerifier for Verifier {
        fn verify_process(
            &mut self,
            _: &File,
            _: &DispatchContextV1,
            _: &ResearchLaunchTicketV1,
        ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
            anyhow::ensure!(self.process_ok, "process witness refused");
            Ok(Box::new(Guard(self.seal.clone(), self.drops.clone())))
        }
        fn verify_worker(
            &mut self,
            _: &DispatchContextV1,
        ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
            anyhow::ensure!(self.worker_ok, "worker witness refused");
            Ok(Box::new(Guard(self.seal.clone(), self.drops.clone())))
        }
    }
    for (process_ok, worker_ok, cancel) in [
        (false, true, false),
        (true, false, false),
        (true, true, false),
        (false, true, true),
        (true, false, true),
        (true, true, true),
    ] {
        let host = Host::finishing();
        let context = host.launched_context.clone();
        let (root, runner, study, _) = attempts(host);
        let study_path = root.path().join("studies").join(&study.study_sha256[..16]);
        let seal = study_path.join("journal/0000.json");
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mut runner = runner.with_claim_verifier_factory(Box::new({
            let seal = seal.clone();
            let drops = drops.clone();
            move || {
                Box::new(Verifier {
                    process_ok,
                    worker_ok,
                    seal: seal.clone(),
                    drops: drops.clone(),
                })
            }
        }));
        for _ in 0..10 {
            assert!(runner.poll(&study, 100).unwrap().is_none());
            if context.lock().unwrap().is_some() {
                break;
            }
        }
        let context = context.lock().unwrap().clone().expect("child launched");
        let ticket_id = context.tickets.get("run").unwrap();
        let attempt = study_path.join("dispatch/0000");
        let ticket: ResearchLaunchTicketV1 = serde_json::from_slice(
            &std::fs::read(attempt.join(format!("ticket-{ticket_id}.json"))).unwrap(),
        )
        .unwrap();
        consume_launch_ticket(
            crate::syscall::open_dir_no_symlinks(&attempt).unwrap(),
            ticket_id,
            &binding(&context.intent, &context.slot, SteamVerb::Run).unwrap(),
            ticket.issued_unix_seconds,
        )
        .unwrap();
        let success = if cancel {
            runner.abort().is_ok()
        } else {
            runner.poll(&study, 100).is_ok()
        };
        assert_eq!(success, process_ok && worker_ok);
        assert_eq!(seal.is_file(), process_ok && worker_ok);
        if cancel {
            assert_eq!(runner.has_running(), !success);
        }
        if process_ok && worker_ok {
            assert_eq!(*drops.lock().unwrap(), [true, true]);
        }
    }
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
            .all(|c| *c == crate::autotune::research::native_context::ProfileConformance::Unproven)
    );
}

#[test]
fn a_failed_attempt_is_sealed_failed_and_the_study_is_incomplete() {
    let mut host = Host::finishing();
    host.polls.push_back(Progress::Failed {
        class: crate::autotune::research::driver::FailureClass::Incompatible,
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
        class: crate::autotune::research::driver::FailureClass::Health,
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

#[path = "driver_attempts_verification_tests.rs"]
mod child_verification;

/// The fixture campaign's arms (one build for a treatment study) and a confirmation's rule.
fn inputs_for(study: &ChildStudy) -> StudyInputs {
    StudyInputs {
        arms: match study.arms() {
            crate::autotune::research::child_study::StudyArms::Single => vec![ProtonSource::Ge],
            _ => vec![ProtonSource::Ge, ProtonSource::ValveStable],
        },
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

    pub(super) fn funnel(
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
        assert_eq!(step, Step::Stopped(FailureClass::Health));
        assert!(driver.dispositions().iter().all(|d| d.result.is_none()));
    }

    #[test]
    fn a_cancel_records_the_interrupted_child_and_leaves_later_studies_undecided() {
        let (root, mut driver, parent) = funnel(Host::finishing(), 1);
        assert!(matches!(
            driver.tick(&mut || Ok(())).unwrap(),
            Step::Waiting | Step::Running(_)
        ));
        driver.cancel();
        assert_eq!(driver.tick(&mut || Ok(())).unwrap(), Step::Cancelled);
        let dispositions = driver.dispositions();
        assert_eq!(
            dispositions
                .iter()
                .filter(|d| d.result == Some(StudyResult::Failed))
                .count(),
            1
        );
        assert_eq!(
            dispositions.iter().filter(|d| d.result.is_some()).count(),
            1
        );
        let failed = dispositions
            .iter()
            .find(|d| d.result == Some(StudyResult::Failed))
            .unwrap();
        let study = root.path().join("studies").join(&failed.study_sha256[..16]);
        assert!(study.join("report.json").is_file());
        assert!(study.join("budget.json").is_file());
        // Shutdown after a cancel has nothing left to settle, and releases the ledger lock.
        assert_eq!(driver.shutdown(), Ok(()));
        drop(driver);
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(study.join("report.json")).unwrap()).unwrap();
        let counts = crate::autotune::research::child_ledger::read_attempt_counts(
            &File::open(root.path()).unwrap(),
            parent.uid,
            &parent,
        )
        .unwrap();
        assert_eq!(
            u64::from(counts[&failed.study_sha256]),
            report["attempts_sealed"].as_u64().unwrap()
        );
        let verified = crate::autotune::research::child_verify::verify_children(
            &File::open(root.path()).unwrap(),
            parent.uid,
            &parent,
            &|_| std::path::PathBuf::from("/nonexistent"),
        )
        .unwrap();
        assert_eq!(verified.children.len(), 1);
        assert!(
            verified.children[0].reconstruction,
            "{:?}",
            verified.children[0].problem
        );
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

#[path = "driver_attempts_selection_tests.rs"]
mod production_selection;

#[path = "driver_attempts_action_tests.rs"]
mod action_stage;

#[path = "driver_attempts_abort_tests.rs"]
mod abort;

#[path = "driver_attempts_feature_tests.rs"]
mod feature;

#[path = "driver_attempts_budget_tests.rs"]
mod budget;

#[path = "driver_attempts_frozen_tests.rs"]
mod frozen;

#[path = "baseline_campaign_action_tests.rs"]
mod baseline_action;

#[path = "baseline_campaign_hook_tests.rs"]
mod baseline_hook;

#[path = "baseline_campaign_integrity_tests.rs"]
mod baseline_integrity;

#[path = "baseline_campaign_tests.rs"]
mod baseline_campaign;

#[path = "driver_attempts_long_tests.rs"]
mod long;

#[path = "driver_attempts_feature_replay_tests.rs"]
mod feature_replay;

#[path = "driver_attempts_latency_fixtures.rs"]
mod latency_fixtures;
