use std::os::unix::fs::PermissionsExt;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::*;

#[tokio::test]
async fn resident_transport_authenticates_and_blocks_without_independent_host_facts() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (tx, mut rx) = mpsc::channel(4);
    let server = ResearchServer::bind(dir.path(), tx.clone()).await.unwrap();
    assert!(ResearchServer::bind(dir.path(), tx).await.is_err());
    let campaign_dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(campaign_dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    // A client that sends no complete request must not block other control traffic.
    let slow = tokio::net::UnixStream::connect(dir.path().join(SOCKET_NAME))
        .await
        .unwrap();
    let mut client = tokio::net::UnixStream::connect(dir.path().join(SOCKET_NAME))
        .await
        .unwrap();
    client.write_all(b"{\"verb\":\"status\"}\n").await.unwrap();
    let envelope = tokio::time::timeout(RESPONSE_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    let mut coordinator = ResearchCoordinator::new(
        Some(campaign_dir.path().to_path_buf()),
        None,
        "BlockedHostIdentity",
    );
    coordinator.handle(envelope, 100);
    let mut line = String::new();
    BufReader::new(client).read_line(&mut line).await.unwrap();
    assert_eq!(
        line,
        "{\"code\":\"BlockedHostIdentity\",\"status\":\"idle\"}\n"
    );
    drop(slow);
    let mut submit = tokio::net::UnixStream::connect(dir.path().join(SOCKET_NAME))
        .await
        .unwrap();
    let intent = serde_json::json!({
        "schema_version": 1, "game": "cyberpunk2077", "app_id": 1091500,
        "uid": crate::syscall::geteuid(), "boot_id": "boot-1", "session_id": "11",
        "steam_client": {
            "branch": "publicbeta", "steamrt64_build_id": 1,
            "steamrt64_manifest_sha256": "e".repeat(64),
            "installed_index_sha256": "f".repeat(64), "identity_sha256": "1".repeat(64),
            "process": {
                "steam_pid": 10, "steam_start_ticks": 1,
                "webhelper_pid": 11, "webhelper_start_ticks": 2, "logged_in": true
            }
        },
        "source_sha256": "a".repeat(64), "policy_sha256": "b".repeat(64),
        "game_executable_sha256": "c".repeat(64), "build_id": 1,
        "tools": [{"source":"ge", "identity_sha256":"d".repeat(64)}],
        "factors": [], "actions": [], "issued_unix_seconds": 10,
        "deadline_unix_seconds": 3610, "max_active_seconds": 3600,
        "max_attempt_seconds": 720, "max_owned_pressure_seconds_per_study": 600,
        "max_attempts": 10, "max_evidence_bytes": 1024,
        "desktop_research_consented": true
    });
    submit
        .write_all(
            format!(
                "{}\n",
                serde_json::json!({"verb":"submit", "intent":intent})
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let envelope = tokio::time::timeout(RESPONSE_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    coordinator.handle(envelope, 100);
    let mut response = String::new();
    BufReader::new(submit)
        .read_line(&mut response)
        .await
        .unwrap();
    assert_eq!(
        response,
        "{\"code\":\"BlockedHostIdentity\",\"status\":\"blocked\"}\n"
    );
    let socket_dir = dir.path().to_path_buf();
    let cli = tokio::spawn(async move {
        crate::commands::research::run(crate::commands::research::ResearchClientInput {
            socket_dir: Some(socket_dir),
            action: crate::commands::research::ResearchClientAction::Status,
        })
        .await
    });
    let envelope = tokio::time::timeout(RESPONSE_TIMEOUT, rx.recv())
        .await
        .unwrap()
        .unwrap();
    coordinator.handle(envelope, 100);
    cli.await.unwrap().unwrap();
    drop(server);
    assert!(!dir.path().join(SOCKET_NAME).exists());
}

fn request(message: &ResearchRequest) -> AuthenticatedRequest {
    let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    serde_json::to_writer(&mut client, message).unwrap();
    client.write_all(b"\n").unwrap();
    receive_request(&server, crate::syscall::geteuid()).unwrap()
}

fn fixture() -> (crate::autotune::research::ResearchIntentV1, CampaignFacts) {
    use crate::autotune::research::{
        ProtonSource, ResearchGame, ResearchIntentV1, ResearchTool, SteamClientIdentity,
        SteamProcessIdentity,
    };

    let intent = ResearchIntentV1 {
        schema_version: 1,
        game: ResearchGame::Cyberpunk2077,
        app_id: 1_091_500,
        uid: crate::syscall::geteuid(),
        boot_id: "boot-1".into(),
        session_id: "11".into(),
        steam_client: SteamClientIdentity {
            branch: "publicbeta".into(),
            steamrt64_build_id: 1,
            steamrt64_manifest_sha256: "e".repeat(64),
            installed_index_sha256: "f".repeat(64),
            identity_sha256: "1".repeat(64),
            process: Some(SteamProcessIdentity {
                steam_pid: 10,
                steam_start_ticks: 1,
                webhelper_pid: 11,
                webhelper_start_ticks: 2,
                logged_in: true,
            }),
        },
        source_sha256: "a".repeat(64),
        policy_sha256: "b".repeat(64),
        game_executable_sha256: "c".repeat(64),
        launcher_executable_sha256: None,
        build_id: 1,
        tools: vec![ResearchTool {
            source: ProtonSource::Ge,
            identity_sha256: "d".repeat(64),
        }],
        factors: vec![],
        actions: vec![],
        issued_unix_seconds: 10,
        deadline_unix_seconds: 3_610,
        max_active_seconds: 3_600,
        max_attempt_seconds: 720,
        max_owned_pressure_seconds_per_study: 600,
        max_attempts: 10,
        max_evidence_bytes: 1_024,
        desktop_research_consented: true,
    };
    let facts = CampaignFacts {
        uid: intent.uid,
        boot_id: intent.boot_id.clone(),
        session_id: intent.session_id.clone(),
        steam_client: intent.steam_client.clone(),
        source_sha256: intent.source_sha256.clone(),
        policy_sha256: intent.policy_sha256.clone(),
        game_executable_sha256: intent.game_executable_sha256.clone(),
        launcher_executable_sha256: intent.launcher_executable_sha256.clone(),
        build_id: intent.build_id,
        tools: intent.tools.clone(),
        validation_override_present: false,
        mutable_action_present: false,
    };
    (intent, facts)
}

#[test]
fn resident_status_tracks_typed_host_readiness_and_steam_login_blockers() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (_, facts) = fixture();
    let mut coordinator =
        ResearchCoordinator::new(Some(dir.path().to_path_buf()), None, "BlockedHostIdentity");
    assert!(coordinator.status().contains("BlockedHostIdentity"));
    coordinator.set_host_context(Err(crate::autotune::research::host::HostFactsError {
        code: "BlockedSteamLogin",
    }));
    assert!(coordinator.status().contains("BlockedSteamLogin"));
    coordinator.set_host_context(Ok(facts.clone()));
    let status: serde_json::Value = serde_json::from_str(&coordinator.status()).unwrap();
    assert_eq!(status["code"], "ReadyForIntent");
    assert_eq!(status["execution"], "BlockedSteamAutomation");
    assert_eq!(status["facts"]["source_sha256"], facts.source_sha256);
    assert!(!status.to_string().contains("/home/"));
    coordinator.set_mutable_action_present(true);
    assert!(coordinator.status().contains("BlockedMutationSlot"));
}

fn send(coordinator: &mut ResearchCoordinator, message: ResearchRequest, now: u64) -> String {
    let (reply, answer) = oneshot::channel();
    coordinator.handle(
        Envelope {
            request: request(&message),
            reply,
        },
        now,
    );
    answer.blocking_recv().unwrap()
}

#[test]
fn one_verified_intent_is_durable_pausable_cancellable_and_terminal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut coordinator = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts.clone()),
        "BlockedHostIdentity",
    );

    let accepted = send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent.clone()),
        },
        100,
    );
    let accepted: serde_json::Value = serde_json::from_str(&accepted).unwrap();
    assert_eq!(accepted["status"], "ok");
    assert_eq!(accepted["phase"], "await_safe_window");

    let second = send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        101,
    );
    assert!(second.contains("SingleCampaignLimit"));

    let paused = send(&mut coordinator, ResearchRequest::Pause {}, 101);
    assert!(paused.contains("\"paused\""));
    let resumed = send(&mut coordinator, ResearchRequest::Resume {}, 102);
    assert!(resumed.contains("\"await_safe_window\""));
    let cancelled = send(&mut coordinator, ResearchRequest::Cancel {}, 103);
    assert!(cancelled.contains("\"aborted\""));
    let status = send(&mut coordinator, ResearchRequest::Status {}, 103);
    assert!(status.contains("\"aborted\""));

    drop(coordinator);
    let recovered = CampaignLease::recover(
        crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
        &facts,
        103,
    )
    .unwrap();
    assert_eq!(recovered.phase(), ResearchPhase::Aborted);
    drop(recovered);

    let mut restarted = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    let status = send(&mut restarted, ResearchRequest::Status {}, 103);
    assert!(status.contains("UnsafeToResume"));
}

