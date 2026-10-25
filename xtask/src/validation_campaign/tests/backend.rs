use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde_json::json;
use stutter_validation_runner::{
    model::{Authorization, Envelope, LiveSuite, RunnerAction, TaskGrant},
    process::ActionExecutor,
    server::Server,
    state::StateDir,
};

use super::{Scratch, fresh, manifest};
use crate::{
    evidence_bundle::tests::{COMMIT, raw},
    validation_campaign::{
        backend::{BackendError, RunnerBackend, SuiteBackend, SuiteInvocation},
        executor::{RunContext, RunnerOutcome, TaskRunner},
        graph::Graph,
        model::FailureClass,
        repository::Binaries,
        runners::{LiveSuiteRunner, SuiteShape},
        state::CampaignDir,
    },
};

/// Scripted backend: optionally materializes a raw directory like a finished suite would.
struct Scripted {
    result: Result<i32, BackendError>,
    populate: Option<PathBuf>,
}

impl SuiteBackend for Scripted {
    fn execute(&self, invocation: &SuiteInvocation<'_>) -> Result<i32, BackendError> {
        if let Some(source) = &self.populate {
            copy_dir(source, invocation.raw);
        }
        match &self.result {
            Ok(code) => Ok(*code),
            Err(BackendError::Privilege(message)) => Err(BackendError::Privilege(message.clone())),
            Err(BackendError::Failed(message)) => Err(BackendError::Failed(message.clone())),
        }
    }
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn run(backend: Scripted, scratch: &Scratch, base: &str) -> RunnerOutcome {
    let (dir, mut state, _) = fresh(scratch);
    state.evidence_base = base.to_owned();
    let manifest = manifest();
    let binaries = Binaries {
        stutter: PathBuf::from("s"),
        xtask: PathBuf::from("x"),
    };
    let runner = LiveSuiteRunner {
        suite: "observation",
        shape: SuiteShape::Seconds,
        backend: Box::new(backend),
        work_units: None,
    };
    let spec = Graph::core()
        .unwrap()
        .get("live-observation")
        .unwrap()
        .clone();
    runner.run(
        &RunContext {
            dir: &dir,
            state: &state,
            manifest: &manifest,
            repository: scratch.path(),
            binaries: &binaries,
            updates: &std::cell::RefCell::new(Vec::new()),
        },
        &spec,
        1,
    )
}

#[test]
fn a_privilege_error_blocks_instead_of_retrying() {
    let scratch = Scratch::new("backend-priv");
    let outcome = run(
        Scripted {
            result: Err(BackendError::Privilege("denied".into())),
            populate: None,
        },
        &scratch,
        COMMIT,
    );
    assert!(
        matches!(outcome, RunnerOutcome::Blocked(ref r) if r.class == FailureClass::BlockedPrivilege)
    );
}

#[test]
fn a_launch_failure_or_missing_result_is_retryable() {
    let scratch = Scratch::new("backend-launch");
    let outcome = run(
        Scripted {
            result: Err(BackendError::Failed("boom".into())),
            populate: None,
        },
        &scratch,
        COMMIT,
    );
    assert!(
        matches!(outcome, RunnerOutcome::Failed { class: FailureClass::Retryable, ref stage, .. } if stage == "launch")
    );
    let scratch = Scratch::new("backend-noresult");
    let outcome = run(
        Scripted {
            result: Ok(0),
            populate: None,
        },
        &scratch,
        COMMIT,
    );
    assert!(
        matches!(outcome, RunnerOutcome::Failed { class: FailureClass::Retryable, ref stage, .. } if stage == "suite_result")
    );
}

#[test]
fn a_blocked_suite_result_is_a_privilege_block_and_a_passing_one_publishes_a_bundle() {
    let source = raw("live_observation", "ok");
    let blocked = Scratch::new("backend-blocked");
    let blocked_source = raw("live_observation", "ok");
    let path = blocked_source.join("result.json");
    let mut result: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    result["status"] = json!("BLOCKED");
    result["error"] = json!("needs CAP_BPF");
    fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
    let outcome = run(
        Scripted {
            result: Ok(1),
            populate: Some(blocked_source.0.clone()),
        },
        &blocked,
        COMMIT,
    );
    assert!(
        matches!(outcome, RunnerOutcome::Blocked(ref r) if r.class == FailureClass::BlockedPrivilege && r.detail.contains("CAP_BPF"))
    );
    let scratch = Scratch::new("backend-pass");
    let outcome = run(
        Scripted {
            result: Ok(0),
            populate: Some(source.0.clone()),
        },
        &scratch,
        COMMIT,
    );
    assert!(matches!(
        outcome,
        RunnerOutcome::Passed {
            receipt: Some(_),
            ..
        }
    ));
    // A bundle certifying another commit is a verification failure, not a pass.
    let wrong = Scratch::new("backend-wrong");
    let outcome = run(
        Scripted {
            result: Ok(0),
            populate: Some(raw("live_observation", "ok").0.clone()),
        },
        &wrong,
        "fedcba9876543210fedcba9876543210fedcba98",
    );
    assert!(
        matches!(outcome, RunnerOutcome::Failed { class: FailureClass::SoftwareDefect, ref stage, .. } if stage == "source_commit")
    );
}

struct Recorded(Arc<Mutex<Vec<String>>>);

impl ActionExecutor for Recorded {
    fn live_suite(
        &self,
        _: &Authorization,
        e: &Envelope,
    ) -> Result<serde_json::Value, (stutter_validation_runner::model::ErrorCode, String)> {
        self.0.lock().unwrap().push(format!("{:?}", e.body));
        Ok(json!({"exit_code": 0}))
    }
    fn scheduler_burst(
        &self,
        _: &Authorization,
        _: &Envelope,
    ) -> Result<serde_json::Value, (stutter_validation_runner::model::ErrorCode, String)> {
        Ok(json!({"pid": 1}))
    }
}

#[test]
fn the_runner_backend_sends_a_typed_request_and_maps_authorization_denials_to_blocks() {
    if stutter_validation_runner::sys::effective_uid() == 0 {
        eprintln!("skipped: the socket test uses the caller's own uid as the campaign owner");
        return;
    }
    let scratch = Scratch::new("runner-backend");
    let (dir, state, _) = fresh(&scratch);
    let uid = stutter_validation_runner::sys::effective_uid();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut grants = std::collections::BTreeSet::new();
    grants.insert(TaskGrant {
        task_id: "live-observation".to_owned(),
        action: RunnerAction::ExecuteLiveSuite,
        suite: Some(LiveSuite::Observation),
    });
    let authorization = Authorization {
        controller: stutter_validation_runner::model::ControllerKind::CoreValidation,
        schema_version: 1,
        campaign_id: state.campaign_id.clone(),
        owner_uid: uid,
        owner_gid: uid,
        repository: "/repo".to_owned(),
        manifest_sha256: state.manifest_sha256.clone(),
        runner_sha256: "a".repeat(64),
        xtask_sha256: "b".repeat(64),
        stutter_sha256: "c".repeat(64),
        path_env: "/usr/bin".to_owned(),
        grants,
        max_reboots: 1,
        reboot_slack: 2,
        original_boot_entry: None,
        allowed_boot_entries: vec![],
        created_unix_ms: 1,
    };
    let server = Arc::new(Server::new(
        authorization,
        StateDir(scratch.path().join("runner-state")),
        Arc::new(Recorded(calls.clone())),
        Arc::new(stutter_validation_runner::boot::UnsupportedBoot),
    ));
    let socket = scratch.path().join("runner.sock");
    let serving = Arc::clone(&server);
    let path = socket.clone();
    std::thread::spawn(move || {
        let _ = serving.serve(&path);
    });
    for _ in 0..100 {
        if socket.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let raw_dir = dir.0.join("raw");
    let log = dir.0.join("log");
    let invocation = |task: &'static str, campaign: &'static str| SuiteInvocation {
        task,
        suite: "observation",
        attempt: 2,
        seconds: 20,
        rounds: None,
        work_units: None,
        raw: &raw_dir,
        log: &log,
        stutter: std::path::Path::new("s"),
        xtask: std::path::Path::new("x"),
        cwd: std::path::Path::new("."),
        campaign_id: campaign,
        manifest_sha256: &state.manifest_sha256,
    };
    let backend = RunnerBackend {
        socket: socket.clone(),
    };
    let campaign: &'static str = Box::leak(state.campaign_id.clone().into_boxed_str());
    assert_eq!(
        backend.execute(&invocation("live-observation", campaign)),
        Ok(0)
    );
    assert!(
        calls.lock().unwrap()[0].contains("attempt: 2"),
        "{:?}",
        calls.lock().unwrap()
    );
    // A task the campaign was never granted is a privilege block, not a retryable failure.
    assert!(matches!(
        backend.execute(&invocation("game-runnable", campaign)),
        Err(BackendError::Privilege(_))
    ));
    assert!(matches!(
        backend.execute(&invocation("live-observation", "other-campaign")),
        Err(BackendError::Privilege(_))
    ));
    // No runner listening at all is a privilege block too.
    let missing = RunnerBackend {
        socket: scratch.path().join("absent.sock"),
    };
    assert!(matches!(
        missing.execute(&invocation("live-observation", campaign)),
        Err(BackendError::Privilege(_))
    ));
    let _ = CampaignDir(PathBuf::new());
}
