use super::*;

pub(super) fn intent() -> ResearchIntentV1 {
    ResearchIntentV1 {
        schema_version: 1,
        game: ResearchGame::Cyberpunk2077,
        app_id: 1_091_500,
        uid: 1000,
        boot_id: "boot-1".into(),
        session_id: "11".into(),
        study_root: None,
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
        deadline_unix_seconds: 3610,
        max_active_seconds: 3600,
        max_attempt_seconds: 720,
        max_owned_pressure_seconds_per_study: 600,
        max_attempts: 10,
        max_evidence_bytes: 1024,
        desktop_research_consented: true,
        child_study_sha256: None,
        confirmation_rule: None,
        long_attempt_seconds: None,
        prefix_cohort: None,
        prefix_init_arms: Vec::new(),
        initialization: false,
        treatment: None,
    }
}

#[test]
fn contract_roundtrip_and_digest_are_stable() {
    let value = intent();
    let encoded = serde_json::to_vec(&value).unwrap();
    let decoded: ResearchIntentV1 = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value, decoded);
    assert_eq!(value.digest().unwrap(), decoded.digest().unwrap());
    let mut changed = decoded;
    changed.max_attempts += 1;
    assert_ne!(value.digest().unwrap(), changed.digest().unwrap());
}

#[test]
fn malformed_or_unbounded_requests_are_rejected() {
    let mut value = intent();
    value.tools.push(value.tools[0].clone());
    assert!(value.validate().is_err());
    value = intent();
    value.policy_sha256 = "A".repeat(64);
    assert!(value.validate().is_err());
    value = intent();
    value.steam_client.branch = "public".into();
    assert!(value.validate().is_err());
    value = intent();
    value.steam_client.process.as_mut().unwrap().logged_in = false;
    assert!(value.validate().is_err());
    value = intent();
    value.deadline_unix_seconds = 50_000;
    assert!(value.validate().is_err());
    value = intent();
    value.desktop_research_consented = false;
    assert!(value.validate().is_err());
    value = intent();
    value.actions = vec![ResearchAction::CpuAffinityProfile; 2];
    assert!(value.validate().is_err());
    let mut json = serde_json::to_value(intent()).unwrap();
    json["command"] = "arbitrary".into();
    assert!(serde_json::from_value::<ResearchIntentV1>(json).is_err());
    let mut json = serde_json::to_value(intent()).unwrap();
    json["game"] = "unknown".into();
    assert!(serde_json::from_value::<ResearchIntentV1>(json).is_err());
}

#[test]
fn terminal_states_require_restoration_and_cannot_restart() {
    use ResearchPhase::*;
    assert!(Requested.permits(Preflight));
    assert!(Measuring.permits(Restoring));
    assert!(!Measuring.permits(Completed));
    assert!(Restoring.permits(Blocked));
    assert!(!Blocked.permits(Launching));
    assert!(!Sealing.permits(Launching));
}
