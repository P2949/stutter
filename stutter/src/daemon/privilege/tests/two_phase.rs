//! The two-phase apply: prepare, the controller's durable journal write, commit. Every failure
//! before the commit leaves the target untouched; the worker never writes on its own initiative.
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::*;

/// Records when its barrier was reached and whether the mutation happened.
#[derive(Debug, Default)]
struct BarrierService {
    mutations: AtomicUsize,
    barriers: AtomicUsize,
}

fn token() -> RollbackToken {
    RollbackToken::NiceRestore {
        records: Vec::new(),
    }
}

impl PrivilegedActionService for BarrierService {
    fn dry_run_candidate(&self, _: CandidateApplyRequest) -> anyhow::Result<CandidateDryRunRecord> {
        Err(PrivilegedWorkerError::PreparedApplyUnsupported.into())
    }

    fn apply_candidate(&self, _: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        Err(PrivilegedWorkerError::PreparedApplyUnsupported.into())
    }

    fn apply_candidate_prepared(
        &self,
        _: CandidateApplyRequest,
        prepared: &mut PreparedHook<'_>,
    ) -> anyhow::Result<ApplyResult> {
        let token = token();
        self.barriers.fetch_add(1, Ordering::SeqCst);
        // Preparation wrote nothing to the target. The commit barrier decides.
        prepared(&token)?;
        self.mutations.fetch_add(1, Ordering::SeqCst);
        Ok(ApplyResult {
            state: ActionState {
                applied: true,
                affected_tasks: 1,
                checked_tasks: 1,
                pending_changes: 0,
                warnings: Vec::new(),
            },
            rollback: token,
        })
    }

    fn rollback(&self, _: RollbackRequest) -> anyhow::Result<RollbackResult> {
        Err(PrivilegedWorkerError::PreparedApplyUnsupported.into())
    }
}

struct Worker {
    socket: PathBuf,
    service: Arc<BarrierService>,
    join: Option<thread::JoinHandle<anyhow::Result<()>>>,
}

impl Worker {
    fn start(name: &str) -> Self {
        let socket = temp_socket_path(name);
        let service = Arc::new(BarrierService::default());
        let (path, shared) = (socket.clone(), Arc::clone(&service));
        let join =
            thread::spawn(move || run_privileged_worker_with_service(&path, shared.as_ref()));
        wait_for_socket(&socket);
        Self {
            socket,
            service,
            join: Some(join),
        }
    }

    fn client(&self) -> UnixSocketPrivilegedActionService {
        UnixSocketPrivilegedActionService::new(&self.socket)
    }

    fn stop(mut self) {
        self.client().request_shutdown().unwrap();
        self.join.take().unwrap().join().unwrap().unwrap();
    }

    fn mutations(&self) -> usize {
        self.service.mutations.load(Ordering::SeqCst)
    }
}

fn request() -> CandidateApplyRequest {
    nice_apply_request(nice_candidate(TaskIdentity {
        tid: 4242.into(),
        process_pid: Some(4242.into()),
        comm: Some("two-phase".to_owned()),
        starttime_ticks: Some(7),
    }))
}

fn raw_exchange(socket: &Path) -> (UnixStream, BufReader<UnixStream>) {
    let mut stream = UnixStream::connect(socket).unwrap();
    let plan = PrivilegedWorkerCandidatePlan::from_plan_request(&request().plan);
    let line = serde_json::to_string(&PrivilegedWorkerRequest::ApplyPrepared {
        plan,
        policy: request().policy,
        context: request().context,
        max_plan_age_nanos: request().max_plan_age_nanos,
    })
    .unwrap();
    stream.write_all(format!("{line}\n").as_bytes()).unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    (stream, reader)
}

