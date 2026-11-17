use std::os::unix::fs::PermissionsExt;

use super::*;

fn setup() -> (tempfile::TempDir, ResearchIntentV1, CampaignFacts) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut intent = super::super::tests::intent();
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

#[test]
fn launcher_identity_is_frozen_and_absence_keeps_the_original_encoding() {
    let (_, mut intent, mut facts) = setup();
    assert!(
        serde_json::to_value(&intent)
            .unwrap()
            .get("launcher_executable_sha256")
            .is_none()
    );
    assert!(
        serde_json::to_value(&facts)
            .unwrap()
            .get("launcher_executable_sha256")
            .is_none()
    );
    intent.launcher_executable_sha256 = Some("1".repeat(64));
    facts.launcher_executable_sha256 = intent.launcher_executable_sha256.clone();
    assert!(facts.validate(&intent).is_ok());
    facts.launcher_executable_sha256 = Some("2".repeat(64));
    assert!(facts.validate(&intent).is_err());
}

#[test]
fn one_lease_persists_only_ordered_transitions_and_recovers_a_boundary() {
    let (dir, intent, facts) = setup();
    let mut lease =
        CampaignLease::create(fd(&dir), intent.clone(), facts.clone(), 11, 123, 456).unwrap();
    assert!(CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).is_err());
    assert!(lease.advance(ResearchPhase::Launching, 11).is_err());
    lease.advance(ResearchPhase::Preflight, 11).unwrap();
    lease.advance(ResearchPhase::Prepared, 11).unwrap();
    drop(lease);
    let mut lease = CampaignLease::recover(fd(&dir), &facts, 12).unwrap();
    assert_eq!(lease.phase(), ResearchPhase::Prepared);
    lease.advance(ResearchPhase::AwaitSafeWindow, 12).unwrap();
    lease.advance(ResearchPhase::Restoring, 12).unwrap();
    lease.advance(ResearchPhase::Blocked, 12).unwrap();
    assert!(lease.advance(ResearchPhase::Launching, 12).is_err());
    drop(lease);
    assert_eq!(
        CampaignLease::recover(fd(&dir), &facts, 12)
            .unwrap()
            .phase(),
        ResearchPhase::Blocked
    );
}

#[test]
fn changed_source_override_deadline_and_torn_journals_fail_closed() {
    let (dir, intent, facts) = setup();
    let mut override_facts = facts.clone();
    override_facts.validation_override_present = true;
    assert!(CampaignLease::create(fd(&dir), intent.clone(), override_facts, 11, 123, 456).is_err());
    let mut busy = facts.clone();
    busy.mutable_action_present = true;
    assert!(CampaignLease::create(fd(&dir), intent.clone(), busy, 11, 123, 456).is_err());
    let mut duplicate = facts.clone();
    duplicate.tools.push(duplicate.tools[0].clone());
    assert!(CampaignLease::create(fd(&dir), intent.clone(), duplicate, 11, 123, 456).is_err());
    assert!(
        CampaignLease::create(fd(&dir), intent.clone(), facts.clone(), 3610, 123, 456).is_err()
    );
    let mut lease = CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).unwrap();
    lease.advance(ResearchPhase::Preflight, 11).unwrap();
    std::fs::write(dir.path().join("event-0001.json"), b"{}").unwrap();
    assert!(lease.advance(ResearchPhase::Prepared, 11).is_err());
    assert!(!dir.path().join("event-0002.json").exists());
    drop(lease);
    assert!(CampaignLease::recover(fd(&dir), &facts, 12).is_err());
    let (other, intent, facts) = setup();
    let lease = CampaignLease::create(fd(&other), intent, facts.clone(), 11, 123, 456).unwrap();
    drop(lease);
    let mut changed = facts.clone();
    changed.source_sha256 = "e".repeat(64);
    assert!(CampaignLease::recover(fd(&other), &changed, 12).is_err());
    let mut changed_steam = facts.clone();
    changed_steam.steam_client.steamrt64_build_id += 1;
    assert!(CampaignLease::recover(fd(&other), &changed_steam, 12).is_err());
    std::fs::write(other.path().join("event-0001.json.tmp"), b"torn").unwrap();
    assert!(CampaignLease::recover(fd(&other), &facts, 12).is_err());
}

#[test]
fn an_interrupted_launch_cannot_be_replayed() {
    let (dir, intent, facts) = setup();
    let mut lease = CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).unwrap();
    for phase in [
        ResearchPhase::Preflight,
        ResearchPhase::Prepared,
        ResearchPhase::AwaitSafeWindow,
        ResearchPhase::Launching,
    ] {
        lease.advance(phase, 11).unwrap();
    }
    drop(lease);
    assert!(
        matches!(CampaignLease::recover(fd(&dir), &facts, 12), Err(error) if error.to_string().contains("UnsafeToResume"))
    );
}

#[test]
fn expiry_blocks_new_work_but_allows_restoration_and_terminal_record() {
    let (dir, intent, facts) = setup();
    let mut lease = CampaignLease::create(fd(&dir), intent, facts.clone(), 11, 123, 456).unwrap();
    lease.advance(ResearchPhase::Preflight, 11).unwrap();
    lease.advance(ResearchPhase::Prepared, 12).unwrap();
    lease.advance(ResearchPhase::AwaitSafeWindow, 13).unwrap();
    drop(lease);
    let mut lease = CampaignLease::recover(fd(&dir), &facts, 3610).unwrap();
    assert!(lease.advance(ResearchPhase::Launching, 3610).is_err());
    assert!(lease.advance(ResearchPhase::Restoring, 12).is_err());
    lease.advance(ResearchPhase::Restoring, 3610).unwrap();
    lease.advance(ResearchPhase::Blocked, 3611).unwrap();
    drop(lease);
    assert_eq!(
        CampaignLease::recover(fd(&dir), &facts, 3611)
            .unwrap()
            .phase(),
        ResearchPhase::Blocked
    );
}

#[test]
fn event_capacity_covers_the_authorized_attempt_budget() {
    let mut intent = super::super::tests::intent();
    intent.max_attempts = 100;
    assert!(event_limit(&intent) >= 100 * 8);
    intent.max_attempts = 1_000;
    assert_eq!(event_limit(&intent), MAX_EVENTS);
}

#[test]
fn a_campaign_is_refused_when_another_mutator_or_a_validation_override_is_present() {
    // PLAN P0/P1: no conflicting mutator. The host reports whether an unsupported override or another
    // mutating action is present, and a lease is never created over either.
    let (dir, intent, facts) = setup();
    for (override_present, mutator_present) in [(true, false), (false, true), (true, true)] {
        let mut conflicting = facts.clone();
        conflicting.validation_override_present = override_present;
        conflicting.mutable_action_present = mutator_present;
        assert!(
            conflicting.validate(&intent).is_err(),
            "{override_present} {mutator_present}"
        );
        assert!(
            CampaignLease::create(fd(&dir), intent.clone(), conflicting, 11, 123, 456).is_err(),
            "no lease is created over {override_present} {mutator_present}"
        );
    }
    // Nothing was created in the campaign directory by the refused attempts.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    // Without either, the same facts admit.
    assert!(CampaignLease::create(fd(&dir), intent, facts, 11, 123, 456).is_ok());
}
