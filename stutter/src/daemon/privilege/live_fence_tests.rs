//! Live qualification of the validation fence against a real, isolated, transient root worker.
//!
//! Ignored by default: it needs a worker started by the qualification procedure (see
//! `docs/validation/worker-fence-qualification-2026-11-09/RESULTS.md`) and two disposable processes
//! of the same account: the fixture the fence names and a decoy it does not. It only sends the
//! worker read-only requests and dry runs, so nothing is mutated.
use super::*;
use crate::autotune::{objective::ObjectiveKind, planning::candidate::CandidateAction};

fn env_number(name: &str) -> u32 {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set"))
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not a number"))
}

fn dry_run(client: &UnixSocketPrivilegedActionService, pid: u32) -> anyhow::Result<()> {
    let candidate = CandidateAction::cpu_affinity_profile_for(
        crate::profiles::Profile {
            name: "live-fence".into(),
            rules: vec![],
        },
        pid,
        ObjectiveKind::GameRunnableLatency,
    );
    client
        .dry_run_candidate(CandidateApplyRequest {
            plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
            policy: crate::daemon_policy::DaemonPolicy::apply_medium_risk(
                crate::daemon_policy::ActionSource::Test,
            ),
            context: Default::default(),
            max_plan_age_nanos: 60_000_000_000,
        })
        .map(|_| ())
}

#[test]
#[ignore = "live: needs STUTTER_QUALIFY_SOCKET, STUTTER_QUALIFY_FIXTURE_PID and STUTTER_QUALIFY_DECOY_PID"]
fn a_real_worker_refuses_a_decoy_the_fence_does_not_name() {
    let socket = std::env::var("STUTTER_QUALIFY_SOCKET").expect("STUTTER_QUALIFY_SOCKET");
    let (fixture, decoy) = (
        env_number("STUTTER_QUALIFY_FIXTURE_PID"),
        env_number("STUTTER_QUALIFY_DECOY_PID"),
    );
    let client = UnixSocketPrivilegedActionService::new(socket);
    let report = client
        .fence_status()
        .expect("the worker answers fence_status");
    assert_eq!(report.state, "active", "{report:?}");
    assert_eq!(report.root_pid, Some(fixture), "{report:?}");

    let refused = dry_run(&client, decoy).expect_err("the decoy must be refused");
    let refused = format!("{refused:#}");
    println!("decoy refusal: {refused}");
    assert!(
        refused.contains("validation_fence_target_not_fenced"),
        "{refused}"
    );

    // The fixture is not refused by the fence. Ordinary checks may still refuse it for their own
    // reasons; those are different, and the fence adds nothing to them.
    match dry_run(&client, fixture) {
        Ok(()) => println!("fixture: dry run accepted"),
        Err(error) => {
            let error = format!("{error:#}");
            println!("fixture ordinary refusal: {error}");
            assert!(!error.contains("validation_fence"), "{error}");
        }
    }
}
