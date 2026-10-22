use std::{
    fs,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};

use super::{MANIFEST, Scratch, authorization, envelope, me, observation, peer};
use crate::{
    authorization::Denial,
    boot::BootControl,
    client::Client,
    model::{Authorization, Envelope, ErrorCode, Request, ResponseStatus},
    process::ActionExecutor,
    protocol::{read_frame, write_frame},
    server::Server,
    state::StateDir,
};

#[derive(Default)]
struct Calls(Mutex<Vec<String>>);

struct FakeExecutor(Arc<Calls>);

impl ActionExecutor for FakeExecutor {
    fn live_suite(&self, _a: &Authorization, e: &Envelope) -> Result<Value, Denial> {
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("suite:{}", e.task_id));
        Ok(json!({"exit_code": 0}))
    }
    fn scheduler_burst(&self, _a: &Authorization, e: &Envelope) -> Result<Value, Denial> {
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("burst:{}", e.task_id));
        Ok(json!({"pid": 1}))
    }
}

struct FakeBoot {
    calls: Arc<Calls>,
    state_dir: std::path::PathBuf,
    fail_reboot: bool,
}

impl BootControl for FakeBoot {
    fn set_next_boot(&self, entry: &str) -> Result<(), String> {
        self.calls.0.lock().unwrap().push(format!("next:{entry}"));
        Ok(())
    }
    fn reboot(&self) -> Result<(), String> {
        // The durable budget and resume flag must already be on disk when the reboot starts.
        let state = fs::read_to_string(self.state_dir.join("boot-state.json")).unwrap();
        assert!(state.contains("\"resume_pending\": true"), "{state}");
        self.calls.0.lock().unwrap().push("reboot".to_owned());
        if self.fail_reboot {
            Err("reboot refused".to_owned())
        } else {
            Ok(())
        }
    }
}

fn server(scratch: &Scratch, fail_reboot: bool) -> (Server, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let state_dir = scratch.path().join("state");
    fs::create_dir_all(&state_dir).unwrap();
    let server = Server::new(
        authorization("/repo"),
        StateDir(state_dir.clone()),
        Arc::new(FakeExecutor(calls.clone())),
        Arc::new(FakeBoot {
            calls: calls.clone(),
            state_dir,
            fail_reboot,
        }),
    );
    (server, calls)
}

fn rejected_with(server: &Server, uid: u32, task: &str, body: Request) -> ErrorCode {
    let response = server.handle(peer(uid), envelope(task, body));
    assert_ne!(response.status, ResponseStatus::Ok, "{response:?}");
    response.error.unwrap().code
}

#[test]
fn authorized_requests_run_and_every_outcome_is_receipted() {
    let scratch = Scratch::new("server-ok");
    let (server, calls) = server(&scratch, false);
    let ok = server.handle(peer(me()), envelope("live-observation", observation()));
    assert_eq!(ok.status, ResponseStatus::Ok);
    assert_eq!(
        calls.0.lock().unwrap().as_slice(),
        ["suite:live-observation"]
    );
    let receipt = ok.receipt.expect("receipt id");
    assert!(server.state.receipts().join(&receipt).is_file());
    let denied = server.handle(peer(me() + 1), envelope("live-observation", observation()));
    assert_eq!(denied.status, ResponseStatus::Rejected);
    assert!(denied.receipt.is_some(), "rejections are receipted too");
    assert_eq!(
        calls.0.lock().unwrap().len(),
        1,
        "a rejected request executes nothing"
    );
}

#[test]
fn unknown_uid_and_unapproved_requests_never_reach_the_executor() {
    let scratch = Scratch::new("server-deny");
    let (server, calls) = server(&scratch, false);
    assert_eq!(
        rejected_with(&server, me() + 1, "live-observation", observation()),
        ErrorCode::UnauthorizedPeer
    );
    assert_eq!(
        rejected_with(&server, me(), "game-runnable", observation()),
        ErrorCode::TaskNotPermitted
    );
    assert!(calls.0.lock().unwrap().is_empty());
}

#[test]
fn the_reboot_budget_is_enforced_and_state_is_durable_before_each_reboot() {
    let scratch = Scratch::new("server-budget");
    let (server, calls) = server(&scratch, false);
    for expected in 1..=2 {
        let response = server.handle(peer(me()), envelope("kernel-matrix", Request::Reboot));
        assert_eq!(response.status, ResponseStatus::Ok, "{response:?}");
        assert_eq!(response.data.unwrap()["reboots_used"], expected);
    }
    assert_eq!(
        rejected_with(&server, me(), "kernel-matrix", Request::Reboot),
        ErrorCode::RebootBudgetExhausted
    );
    assert_eq!(
        calls
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "reboot")
            .count(),
        2
    );
    let state = server.state.load("core-abc1234-1").unwrap();
    assert_eq!((state.reboots_used, state.resume_pending), (2, true));
}

#[test]
fn a_failed_reboot_rolls_the_budget_back() {
    let scratch = Scratch::new("server-reboot-fail");
    let (server, _) = server(&scratch, true);
    let response = server.handle(peer(me()), envelope("kernel-matrix", Request::Reboot));
    assert_eq!(response.status, ResponseStatus::Failed);
    let state = server.state.load("core-abc1234-1").unwrap();
    assert_eq!((state.reboots_used, state.resume_pending), (0, false));
}

