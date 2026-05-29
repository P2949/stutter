use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use super::*;
use crate::{
    actions::{ActionState, SafetyClass},
    audit::AuditEvent,
    autotune::objective::ObjectiveKind,
};

fn check(request: PrivilegeCommandRequest) -> PrivilegeDecision {
    PrivilegeCommandAllowlist.check(&request)
}
fn fake_apply_request() -> CandidateApplyRequest {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake:privilege".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    CandidateApplyRequest {
        plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
        policy: DaemonPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test),
        context: DaemonPolicyContext::default(),
        max_plan_age_nanos: 1_000_000_000,
    }
}

fn nice_candidate(target: TaskIdentity) -> CandidateAction {
    CandidateAction::Nice {
        plan: crate::autotune::planning::executable_plan::NiceActionPlan {
            name: format!("nice-{}", target.tid),
            action: crate::actions::nice::NiceAction {
                targets: vec![target],
                nice: 5,
                policy: crate::actions::nice::NicePolicy::default(),
            },
            target_root_pid: Some(100),
            evidence: vec![
                crate::autotune::planning::candidate::CandidateEvidence::new(
                    "test",
                    "target revalidation test",
                    1.0,
                ),
            ],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    }
}

fn nice_apply_request(candidate: CandidateAction) -> CandidateApplyRequest {
    CandidateApplyRequest {
        plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
        policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
        context: DaemonPolicyContext::default(),
        max_plan_age_nanos: 1_000_000_000,
    }
}

fn target(tid: u32, process_pid: u32, comm: &str, starttime: u64) -> TaskIdentity {
    TaskIdentity {
        tid: tid.into(),
        process_pid: Some((process_pid).into()),
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime),
    }
}

fn temp_proc_root(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "stutter-target-revalidation-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn temp_audit_path(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "stutter-privilege-audit-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&root).unwrap();
    root.join("audit.jsonl")
}

fn read_audit_events(path: &std::path::Path) -> Vec<AuditEvent> {
    crate::audit::read_audit_tail(path, 100).unwrap()
}

fn proc_stat(tid: u32, comm: &str, starttime: u64) -> String {
    let mut fields = vec!["S".to_owned()];
    fields.extend((0..18).map(|_| "0".to_owned()));
    fields.push(starttime.to_string());
    fields.extend((0..24).map(|_| "0".to_owned()));
    format!("{tid} ({comm}) {}\n", fields.join(" "))
}

fn write_expected_task(
    proc_root: &std::path::Path,
    process_pid: u32,
    tid: u32,
    comm: &str,
    starttime: u64,
) {
    let task_dir = proc_root
        .join(process_pid.to_string())
        .join("task")
        .join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("stat"), proc_stat(tid, comm, starttime)).unwrap();
}

fn write_top_level_task(
    proc_root: &std::path::Path,
    tgid: u32,
    tid: u32,
    comm: &str,
    starttime: u64,
) {
    let task_dir = proc_root.join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("stat"), proc_stat(tid, comm, starttime)).unwrap();
    fs::write(
        task_dir.join("status"),
        format!("Name:\t{comm}\nTgid:\t{tgid}\n"),
    )
    .unwrap();
}

#[derive(Debug, Default)]
struct FakeWorkerService {
    dry_run_calls: Mutex<usize>,
    apply_calls: Mutex<usize>,
    rollback_calls: Mutex<usize>,
}

impl FakeWorkerService {
    fn calls(&self, field: &Mutex<usize>) -> usize {
        *field.lock().unwrap()
    }
}

impl PrivilegedActionService for FakeWorkerService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord> {
        *self.dry_run_calls.lock().unwrap() += 1;
        Ok(CandidateDryRunRecord {
            candidate_name: request.plan.candidate.candidate_name().to_owned(),
            affected_tasks: 2,
            warnings: Vec::new(),
            safety_class: request.plan.candidate.safety_class(),
            eligible: true,
            reason: None,
        })
    }

    fn apply_candidate(&self, _request: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        *self.apply_calls.lock().unwrap() += 1;
        Ok(ApplyResult {
            state: ActionState {
                applied: true,
                affected_tasks: 2,
                checked_tasks: 2,
                pending_changes: 2,
                warnings: Vec::new(),
            },
            rollback: RollbackToken::NiceRestore {
                records: Vec::new(),
            },
        })
    }

    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult> {
        *self.rollback_calls.lock().unwrap() += 1;
        Ok(RollbackResult {
            affected_tasks: request.token.affected_tasks(),
        })
    }
}

fn temp_socket_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "stutter-privileged-worker-{name}-{}-{}.sock",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    path
}

fn wait_for_socket(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for socket {}", path.display());
}

fn unix_socket_bind_supported() -> bool {
    let socket = temp_socket_path("support-probe");
    match UnixListener::bind(&socket) {
        Ok(listener) => {
            drop(listener);
            fs::remove_file(socket).ok();
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(err) => panic!("unexpected privileged-worker unix socket probe error: {err}"),
    }
}

mod socket;

mod worker;

mod audit;

mod policy_validation;

mod errors;
