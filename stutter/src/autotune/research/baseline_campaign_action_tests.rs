//! The production-composed synthetic campaign with an authorized action: the funnel runs the screens, pilots,
//! confirmations and holdouts over real native files, then the action study over the long-session host. The
//! controller either never applies the action (a justified abstention) or applies it in every treated attempt with a
//! sealed measurement (a qualified action, or a worse one). Each record is
//! replayed by `verify_children` (the abstention replay, the budget, the raw archive of the other children), the
//! overall verdict resolves it, and G5 needs the owned-pressure control beside it.

use std::os::unix::fs::PermissionsExt;

use super::{
    baseline_campaign::{NativeHost, NoProfiles},
    *,
};
use crate::autotune::research::{
    ResearchAction,
    action_gate::{ActionGate, KernelState},
    child_study::{Stage, StudyResult},
    funnel_driver::{FunnelDriver, RuleRegistry, production_inputs},
    long_host::LongAttemptHost,
    long_receipt::{LongProgress, LongSessionDriver, LongSessionRecord, Phase, SessionProcess},
    long_session::LongWorkUnit,
};

fn unit() -> LongWorkUnit {
    LongWorkUnit {
        id: "save-a-route-1".into(),
        duration_seconds: 1200,
        definition_sha256: "a".repeat(64),
    }
}

/// A route that completes on its second look, as a settled session of the authorized unit. It plays on the
/// campaign's own clock: completing the route is twenty minutes passing on that clock, so the production accounting
/// measures what a real route would take.
struct Route {
    polls: u32,
    started: u64,
    clock: std::sync::Arc<std::sync::atomic::AtomicU64>,
    sighted: Option<SessionProcess>,
    /// The controller that applies the action in every attempt the schedule treats, when the campaign has one.
    acting: Option<Acting>,
}

/// The normal controller and the resident collectors of a campaign whose action is applied: it applies the action once
/// in each attempt the gate allows, restores it, and seals the measurement its collectors counted.
struct Acting {
    gate: crate::autotune::research::action_gate::SharedActionGate,
    tokens: std::sync::Arc<std::sync::atomic::AtomicU32>,
    acted_at: std::sync::Arc<std::sync::atomic::AtomicU64>,
    campaign: std::path::PathBuf,
    /// Each finished session in order: the process it ran as and its interval.
    history: Vec<(SessionProcess, u64, u64)>,
    /// How much faster the candidate's tail is than the baseline's in the sealed measurement.
    on_tail_ns: u64,
    /// Whether the controller applies the action (else the planner never proposes it and the study abstains).
    applies: bool,
    /// The controller's own journal of every transaction so far, as the resident runtime archives it.
    journal: Vec<serde_json::Value>,
    /// The privileged worker's own audit events of those transactions, as the resident runtime archives them.
    worker_export: crate::autotune::research::evidence_producers::WorkerExport,
}