#[test]
fn deadline_seals_a_waiting_or_paused_campaign_without_a_client_request() {
    for paused in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (intent, facts) = fixture();
        let mut coordinator = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts.clone()),
            "BlockedHostIdentity",
        );
        send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        if paused {
            send(&mut coordinator, ResearchRequest::Pause {}, 101);
        }
        coordinator.on_deadline(3_609);
        assert!(coordinator.active.is_some());
        coordinator.on_deadline(3_610);
        let status = send(&mut coordinator, ResearchRequest::Status {}, 3_610);
        assert!(status.contains("DeadlineExpired") && status.contains("blocked"));
        drop(coordinator);
        let recovered = CampaignLease::recover(
            crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
            &facts,
            3_610,
        )
        .unwrap();
        assert_eq!(recovered.phase(), ResearchPhase::Blocked);
        assert_eq!(
            recovered.block_reason(),
            Some(crate::autotune::research::CampaignBlockReason::DeadlineExpired)
        );
    }
}

#[path = "research_control_recovery_tests.rs"]
mod recovery_tests;

#[test]
fn intent_without_verified_facts_never_creates_campaign_records() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, _) = fixture();
    let mut coordinator = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        None,
        "BlockedSourceIdentity",
    );
    let response = send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    assert!(response.contains("BlockedSourceIdentity"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn public_campaign_directory_is_rejected_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let (intent, facts) = fixture();
    let mut coordinator = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    let result = send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    assert!(result.contains("BlockedResearchStorage"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn independent_controllers_cannot_own_the_same_campaign_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut first = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts.clone()),
        "BlockedHostIdentity",
    );
    let mut second = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );

    let admitted = send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent.clone()),
        },
        100,
    );
    assert!(admitted.contains("await_safe_window"));
    let refused = send(
        &mut second,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    assert!(refused.contains("CampaignAlreadyOwned"));
}