fn read_response(reader: &mut BufReader<UnixStream>) -> PrivilegedWorkerResponse {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

fn send(stream: &mut UnixStream, request: &PrivilegedWorkerRequest) {
    let line = serde_json::to_string(request).unwrap();
    stream.write_all(format!("{line}\n").as_bytes()).unwrap();
}

#[test]
fn the_controller_journals_between_prepare_and_commit_and_only_then_the_worker_mutates() {
    if !unix_socket_bind_supported() {
        return;
    }
    let worker = Worker::start("two-phase-happy");
    let journaled = AtomicUsize::new(0);
    let observed_mutations = AtomicUsize::new(usize::MAX);
    let result = worker
        .client()
        .apply_candidate_prepared(request(), &mut |token| {
            // At the journal write nothing has been mutated yet.
            observed_mutations.store(worker.mutations(), Ordering::SeqCst);
            journaled.fetch_add(1, Ordering::SeqCst);
            assert_eq!(token, &super::two_phase::token());
            Ok(())
        })
        .unwrap();
    assert_eq!(journaled.load(Ordering::SeqCst), 1);
    assert_eq!(observed_mutations.load(Ordering::SeqCst), 0);
    assert_eq!(worker.mutations(), 1);
    assert!(result.state.applied);
    worker.stop();
}

#[test]
fn a_failed_journal_write_aborts_before_any_mutation() {
    if !unix_socket_bind_supported() {
        return;
    }
    let worker = Worker::start("two-phase-journal-failure");
    let error = worker
        .client()
        .apply_candidate_prepared(request(), &mut |_| Err(anyhow::anyhow!("disk full")))
        .unwrap_err();
    assert!(format!("{error:#}").contains("disk full"), "{error:#}");
    // The worker saw an abort, not a commit.
    thread::sleep(Duration::from_millis(200));
    assert_eq!(worker.service.barriers.load(Ordering::SeqCst), 1);
    assert_eq!(worker.mutations(), 0);
    worker.stop();
}

#[test]
fn a_controller_that_vanishes_after_the_prepare_causes_no_mutation() {
    if !unix_socket_bind_supported() {
        return;
    }
    let worker = Worker::start("two-phase-controller-death");
    {
        let (stream, mut reader) = raw_exchange(&worker.socket);
        assert!(matches!(
            read_response(&mut reader),
            PrivilegedWorkerResponse::Prepared { .. }
        ));
        drop(reader);
        drop(stream);
    }
    thread::sleep(Duration::from_millis(300));
    assert_eq!(worker.service.barriers.load(Ordering::SeqCst), 1);
    assert_eq!(worker.mutations(), 0);
    worker.stop();
}

#[test]
fn silence_after_the_prepare_expires_without_mutation() {
    if !unix_socket_bind_supported() {
        return;
    }
    set_commit_timeout_for_tests(Duration::from_millis(250));
    let worker = Worker::start("two-phase-expiry");
    let (_stream, mut reader) = raw_exchange(&worker.socket);
    assert!(matches!(
        read_response(&mut reader),
        PrivilegedWorkerResponse::Prepared { .. }
    ));
    match read_response(&mut reader) {
        PrivilegedWorkerResponse::Error { reason_code, .. } => {
            assert_eq!(reason_code, "privileged_commit_expired");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(worker.mutations(), 0);
    set_commit_timeout_for_tests(Duration::from_secs(15));
    worker.stop();
}

#[test]
fn a_commit_for_another_snapshot_is_rejected() {
    if !unix_socket_bind_supported() {
        return;
    }
    let worker = Worker::start("two-phase-wrong-digest");
    let (mut stream, mut reader) = raw_exchange(&worker.socket);
    assert!(matches!(
        read_response(&mut reader),
        PrivilegedWorkerResponse::Prepared { .. }
    ));
    send(
        &mut stream,
        &PrivilegedWorkerRequest::CommitPrepared {
            commit: true,
            snapshot_digest: "0".repeat(64),
        },
    );
    match read_response(&mut reader) {
        PrivilegedWorkerResponse::Error { reason_code, .. } => {
            assert_eq!(reason_code, "privileged_commit_rejected");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(worker.mutations(), 0);
    worker.stop();
}

#[test]
fn a_duplicate_commit_cannot_apply_twice() {
    if !unix_socket_bind_supported() {
        return;
    }
    let worker = Worker::start("two-phase-duplicate-commit");
    let (mut stream, mut reader) = raw_exchange(&worker.socket);
    let digest = match read_response(&mut reader) {
        PrivilegedWorkerResponse::Prepared {
            snapshot_digest, ..
        } => snapshot_digest,
        other => panic!("{other:?}"),
    };
    let commit = PrivilegedWorkerRequest::CommitPrepared {
        commit: true,
        snapshot_digest: digest,
    };
    send(&mut stream, &commit);
    send(&mut stream, &commit);
    assert!(matches!(
        read_response(&mut reader),
        PrivilegedWorkerResponse::Apply { .. }
    ));
    assert_eq!(worker.mutations(), 1);
    // The exchange is over; the second frame is never read as a new request.
    thread::sleep(Duration::from_millis(100));
    assert_eq!(worker.mutations(), 1);
    worker.stop();
}

#[test]
fn a_commit_without_a_prepare_is_refused() {
    if !unix_socket_bind_supported() {
        return;
    }
    let worker = Worker::start("two-phase-stray-commit");
    let mut stream = UnixStream::connect(&worker.socket).unwrap();
    send(
        &mut stream,
        &PrivilegedWorkerRequest::CommitPrepared {
            commit: true,
            snapshot_digest: "0".repeat(64),
        },
    );
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    match read_response(&mut reader) {
        PrivilegedWorkerResponse::Error { reason_code, .. } => {
            assert_eq!(reason_code, "privileged_commit_without_prepare");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(worker.mutations(), 0);
    worker.stop();
}

#[test]
fn an_operation_without_a_prepared_transaction_is_refused_before_it_can_mutate() {
    let pid = std::process::id();
    let ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), pid).unwrap();
    let candidate = nice_candidate(TaskIdentity {
        tid: pid.into(),
        process_pid: Some(pid.into()),
        comm: std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .ok()
            .map(|comm| comm.trim().to_owned()),
        starttime_ticks: Some(ticks),
    });
    let executor =
        crate::autotune::apply::executor_for_candidate_preview(candidate.clone()).unwrap();
    assert!(!executor.prepared_before_mutation());
    let nice_before = process_nice();
    let reached = AtomicUsize::new(0);
    let error = InProcessPrivilegedActionService::default()
        .apply_candidate_prepared(nice_apply_request(candidate), &mut |_| {
            reached.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("prepared_transaction_missing"),
        "{error:#}"
    );
    assert_eq!(reached.load(Ordering::SeqCst), 0);
    assert_eq!(process_nice(), nice_before);
}

/// The process's nice value, read from `/proc` (no syscall wrappers needed).
fn process_nice() -> String {
    std::fs::read_to_string("/proc/self/stat")
        .unwrap()
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().nth(16).map(str::to_owned))
        .unwrap()
}

/// A scripted worker for failure cases a real worker cannot reproduce on demand: it accepts one
/// connection, reads the request and then follows `script`.
fn scripted_worker(
    name: &str,
    script: impl FnOnce(UnixStream, BufReader<UnixStream>) + Send + 'static,
) -> (PathBuf, thread::JoinHandle<()>) {
    let socket = temp_socket_path(name);
    let listener = UnixListener::bind(&socket).unwrap();
    let join = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        script(stream, reader);
    });
    (socket, join)
}

fn prepared_frame() -> String {
    let token = token();
    serde_json::to_string(&PrivilegedWorkerResponse::Prepared {
        snapshot_digest: rollback_snapshot_digest(&token),
        rollback: token,
    })
    .unwrap()
}

#[test]
fn a_worker_that_dies_before_answering_the_prepare_leaves_no_journal_entry() {
    if !unix_socket_bind_supported() {
        return;
    }
    let (socket, join) = scripted_worker("tp-dead-pre", |stream, _| drop(stream));
    let journaled = AtomicUsize::new(0);
    let error = UnixSocketPrivilegedActionService::new(&socket)
        .apply_candidate_prepared(request(), &mut |_| {
            journaled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
    join.join().unwrap();
    assert_eq!(journaled.load(Ordering::SeqCst), 0, "{error:#}");
}

#[test]
fn a_worker_that_dies_after_the_prepare_is_an_error_with_the_token_already_journaled() {
    if !unix_socket_bind_supported() {
        return;
    }
    let (socket, join) = scripted_worker("tp-dead-post", |mut stream, _| {
        stream
            .write_all(format!("{}\n", prepared_frame()).as_bytes())
            .unwrap();
        // Dies without waiting for the controller's answer.
    });
    let journaled = AtomicUsize::new(0);
    let result = UnixSocketPrivilegedActionService::new(&socket).apply_candidate_prepared(
        request(),
        &mut |token| {
            assert_eq!(token, &super::two_phase::token());
            journaled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );
    join.join().unwrap();
    assert_eq!(journaled.load(Ordering::SeqCst), 1);
    assert!(
        result.is_err(),
        "an apply that never completed is not a success"
    );
}

#[test]
fn a_worker_that_dies_during_the_commit_is_never_reported_as_applied() {
    if !unix_socket_bind_supported() {
        return;
    }
    let (socket, join) = scripted_worker("tp-dead-commit", |mut stream, mut reader| {
        stream
            .write_all(format!("{}\n", prepared_frame()).as_bytes())
            .unwrap();
        let mut commit = String::new();
        reader.read_line(&mut commit).unwrap();
        assert!(commit.contains("commit_prepared") || commit.contains("CommitPrepared"));
        // Dies after receiving the commit, before any answer.
    });
    let journaled = AtomicUsize::new(0);
    let result = UnixSocketPrivilegedActionService::new(&socket).apply_candidate_prepared(
        request(),
        &mut |_| {
            journaled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );
    join.join().unwrap();
    assert_eq!(journaled.load(Ordering::SeqCst), 1);
    assert!(result.is_err());
}

#[test]
fn a_worker_that_misreports_its_snapshot_digest_is_never_committed() {
    if !unix_socket_bind_supported() {
        return;
    }
    let (socket, join) = scripted_worker("tp-lying", |mut stream, mut reader| {
        let frame = serde_json::to_string(&PrivilegedWorkerResponse::Prepared {
            rollback: token(),
            snapshot_digest: "f".repeat(64),
        })
        .unwrap();
        stream.write_all(format!("{frame}\n").as_bytes()).unwrap();
        let mut answer = String::new();
        reader.read_line(&mut answer).unwrap();
        let request: PrivilegedWorkerRequest = serde_json::from_str(&answer).unwrap();
        assert!(matches!(
            request,
            PrivilegedWorkerRequest::CommitPrepared { commit: false, .. }
        ));
    });
    let journaled = AtomicUsize::new(0);
    let result = UnixSocketPrivilegedActionService::new(&socket).apply_candidate_prepared(
        request(),
        &mut |_| {
            journaled.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );
    join.join().unwrap();
    assert_eq!(journaled.load(Ordering::SeqCst), 0);
    assert!(result.is_err());
}