impl Acting {
    fn act(&mut self, process: SessionProcess, now: u64) {
        if !self.applies {
            return;
        }
        use std::sync::atomic::Ordering::SeqCst;
        self.acted_at.store(now, SeqCst);
        GATE_READS.with(|reads| reads.set(0));
        let target = crate::autotune::research::action_child::ChildTarget {
            pid: process.pid,
            start_ticks: process.start_ticks,
            uid: process.uid,
        };
        let mut gate = self.gate.lock().unwrap();
        if gate.permit(Some(ResearchAction::CpuAffinityProfile), target, now) {
            self.tokens.store(1, SeqCst);
            gate.observe_holdings(
                true,
                0,
                vec![crate::autotune::research::action_gate::Holding {
                    experiment_id: stutter_core::ids::ExperimentId::new(format!(
                        "exp-{:?}",
                        process.pid
                    )),
                    action: Some(ResearchAction::CpuAffinityProfile),
                    pid: Some(process.pid),
                }],
            );
            self.tokens.store(0, SeqCst);
            gate.observe_runtime(false, 0);
            drop(gate);
            self.journal.push(
                crate::autotune::research::evidence_producers::controller_journal_record(
                    &format!("exp-{:?}", process.pid),
                    "cpu-affinity-profile:game-main",
                    (process.pid.as_u32(), process.start_ticks),
                    (now, now + 1),
                ),
            );
            let file = self
                .running_study()
                .join(crate::autotune::research::controller_provenance::FILE);
            std::fs::write(&file, serde_json::to_vec(&self.journal).unwrap()).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
            // The worker issued, applied and restored a transaction for this very process, and exports it.
            self.worker_export.transaction(
                &crate::autotune::research::evidence_producers::worker_transaction_of(&format!(
                    "exp-{:?}",
                    process.pid
                )),
                "cpu-affinity-profile:game-main",
                (process.pid.as_u32(), process.start_ticks),
                (now, now + 1, now + 1),
            );
            let file = self
                .running_study()
                .join(crate::autotune::research::worker_provenance::FILE);
            std::fs::write(
                &file,
                serde_json::to_vec(self.worker_export.records()).unwrap(),
            )
            .unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    /// The study being run: the one beside which the sealed records go.
    fn running_study(&self) -> std::path::PathBuf {
        std::fs::read_dir(self.campaign.join("studies"))
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.join("intent.json").is_file() && !path.join("report.json").is_file())
            .expect("the running study")
    }

    /// What the planner saw in each session: it evaluated the game throughout and never proposed the action.
    fn seal_planner_observation(&self) {
        let attempts: Vec<_> = self
            .history
            .iter()
            .enumerate()
            .map(|(slot, (process, started, ended))| {
                serde_json::json!({
                    "slot": slot,
                    "process": { "pid": process.pid, "start_ticks": process.start_ticks, "uid": process.uid },
                    "start_unix": started + 10, "end_unix": ended - 10,
                    "evaluations": 600, "candidates": 0, "lost": 0, "source_quality": "direct",
                })
            })
            .collect();
        let file = self
            .running_study()
            .join(crate::autotune::research::planner_observation::FILE);
        std::fs::write(
            &file,
            serde_json::to_vec(&serde_json::json!({
                "schema": 1, "action": "CpuAffinityProfile", "attempts": attempts,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        // The raw windows the observation is checked against: what the runtime counted of each session.
        let raw: Vec<_> = self
            .history
            .iter()
            .enumerate()
            .map(|(slot, (process, started, ended))| {
                super::latency_fixtures::raw_planner_window(
                    (process.pid, process.start_ticks, process.uid),
                    slot as u32,
                    started + 10,
                    ended - 10,
                )
            })
            .collect();
        let windows = self
            .running_study()
            .join(crate::autotune::research::resident_observer::WINDOWS_FILE);
        std::fs::write(&windows, serde_json::to_vec(&raw).unwrap()).unwrap();
        std::fs::set_permissions(&windows, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// The latency record the collectors counted over the study's eight sessions, sealed beside the running study: each
    /// attempt's window is its own epochs, cut over the stretch the gate holds the action (shown held ten seconds into the
    /// session, restored 300 s later), the same cut the production sealing and the replay make.
    fn seal_measurement(&self) {
        use crate::autotune::research::{
            action_child::ChildTarget,
            action_latency,
            child_verify::AttemptProcess,
            treatment::{Treatment, treatments_of},
        };
        let dir = self.running_study();
        let intent: ResearchIntentV1 =
            serde_json::from_slice(&std::fs::read(dir.join("intent.json")).unwrap()).unwrap();
        let schedule = treatments_of(&intent).unwrap().unwrap();
        let processes: Vec<AttemptProcess> = (0..schedule.len())
            .map(|slot| {
                let (session, started, ended) = &self.history[slot];
                AttemptProcess {
                    slot: slot as u32,
                    pid: session.pid,
                    start_ticks: session.start_ticks,
                    uid: session.uid,
                    started_unix: *started,
                    completed_unix: *ended,
                }
            })
            .collect();
        let receipts: Vec<serde_json::Value> = processes
            .iter()
            .filter(|p| schedule[p.slot as usize] == Treatment::On)
            .map(|p| {
                serde_json::json!({ "pid": p.pid, "start_ticks": p.start_ticks,
                    "applied_unix": p.started_unix + 11, "settled_unix": p.started_unix + 311 })
            })
            .collect();
        let windows: Vec<_> = processes
            .iter()
            .map(|p| {
                let tail = if schedule[p.slot as usize] == Treatment::On {
                    self.on_tail_ns
                } else {
                    20_000_000
                };
                super::latency_fixtures::fixture_window(
                    ChildTarget {
                        pid: p.pid,
                        start_ticks: p.start_ticks,
                        uid: p.uid,
                    },
                    p.slot,
                    p.started_unix,
                    p.completed_unix,
                    tail,
                )
            })
            .collect();
        let bytes = crate::autotune::research::driver_attempts::resident::latency_document(
            &schedule, &processes, &receipts, &windows,
        )
        .expect("the collectors' windows make a valid record");
        super::latency_fixtures::write_guard_events(&dir, &windows);
        let raw: Vec<_> = windows
            .iter()
            .map(crate::autotune::research::resident_observer::window_record)
            .collect();
        for (name, content) in [
            (
                crate::autotune::research::resident_observer::WINDOWS_FILE,
                serde_json::to_vec(&raw).unwrap(),
            ),
            (action_latency::FILE, bytes),
        ] {
            let file = dir.join(name);
            std::fs::write(&file, content).unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

impl LongSessionDriver for Route {
    fn observe_process(&mut self, process: Option<SessionProcess>) {
        self.sighted = process;
    }
    fn start(&mut self, _: &LongWorkUnit) -> Result<(), &'static str> {
        self.polls = 0;
        self.started = self.clock.load(std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn poll(&mut self) -> LongProgress {
        self.polls += 1;
        if self.polls < 2 {
            if let (Some(acting), Some(process)) = (self.acting.as_mut(), self.sighted) {
                acting.act(process, self.started + 10);
            }
            // The route runs for twenty minutes; the controller reads the clock again only on its next look.
            self.clock.fetch_add(
                u64::from(unit().duration_seconds),
                std::sync::atomic::Ordering::SeqCst,
            );
            return LongProgress::Running;
        }
        let (started, ended) = (
            self.started,
            self.started + u64::from(unit().duration_seconds),
        );
        let process = self.sighted.unwrap_or(SessionProcess {
            pid: stutter_core::ids::Pid::new(4242),
            start_ticks: 99,
            uid: crate::syscall::geteuid(),
        });
        if let Some(acting) = self.acting.as_mut() {
            acting.history.push((process, started, ended));
            if acting.history.len() == 8 {
                if acting.applies {
                    acting.seal_measurement();
                } else {
                    acting.seal_planner_observation();
                }
            }
        }
        LongProgress::Completed(Box::new(LongSessionRecord {
            unit_id: unit().id,
            definition_sha256: unit().definition_sha256,
            started_unix: started,
            ended_unix: ended,
            process,
            phases: vec![Phase {
                name: "route".into(),
                started_unix: started,
                ended_unix: ended,
            }],
            max_poll_gap_seconds: Some(1),
            settled: true,
        }))
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

struct Campaign {
    _root: tempfile::TempDir,
    results: tempfile::TempDir,
    driver: FunnelDriver,
    parent: ResearchIntentV1,
    campaign: File,
}

thread_local! {
    /// How many times the gate has read its clock since the attempt's action began.
    static GATE_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

thread_local! {
    /// The active-time authorization of the synthetic campaign (a test lowers it to exercise the reservation).
    pub(super) static ACTIVE_SECONDS: std::cell::Cell<u32> = const { std::cell::Cell::new(28_000) };
}

fn campaign_with_an_action() -> Campaign {
    build_campaign(None, None)
}

/// The same campaign whose controller applies the action in every attempt the gate allows, with the candidate's
/// runnable tail at `on_tail_ns` against the baseline's 20 ms in the measurement the collectors seal.
fn campaign_that_acts(on_tail_ns: u64) -> Campaign {
    build_campaign(Some(on_tail_ns), None)
}

fn build_campaign(
    acting: Option<u64>,
    factor: Option<crate::autotune::research::ResearchFactor>,
) -> Campaign {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let results = tempfile::tempdir().unwrap();
    std::fs::set_permissions(results.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let mut parent = parent();
    parent.max_evidence_bytes = 256 * 1024 * 1024;
    parent.deadline_unix_seconds = parent.issued_unix_seconds + 40_000;
    parent.max_active_seconds = ACTIVE_SECONDS.with(std::cell::Cell::get);
    parent.max_attempts = 400;
    parent.actions = vec![ResearchAction::CpuAffinityProfile];
    parent.factors = factor.into_iter().collect();
    parent.long_attempt_seconds = Some(1800);
    parent.long_work_unit = Some(unit());
    parent.validate().unwrap();
    let rules: RuleRegistry = Default::default();
    let host = NativeHost {
        settings_root: None,
        results: results.path().to_path_buf(),
        intent: parent.clone(),
        current: None,
        launches: 0,
        cold_until_initialized: false,
        initialized: false,
        initializing: false,
        warms_prefix: true,
        initialization_launches: 0,
        screens_before_init: 0,
        processes: acting.is_some() || factor.is_some(),
        scheduled: None,
        treatment_now: Default::default(),
    };
    let treatment_now = host.treatment_now.clone();
    let tokens = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let acted_at = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (held, at) = (tokens.clone(), acted_at.clone());
    let uid = crate::syscall::geteuid();
    let gate: crate::autotune::research::action_gate::SharedActionGate =
        std::sync::Arc::new(std::sync::Mutex::new(
            ActionGate::unarmed(Box::new(move || {
                Some(held.load(std::sync::atomic::Ordering::SeqCst))
            }))
            // The first read is when the action is shown held, the next when its restoration is accepted, 300 s later.
            .with_clock(Box::new(move || {
                at.load(std::sync::atomic::Ordering::SeqCst)
                    + 1
                    + 300
                        * GATE_READS.with(|reads| {
                            let n = reads.get();
                            reads.set(n + 1);
                            n.min(1)
                        })
            }))
            .with_state_probe(Box::new(move |pid| {
                Some(KernelState {
                    cpus_allowed: "0-15".into(),
                    cgroup: "0::/app.slice".into(),
                    tasks: [(pid, "0-15".to_owned())].into(),
                    identity: Some((u64::from(pid) - 5_800, uid)),
                })
            })),
        ));
    let applies = acting.is_some();
    let acting = Some(Acting {
        gate: gate.clone(),
        tokens,
        acted_at,
        campaign: root.path().to_path_buf(),
        history: Vec::new(),
        on_tail_ns: acting.unwrap_or(0),
        applies,
        journal: Vec::new(),
        worker_export: Default::default(),
    });
    let results_root = results.path().to_path_buf();
    let clock = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
        parent.issued_unix_seconds + 1,
    ));
    let shared = clock.clone();
    let attempts = DriverStudyAttempts::new(
        campaign.try_clone().unwrap(),
        parent.clone(),
        Box::new(LongAttemptHost::new(
            host,
            Route {
                polls: 0,
                started: 0,
                clock,
                sighted: None,
                acting,
            },
        )),
        Box::new(move || shared.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1),
        Box::new(|| Ok(())),
        Box::new(move |source| results_root.join(source.dir_name())),
        production_inputs(campaign.try_clone().unwrap(), parent.clone(), rules.clone()),
    )
    .with_action_gate(gate);
    // A feature campaign: the game shows the treatment's environment and the kernel's witness of the factor's backend.
    let attempts = if factor.is_some() {
        let (env_now, witness_now) = (treatment_now.clone(), treatment_now);
        attempts
            .with_environ_reader(Box::new(move |_| {
                let on = *env_now.lock().unwrap() == Some(crate::autotune::research::Treatment::On);
                Some(if on {
                    b"HOME=/h\0".to_vec()
                } else {
                    b"PROTON_NO_NTSYNC=1\0".to_vec()
                })
            }))
            .with_witness_reader(Box::new(move |_, _| {
                let on =
                    *witness_now.lock().unwrap() == Some(crate::autotune::research::Treatment::On);
                // The fixture's scan read the whole process, as a real one does.
                let mut witness = crate::autotune::research::feature_proof::Witness {
                    complete: true,
                    ..Default::default()
                };
                if on {
                    witness.devices.insert("/dev/ntsync".to_owned());
                }
                Some(witness)
            }))
    } else {
        attempts
    };
    let driver = FunnelDriver::new(
        &campaign,
        &parent,
        Box::new(NoProfiles {
            digest: Some("c".repeat(64)),
            campaign_tools: parent.tools.len(),
            arms: 0,
        }),
        attempts,
        |_| None,
        rules,
    )
    .unwrap();
    Campaign {
        _root: root,
        results,
        driver,
        parent,
        campaign,
    }
}

fn run(campaign: &mut Campaign) {
    use crate::autotune::research::driver::Step;
    for _ in 0..20_000 {
        match campaign.driver.tick(&mut || Ok(())).unwrap() {
            Step::Done => return,
            Step::Stopped(_) | Step::Halted(_) | Step::OwnershipUnresolved(_) => {
                panic!("the campaign stopped")
            }
            _ => {}
        }
    }
    panic!("the campaign never ended");
}

#[test]
fn an_action_the_controller_never_applied_is_a_justified_abstention_through_the_whole_campaign() {
    let mut run_of = campaign_with_an_action();
    run(&mut run_of);
    let dispositions = run_of.driver.dispositions();
    let action = dispositions
        .iter()
        .find(|d| matches!(d.stage, Stage::Action(_)))
        .expect("the action study is planned");
    assert_eq!(
        (action.result, action.reason.as_str()),
        (Some(StudyResult::Unqualified), "ActionNotIndicated"),
        "{dispositions:?}"
    );
    // Everything else the funnel planned qualified.
    assert!(
        dispositions
            .iter()
            .filter(|d| !matches!(d.stage, Stage::Action(_)))
            .all(|d| d.result == Some(StudyResult::Qualified)),
        "{dispositions:?}"
    );
    // The independent verification replays every child, including the abstention (its own record, no receipts).
    let results = run_of.results.path().to_path_buf();
    let verified = crate::autotune::research::child_verify::verify_children(
        &run_of.campaign,
        crate::syscall::geteuid(),
        &run_of.parent,
        &move |source| results.join(source.dir_name()),
    )
    .unwrap();
    assert!(
        verified.children.iter().all(|c| c.problem.is_none()),
        "{:?}",
        verified.children
    );
    assert!(
        verified.all_verified(),
        "missing {:?} budget {:?} {:?}",
        verified.missing,
        verified.budget_problem,
        verified
            .children
            .iter()
            .map(|c| (c.study.clone(), c.reconstruction, c.raw_files_verified))
            .collect::<Vec<_>>()
    );
    // A justified abstention applied nothing, so nothing is left to restore: the restoration check accepts it.
    assert!(verified.actions_restored(&dispositions));
    // And a campaign without the abstention record is refused at replay.
    let path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run_of.campaign)
    ))
    .unwrap();
    let study = std::fs::read_dir(path.join("studies"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.join(crate::autotune::research::action_coverage::ABSTENTION_RECORD)
                .is_file()
        })
        .expect("the abstention record");
    std::fs::remove_file(study.join(crate::autotune::research::action_coverage::ABSTENTION_RECORD))
        .unwrap();
    let results = run_of.results.path().to_path_buf();
    let tampered = crate::autotune::research::child_verify::verify_children(
        &run_of.campaign,
        crate::syscall::geteuid(),
        &run_of.parent,
        &move |source| results.join(source.dir_name()),
    )
    .unwrap();
    assert!(tampered.children.iter().any(|c| c.problem.is_some()));
}

#[test]
fn a_budget_record_that_understates_the_witnessed_sessions_is_refused_at_verification() {
    let mut run_of = campaign_with_an_action();
    run(&mut run_of);
    let path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run_of.campaign)
    ))
    .unwrap();
    let study = std::fs::read_dir(path.join("studies"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.join(crate::autotune::research::action_coverage::ABSTENTION_RECORD)
                .is_file()
        })
        .expect("the action study");
    let verify = |run_of: &Campaign| {
        let results = run_of.results.path().to_path_buf();
        crate::autotune::research::child_verify::verify_children(
            &run_of.campaign,
            crate::syscall::geteuid(),
            &run_of.parent,
            &move |source| results.join(source.dir_name()),
        )
        .unwrap()
    };
    // As the campaign recorded it, the accounting is whole.
    assert!(verify(&run_of).budget_problem.is_none());
    // Lowering the active time the long child recorded below what its sealed sessions witnessed is caught.
    let budget = study.join("budget.json");
    let mut spent: crate::autotune::research::campaign_budget::Spent =
        serde_json::from_slice(&std::fs::read(&budget).unwrap()).unwrap();
    spent.active_seconds = 60;
    std::fs::set_permissions(&budget, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&budget, serde_json::to_vec(&spent).unwrap()).unwrap();
    let verified = verify(&run_of);
    assert!(
        verified
            .budget_problem
            .as_deref()
            .is_some_and(|problem| problem.starts_with("BudgetActiveSecondsUnderstated")),
        "{:?}",
        verified.budget_problem
    );
}

fn study_with(run_of: &Campaign, file: &str) -> std::path::PathBuf {
    let path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run_of.campaign)
    ))
    .unwrap();
    std::fs::read_dir(path.join("studies"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join(file).is_file())
        .expect("the action study")
}

fn verify_all(run_of: &Campaign) -> crate::autotune::research::child_verify::ChildrenVerification {
    let results = run_of.results.path().to_path_buf();
    crate::autotune::research::child_verify::verify_children(
        &run_of.campaign,
        crate::syscall::geteuid(),
        &run_of.parent,
        &move |source| results.join(source.dir_name()),
    )
    .unwrap()
}

fn rewrite(path: &std::path::Path, bytes: &[u8]) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[path = "baseline_campaign_graphics_tests.rs"]
mod graphics;
#[path = "baseline_campaign_qualified_tests.rs"]
mod qualified;
