use super::{Scratch, fresh};
use crate::validation_campaign::{
    model::{AttemptOutcome, BlockReason, FailureClass, TaskAttempt, TaskStatus},
    state::{CampaignDir, load, save, write_atomic},
};

fn attempt(
    state: &crate::validation_campaign::model::CampaignState,
    graph: &crate::validation_campaign::graph::Graph,
    task: &str,
    number: u32,
    outcome: AttemptOutcome,
) -> TaskAttempt {
    TaskAttempt {
        number,
        started_unix_ms: 1,
        finished_unix_ms: 2,
        outcome,
        fingerprint: state.expected_fingerprint(graph.get(task).unwrap()),
        artifacts: None,
        receipt: None,
        limitations: Vec::new(),
    }
}

#[test]
fn a_fresh_campaign_starts_at_offline_ready_and_unlocks_collection() {
    let scratch = Scratch::new("ready");
    let (dir, mut state, graph) = fresh(&scratch);
    assert_eq!(state.ready(&graph), ["offline-ready"]);
    let passed = attempt(&state, &graph, "offline-ready", 1, AttemptOutcome::Passed);
    state.record_attempt(&dir, "offline-ready", passed).unwrap();
    let ready = state.ready(&graph);
    assert_eq!(ready.len(), 8, "{ready:?}");
    assert!(ready.contains(&"live-observation") && ready.contains(&"corpus"));
    assert!(
        !ready.contains(&"evidence-integration"),
        "integration waits for collection"
    );
}

#[test]
fn only_a_passed_task_satisfies_its_dependents() {
    let scratch = Scratch::new("deps");
    let (dir, mut state, graph) = fresh(&scratch);
    for (number, outcome) in [
        AttemptOutcome::Failed {
            class: FailureClass::SoftwareDefect,
            stage: "gate".into(),
            summary: "x".into(),
        },
        AttemptOutcome::Blocked(BlockReason {
            class: FailureClass::BlockedPrivilege,
            detail: "no".into(),
        }),
        AttemptOutcome::WaitingForReboot,
    ]
    .into_iter()
    .enumerate()
    {
        let record = attempt(&state, &graph, "offline-ready", number as u32 + 1, outcome);
        state.record_attempt(&dir, "offline-ready", record).unwrap();
        assert!(
            state.ready(&graph).iter().all(|id| *id == "offline-ready"),
            "nothing unlocks"
        );
    }
}

#[test]
fn attempts_are_append_only_sequential_and_immutable_on_disk() {
    let scratch = Scratch::new("attempts");
    let (dir, mut state, graph) = fresh(&scratch);
    let failure = AttemptOutcome::Failed {
        class: FailureClass::Retryable,
        stage: "s".into(),
        summary: "first".into(),
    };
    state
        .record_attempt(
            &dir,
            "offline-ready",
            attempt(&state, &graph, "offline-ready", 1, failure),
        )
        .unwrap();
    state
        .record_attempt(
            &dir,
            "offline-ready",
            attempt(&state, &graph, "offline-ready", 2, AttemptOutcome::Passed),
        )
        .unwrap();
    let task = &state.tasks["offline-ready"];
    assert_eq!(
        task.attempts.len(),
        2,
        "a retry adds a record and keeps the failure"
    );
    assert_eq!(task.failed_attempts(), 1);
    assert!(
        dir.attempt_dir("offline-ready", 1)
            .join("attempt.json")
            .is_file()
    );
    assert!(
        dir.attempt_dir("offline-ready", 2)
            .join("attempt.json")
            .is_file()
    );
    // Skipping or repeating a number is a bug, not a silent overwrite.
    assert!(
        state
            .record_attempt(
                &dir,
                "offline-ready",
                attempt(&state, &graph, "offline-ready", 2, AttemptOutcome::Passed)
            )
            .is_err()
    );
    assert!(
        state
            .record_attempt(
                &dir,
                "offline-ready",
                attempt(&state, &graph, "offline-ready", 9, AttemptOutcome::Passed)
            )
            .is_err()
    );
}

#[test]
fn state_round_trips_atomically_and_corruption_is_an_error() {
    let scratch = Scratch::new("persist");
    let (dir, state, _) = fresh(&scratch);
    save(&dir, &state).unwrap();
    assert_eq!(load(&dir).unwrap(), state);
    // No temporary files are left behind by an atomic write.
    let leftovers = std::fs::read_dir(&dir.0)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
        .count();
    assert_eq!(leftovers, 0);
    write_atomic(&dir.state_path(), b"{ not json").unwrap();
    let error = format!("{:#}", load(&dir).unwrap_err());
    assert!(error.contains("corrupt"), "{error}");
    write_atomic(&dir.state_path(), br#"{"schema_version":99}"#).unwrap();
    assert!(load(&dir).is_err());
    let missing = CampaignDir(scratch.path().join("nowhere"));
    assert!(load(&missing).is_err());
}

#[test]
fn skipped_by_policy_never_satisfies_a_dependency() {
    assert!(!TaskStatus::SkippedByPolicy.satisfies_dependents());
    for status in [
        TaskStatus::Pending,
        TaskStatus::Ready,
        TaskStatus::Running,
        TaskStatus::Failed,
        TaskStatus::BlockedResource,
        TaskStatus::BlockedPrivilege,
        TaskStatus::WaitingForReboot,
        TaskStatus::Invalidated,
    ] {
        assert!(!status.satisfies_dependents(), "{status:?}");
    }
    assert!(TaskStatus::Passed.satisfies_dependents());
}

#[test]
fn a_passed_result_with_a_different_fingerprint_is_historical_not_current() {
    let scratch = Scratch::new("fingerprint");
    let (dir, mut state, graph) = fresh(&scratch);
    let passed = attempt(
        &state,
        &graph,
        "live-observation",
        1,
        AttemptOutcome::Passed,
    );
    state
        .record_attempt(&dir, "live-observation", passed)
        .unwrap();
    let spec = graph.get("live-observation").unwrap();
    assert_eq!(state.effective_status(spec), TaskStatus::Passed);
    state.manifest_sha256 = "different".repeat(8);
    assert_eq!(state.effective_status(spec), TaskStatus::Invalidated);
    assert_eq!(
        state.tasks["live-observation"].attempts.len(),
        1,
        "history is retained"
    );
}

#[test]
fn the_binary_hash_cache_recomputes_when_the_file_changes() {
    use crate::validation_campaign::repository::{cached_file_sha256, file_sha256};
    let scratch = Scratch::new("hash-cache");
    let path = scratch.path().join("binary");
    std::fs::write(&path, b"first").unwrap();
    let first = cached_file_sha256(&path).unwrap();
    assert_eq!(first, file_sha256(&path).unwrap());
    assert_eq!(
        cached_file_sha256(&path).unwrap(),
        first,
        "unchanged file hits the cache"
    );
    std::fs::write(&path, b"second-and-longer").unwrap();
    let second = cached_file_sha256(&path).unwrap();
    assert_ne!(second, first, "a rewritten file is rehashed");
    assert_eq!(second, file_sha256(&path).unwrap());
}
