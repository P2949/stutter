use std::{
    fs,
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use stutter::api::{
    actions::{NiceAction, NicePolicy, RollbackToken, TaskIdentity},
    autotune::{
        ObjectiveKind,
        candidate::{CandidateAction, CandidateEvidence, NiceActionPlan},
    },
    daemon::{
        ActionSource, CandidateApplyRequest, CandidatePlanRequest, DaemonPolicy,
        DaemonPolicyContext, InProcessPrivilegedActionService, PrivilegedActionService,
        PrivilegedWorkerHandle, RollbackRequest, UnixSocketPrivilegedActionService,
        run_privileged_worker_with_service,
    },
};

fn unix_nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos()
}

fn wait_for_socket(path: &Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for socket {}", path.display());
}

fn parse_thread_stat() -> anyhow::Result<(u32, i32, u64)> {
    let stat = fs::read_to_string("/proc/thread-self/stat")?;
    let tid = stat
        .split_once(' ')
        .and_then(|(tid, _)| tid.parse::<u32>().ok())
        .ok_or_else(|| anyhow::anyhow!("failed to parse thread tid from stat"))?;
    let close_paren = stat
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("stat line does not contain closing comm parenthesis"))?;
    let fields = stat[close_paren + 1..]
        .split_whitespace()
        .collect::<Vec<_>>();
    let nice = fields
        .get(16)
        .ok_or_else(|| anyhow::anyhow!("stat line missing nice field"))?
        .parse::<i32>()?;
    let starttime_ticks = fields
        .get(19)
        .ok_or_else(|| anyhow::anyhow!("stat line missing starttime field"))?
        .parse::<u64>()?;

    Ok((tid, nice, starttime_ticks))
}

fn current_thread_nice_candidate() -> anyhow::Result<CandidateAction> {
    let (tid, nice, starttime_ticks) = parse_thread_stat()?;
    let comm = fs::read_to_string("/proc/thread-self/comm")
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty());
    let target = TaskIdentity {
        tid,
        process_pid: Some(std::process::id()),
        comm,
        starttime_ticks: Some(starttime_ticks),
    };

    Ok(CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "socket-nice-noop".to_owned(),
            action: NiceAction {
                targets: vec![target],
                nice,
                policy: NicePolicy::default(),
            },
            target_root_pid: Some(std::process::id()),
            evidence: vec![CandidateEvidence::new(
                "test_current_thread",
                format!("tid={tid} nice={nice}"),
                1.0,
            )],
            objective: ObjectiveKind::GameRunnableLatency,
        },
    })
}

fn candidate_apply_request(candidate: CandidateAction) -> CandidateApplyRequest {
    CandidateApplyRequest {
        plan: CandidatePlanRequest::from_candidate(candidate, unix_nanos_now()),
        policy: DaemonPolicy::apply_medium_risk(ActionSource::Test),
        context: DaemonPolicyContext::default(),
        max_plan_age_nanos: 60_000_000_000,
    }
}

#[test]
fn worker_socket_apply_and_rollback_roundtrip() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let socket = temp.path().join("worker.sock");
    let service = Arc::new(InProcessPrivilegedActionService::default());
    let worker_service = Arc::clone(&service);
    let worker_socket = socket.clone();
    let handle = thread::spawn(move || {
        run_privileged_worker_with_service(&worker_socket, worker_service.as_ref())
    });
    wait_for_socket(&socket);

    let client = UnixSocketPrivilegedActionService::new(&socket);
    let candidate = current_thread_nice_candidate()?;
    let dry_run = client.dry_run_candidate(candidate_apply_request(candidate.clone()))?;
    assert_eq!(dry_run.candidate_name, "socket-nice-noop");

    let apply = client.apply_candidate(candidate_apply_request(candidate.clone()))?;
    let rollback_token = apply.rollback.clone();
    assert!(matches!(rollback_token, RollbackToken::NiceRestore { .. }));

    let rollback = client.rollback(RollbackRequest {
        candidate,
        token: apply.rollback,
        policy: DaemonPolicy::apply_medium_risk(ActionSource::Test),
        context: DaemonPolicyContext::default(),
    })?;
    assert_eq!(rollback.affected_tasks, rollback_token.affected_tasks());

    client.request_shutdown()?;
    handle.join().expect("worker thread panicked")?;
    assert!(!socket.exists());

    Ok(())
}

#[test]
fn worker_connection_refused_surfaces_as_error() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let socket = temp.path().join("missing.sock");
    let client = UnixSocketPrivilegedActionService::new(&socket);
    let err = client
        .dry_run_candidate(candidate_apply_request(current_thread_nice_candidate()?))
        .expect_err("missing socket should fail before request execution");

    assert!(format!("{err:#}").contains("failed to connect"));

    Ok(())
}

#[test]
fn worker_handle_restart_recovers_connectivity() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let socket = temp.path().join("managed-worker.sock");
    let executable = Path::new(env!("CARGO_BIN_EXE_stutter"));
    let mut handle = PrivilegedWorkerHandle::spawn_with_executable(&socket, executable)?;
    assert!(handle.is_alive());

    handle.terminate()?;
    assert!(!handle.is_alive());
    handle.restart()?;
    assert!(handle.is_alive());

    let client = UnixSocketPrivilegedActionService::new(&socket);
    let dry_run =
        client.dry_run_candidate(candidate_apply_request(current_thread_nice_candidate()?))?;
    assert_eq!(dry_run.candidate_name, "socket-nice-noop");

    handle.shutdown_gracefully(3_000)?;
    assert!(!handle.is_alive());

    Ok(())
}