#[test]
fn corrupt_state_stops_everything_and_never_reboots() {
    let scratch = Scratch::new("server-corrupt");
    let (server, calls) = server(&scratch, false);
    fs::write(server.state.state_path(), b"{ not json").unwrap();
    for body in [Request::Reboot, observation(), Request::Status] {
        let task = if matches!(body, Request::Reboot) {
            "kernel-matrix"
        } else if matches!(body, Request::Status) {
            "status"
        } else {
            "live-observation"
        };
        assert_eq!(
            rejected_with(&server, me(), task, body),
            ErrorCode::CorruptState
        );
    }
    assert!(
        calls.0.lock().unwrap().is_empty(),
        "nothing ran and no reboot was attempted"
    );
    // A state file for another campaign is equally corrupt.
    fs::write(
        server.state.state_path(),
        br#"{"schema_version":1,"campaign_id":"someone-else"}"#,
    )
    .unwrap();
    assert_eq!(
        rejected_with(&server, me(), "kernel-matrix", Request::Reboot),
        ErrorCode::CorruptState
    );
}

#[test]
fn boot_selection_is_recorded_and_unauthorized_entries_are_refused() {
    let scratch = Scratch::new("server-boot");
    let (server, calls) = server(&scratch, false);
    let ok = server.handle(
        peer(me()),
        envelope(
            "kernel-matrix",
            Request::SetNextBoot {
                entry_id: "kernel-b".into(),
            },
        ),
    );
    assert_eq!(ok.status, ResponseStatus::Ok);
    assert_eq!(
        server
            .state
            .load("core-abc1234-1")
            .unwrap()
            .pending_boot_entry
            .as_deref(),
        Some("kernel-b")
    );
    assert_eq!(
        rejected_with(
            &server,
            me(),
            "kernel-matrix",
            Request::SetNextBoot {
                entry_id: "evil".into()
            }
        ),
        ErrorCode::BootEntryNotAuthorized
    );
    assert_eq!(calls.0.lock().unwrap().as_slice(), ["next:kernel-b"]);
    let resumed = server.handle(
        peer(me()),
        envelope("kernel-matrix", Request::ResumeCampaign),
    );
    assert_eq!(resumed.status, ResponseStatus::Ok);
    assert_eq!(
        server
            .state
            .load("core-abc1234-1")
            .unwrap()
            .pending_boot_entry,
        None
    );
}

#[test]
fn cancellation_blocks_further_work_but_not_status() {
    let scratch = Scratch::new("server-cancel");
    let (server, calls) = server(&scratch, false);
    assert_eq!(
        server
            .handle(
                peer(me()),
                envelope("kernel-matrix", Request::CancelCampaign)
            )
            .status,
        ResponseStatus::Ok
    );
    assert_eq!(
        rejected_with(&server, me(), "live-observation", observation()),
        ErrorCode::Cancelled
    );
    assert_eq!(
        rejected_with(&server, me(), "kernel-matrix", Request::Reboot),
        ErrorCode::Cancelled
    );
    assert_eq!(
        server
            .handle(peer(me()), envelope("status", Request::Status))
            .status,
        ResponseStatus::Ok
    );
    assert!(calls.0.lock().unwrap().is_empty());
}

#[test]
fn the_socket_protocol_authenticates_the_peer_with_kernel_credentials() {
    let scratch = Scratch::new("server-socket");
    let (server, _) = server(&scratch, false);
    let server = Arc::new(server);
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
        std::thread::sleep(Duration::from_millis(20));
    }
    let client = Client {
        socket: &socket,
        campaign_id: "core-abc1234-1",
        manifest_sha256: MANIFEST,
    };
    // The test process's own uid is the configured owner, so the kernel-reported peer matches.
    if crate::sys::effective_uid() != 0 {
        let response = client.send("live-observation", observation()).unwrap();
        assert_eq!(response.status, ResponseStatus::Ok, "{response:?}");
        let status = client.send("status", Request::Status).unwrap();
        assert_eq!(status.data.unwrap()["campaign_id"], "core-abc1234-1");
    }
    // A client claiming another campaign is rejected whatever the socket says.
    let stranger = Client {
        socket: &socket,
        campaign_id: "core-other",
        manifest_sha256: MANIFEST,
    };
    let response = stranger.send("live-observation", observation()).unwrap();
    assert_ne!(response.status, ResponseStatus::Ok);
    // Garbage and oversized frames get a structured rejection, not a crash.
    let mut raw = UnixStream::connect(&socket).unwrap();
    std::io::Write::write_all(&mut raw, &(10_000_000_u32).to_be_bytes()).unwrap();
    let response: crate::model::Response = read_frame(&mut raw).unwrap();
    assert_eq!(response.status, ResponseStatus::Rejected);
    let mut malformed = UnixStream::connect(&socket).unwrap();
    write_frame(&mut malformed, &json!({"not": "an envelope"})).unwrap();
    let response: crate::model::Response = read_frame(&mut malformed).unwrap();
    assert_eq!(response.error.unwrap().code, ErrorCode::InvalidArgument);
}