#[test]
fn cancellation_during_an_unsettled_launch_does_not_claim_restoration() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let digest = intent.digest().unwrap();
    let mut lease = CampaignLease::create(
        crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
        intent,
        facts.clone(),
        100,
        std::process::id(),
        1,
    )
    .unwrap();
    for phase in [
        ResearchPhase::Preflight,
        ResearchPhase::Prepared,
        ResearchPhase::AwaitSafeWindow,
        ResearchPhase::Launching,
    ] {
        lease.advance(phase, 100).unwrap();
    }
    let mut coordinator = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts.clone()),
        "BlockedHostIdentity",
    );
    coordinator.active = Some(ActiveCampaign {
        lease,
        intent_sha256: digest,
        deadline_unix_seconds: 3_610,
    });
    coordinator.submitted = true;
    coordinator.on_deadline(3_610);
    let result = send(&mut coordinator, ResearchRequest::Cancel {}, 101);
    assert!(result.contains("CancellationNeedsExecutorSettlement"));
    assert_eq!(
        coordinator.active.as_ref().unwrap().lease.phase(),
        ResearchPhase::Launching
    );
    drop(coordinator);
    assert!(
        CampaignLease::recover(
            crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
            &facts,
            101,
        )
        .is_err()
    );
}

#[tokio::test]
async fn cleanup_preserves_a_replacement_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let server = ResearchServer::bind(dir.path(), tx).await.unwrap();
    let path = dir.path().join(SOCKET_NAME);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"keep").unwrap();
    drop(server);
    assert_eq!(std::fs::read(path).unwrap(), b"keep");
}

#[tokio::test]
async fn cleanup_stays_with_the_opened_directory_after_rename() {
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("control");
    let moved = root.path().join("moved");
    std::fs::create_dir(&original).unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let server = ResearchServer::bind(&original, tx).await.unwrap();
    std::fs::rename(&original, &moved).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::write(original.join(SOCKET_NAME), b"keep").unwrap();
    drop(server);
    assert_eq!(std::fs::read(original.join(SOCKET_NAME)).unwrap(), b"keep");
    assert!(!moved.join(SOCKET_NAME).exists());
}

#[tokio::test]
async fn stale_socket_is_recovered_but_a_live_or_non_socket_path_is_preserved() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = dir.path().join(SOCKET_NAME);
    let old = std::os::unix::net::UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let (tx, _rx) = mpsc::channel(1);
    assert!(ResearchServer::bind(dir.path(), tx.clone()).await.is_err());
    assert!(path.exists());
    drop(old);
    let server = ResearchServer::bind(dir.path(), tx.clone()).await.unwrap();
    drop(server);
    std::fs::write(&path, b"keep").unwrap();
    assert!(ResearchServer::bind(dir.path(), tx).await.is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"keep");
}
