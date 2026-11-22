//! The production-composed synthetic campaign with an authorized action: the funnel runs the screens, pilots,
//! confirmations and holdouts over real native files, then the action study over the long-session host. The
//! controller never applies the action (nothing to act on), so the study is a justified abstention. Its record is
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

/// A route that completes on its second look, as a settled session of the authorized unit.
struct Route(u32);
impl LongSessionDriver for Route {
    fn start(&mut self, _: &LongWorkUnit) -> Result<(), &'static str> {
        self.0 = 0;
        Ok(())
    }
    fn poll(&mut self) -> LongProgress {
        self.0 += 1;
        if self.0 < 2 {
            return LongProgress::Running;
        }
        LongProgress::Completed(Box::new(LongSessionRecord {
            unit_id: unit().id,
            definition_sha256: unit().definition_sha256,
            started_unix: 5_000,
            ended_unix: 6_200,
            process: SessionProcess {
                pid: stutter_core::ids::Pid::new(4242),
                start_ticks: 99,
                uid: crate::syscall::geteuid(),
            },
            phases: vec![Phase {
                name: "route".into(),
                started_unix: 5_000,
                ended_unix: 6_200,
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

fn campaign_with_an_action() -> Campaign {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let results = tempfile::tempdir().unwrap();
    std::fs::set_permissions(results.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let campaign = File::open(root.path()).unwrap();
    let mut parent = parent();
    parent.max_evidence_bytes = 256 * 1024 * 1024;
    parent.deadline_unix_seconds = parent.issued_unix_seconds + 40_000;
    parent.max_active_seconds = 28_000;
    parent.max_attempts = 400;
    parent.actions = vec![ResearchAction::CpuAffinityProfile];
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
    };
    let gate: crate::autotune::research::action_gate::SharedActionGate =
        std::sync::Arc::new(std::sync::Mutex::new(
            ActionGate::unarmed(Box::new(|| Some(0))).with_state_probe(Box::new(|_| {
                Some(KernelState {
                    cpus_allowed: "0-15".into(),
                    cgroup: "0::/app.slice".into(),
                    ..Default::default()
                })
            })),
        ));
    let results_root = results.path().to_path_buf();
    let mut now = parent.issued_unix_seconds + 1;
    let attempts = DriverStudyAttempts::new(
        campaign.try_clone().unwrap(),
        parent.clone(),
        Box::new(LongAttemptHost::new(host, Route(0))),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(move |source| results_root.join(source.dir_name())),
        production_inputs(campaign.try_clone().unwrap(), parent.clone(), rules.clone()),
    )
    .with_action_gate(gate);
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
    // The synthetic controller clock ticks one second per read, while a real twenty-minute route witnesses twenty
    // minutes: give the long child the active time its attempts witnessed, as a real clock would have recorded.
    let path = std::fs::read_link(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&run_of.campaign)
    ))
    .unwrap();
    let action_dir = std::fs::read_dir(path.join("studies"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.join(crate::autotune::research::action_coverage::ABSTENTION_RECORD)
                .is_file()
        })
        .expect("the abstention record");
    let budget = action_dir.join("budget.json");
    let mut spent: crate::autotune::research::campaign_budget::Spent =
        serde_json::from_slice(&std::fs::read(&budget).unwrap()).unwrap();
    spent.active_seconds = 8 * 1200;
    std::fs::set_permissions(&budget, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&budget, serde_json::to_vec(&spent).unwrap()).unwrap();
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
