use std::os::{fd::OwnedFd, unix::fs::PermissionsExt};

use super::*;
use crate::autotune::research::lease::{CampaignFacts, CampaignLease};

fn setup() -> (tempfile::TempDir, ResearchIntentV1, CampaignFacts) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut intent = crate::autotune::research::tests::intent();
    intent.uid = crate::syscall::geteuid();
    let facts = CampaignFacts {
        uid: intent.uid,
        boot_id: intent.boot_id.clone(),
        session_id: intent.session_id.clone(),
        study_root: intent.study_root,
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
    (dir, intent, facts)
}

fn fd(dir: &tempfile::TempDir) -> OwnedFd {
    crate::syscall::open_dir_no_symlinks(dir.path()).unwrap()
}

/// A real lease walked to a terminal phase through the product's own API.
fn settled() -> (tempfile::TempDir, u32) {
    let (dir, intent, facts) = setup();
    let uid = intent.uid;
    let now = intent.issued_unix_seconds + 1;
    let mut lease = CampaignLease::create(fd(&dir), intent, facts, now, 123, 456).unwrap();
    for phase in [
        ResearchPhase::Preflight,
        ResearchPhase::Prepared,
        ResearchPhase::AwaitSafeWindow,
    ] {
        lease.advance(phase, now).unwrap();
    }
    // Any live phase restores, and restoring is what ends a campaign.
    lease.advance(ResearchPhase::Restoring, now).unwrap();
    lease.advance(ResearchPhase::Completed, now).unwrap();
    drop(lease);
    (dir, uid)
}

fn rewrite(dir: &tempfile::TempDir, name: &str, edit: impl FnOnce(String) -> String) {
    let path = dir.path().join(name);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, edit(text)).unwrap();
}

#[test]
fn a_real_settled_lease_is_consistent_and_terminal() {
    let (dir, uid) = settled();
    let inspection = inspect(dir.path(), uid).unwrap();
    assert!(inspection.chain_valid && inspection.transitions_valid);
    assert!(inspection.header_matches_intent && inspection.terminal);
    assert_eq!(inspection.events, 5);
}

#[test]
fn an_interrupted_lease_is_consistent_but_not_settled() {
    let (dir, intent, facts) = setup();
    let uid = intent.uid;
    let now = intent.issued_unix_seconds + 1;
    let mut lease = CampaignLease::create(fd(&dir), intent, facts, now, 123, 456).unwrap();
    for phase in [
        ResearchPhase::Preflight,
        ResearchPhase::Prepared,
        ResearchPhase::AwaitSafeWindow,
        ResearchPhase::Launching,
    ] {
        lease.advance(phase, now).unwrap();
    }
    drop(lease);
    let inspection = inspect(dir.path(), uid).unwrap();
    assert!(inspection.chain_valid && inspection.transitions_valid);
    assert_eq!(inspection.final_phase, ResearchPhase::Launching);
    assert!(
        !inspection.terminal,
        "a launch in flight is not a settled campaign"
    );
}

#[test]
fn an_edited_missing_or_extra_record_breaks_the_chain() {
    // An edited event.
    let (dir, uid) = settled();
    rewrite(&dir, "event-0002.json", |text| {
        text.replace("\"prepared\"", "\"preflight\"")
    });
    assert!(!inspect(dir.path(), uid).unwrap().chain_valid);
    // A missing middle event: the chain ends early and a later record is unaccounted for.
    let (dir, uid) = settled();
    std::fs::remove_file(dir.path().join("event-0002.json")).unwrap();
    assert!(!inspect(dir.path(), uid).unwrap().chain_valid);
    // A record past the end.
    let (dir, uid) = settled();
    std::fs::copy(
        dir.path().join("event-0004.json"),
        dir.path().join("event-0007.json"),
    )
    .unwrap();
    assert!(!inspect(dir.path(), uid).unwrap().chain_valid);
}

#[test]
fn an_impossible_transition_and_a_header_that_is_not_its_intents_digest_are_reported() {
    let (dir, uid) = settled();
    // Rewrite the second event to a transition the state machine does not allow, keeping the shape.
    rewrite(&dir, "event-0002.json", |text| {
        text.replace("\"to\":\"prepared\"", "\"to\":\"launching\"")
    });
    let inspection = inspect(dir.path(), uid).unwrap();
    assert!(!inspection.transitions_valid || !inspection.chain_valid);
    // A header whose digest is not its intent's digest.
    let (dir, uid) = settled();
    rewrite(&dir, "intent.json", |text| {
        let digest = text
            .split("\"intent_sha256\":\"")
            .nth(1)
            .and_then(|rest| rest.get(..64))
            .unwrap()
            .to_owned();
        text.replacen(&digest, &"0".repeat(64), 1)
    });
    assert!(!inspect(dir.path(), uid).unwrap().header_matches_intent);
}

#[test]
fn a_directory_that_is_not_private_is_refused() {
    let (dir, uid) = settled();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(inspect(dir.path(), uid).is_err());
}

/// Read-only on a real lease directory:
/// `STUTTER_REAL_LEASE=<dir> cargo test real_lease_inspection -- --ignored --nocapture`.
#[test]
#[ignore = "needs a real lease directory"]
fn real_lease_inspection() {
    let dir = std::env::var_os("STUTTER_REAL_LEASE").expect("set STUTTER_REAL_LEASE");
    let inspection = inspect(Path::new(&dir), crate::syscall::geteuid()).unwrap();
    println!("{inspection:#?}");
    assert!(
        inspection.chain_valid && inspection.transitions_valid && inspection.header_matches_intent
    );
}

#[test]
fn a_record_far_past_the_end_or_a_stray_event_entry_cannot_hide_from_the_inspection() {
    // The old probe looked only two names past the end of the chain.
    let (dir, uid) = settled();
    std::fs::copy(
        dir.path().join("event-0004.json"),
        dir.path().join("event-0009.json"),
    )
    .unwrap();
    assert!(!inspect(dir.path(), uid).unwrap().chain_valid);
    // A leftover or foreign `event-` entry is not an accounted record either.
    let (dir, uid) = settled();
    std::fs::write(dir.path().join("event-0003.json.tmp"), b"torn").unwrap();
    assert!(!inspect(dir.path(), uid).unwrap().chain_valid);
    let (dir, uid) = settled();
    std::fs::write(dir.path().join("event-zero.json"), b"{}").unwrap();
    assert!(!inspect(dir.path(), uid).unwrap().chain_valid);
    // A record beyond the bound is not a lease record.
    let (dir, uid) = settled();
    std::fs::write(dir.path().join("event-9999.json"), b"{}").unwrap();
    assert!(!inspect(dir.path(), uid).unwrap().chain_valid);
}

#[test]
fn a_record_that_is_present_but_unreadable_is_an_error_not_the_end_of_the_chain() {
    let (dir, uid) = settled();
    let path = dir.path().join("event-0003.json");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let result = inspect(dir.path(), uid);
    // (A superuser can read anything; then the chain is simply valid.)
    if crate::syscall::geteuid() != 0 {
        assert!(result.is_err(), "{result:?}");
    }
    // The same is true of a record that is not valid JSON.
    let (dir, uid) = settled();
    std::fs::write(dir.path().join("event-0003.json"), b"not json").unwrap();
    assert!(inspect(dir.path(), uid).is_err());
}
