use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;

fn fixture() -> (tempfile::TempDir, ResearchIntentV1, AttemptSlot) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut intent = super::super::tests::intent();
    intent.uid = crate::syscall::geteuid();
    let slot = AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index: 0,
        arm: intent.tools[0].source,
    };
    (dir, intent, slot)
}

fn fd(dir: &tempfile::TempDir) -> OwnedFd {
    crate::syscall::open_dir_no_symlinks(dir.path()).unwrap()
}

#[test]
fn ticket_binds_exact_game_tool_session_attempt_and_steam_verb_then_cannot_replay() {
    let (dir, intent, slot) = fixture();
    let ticket = issue_launch_ticket(
        fd(&dir),
        &intent,
        &slot,
        SteamVerb::WaitForExitAndRun,
        11,
        60,
    )
    .unwrap();
    assert_eq!(ticket.uid, intent.uid);
    assert_eq!(ticket.steam_app_id, 1_091_500);
    assert_eq!(ticket.game_build_id, intent.build_id);
    assert_eq!(ticket.tool, intent.tools[0]);
    assert_eq!(
        ticket.steam_client_identity_sha256,
        intent.steam_client.identity_sha256
    );
    assert_eq!(
        ticket.steam_process,
        intent.steam_client.process.clone().unwrap()
    );
    assert_eq!(ticket.verb, SteamVerb::WaitForExitAndRun);
    assert_eq!(ticket.boot_id, intent.boot_id);
    assert_eq!(ticket.session_id, intent.session_id);

    let expected = binding(&intent, &slot, SteamVerb::WaitForExitAndRun).unwrap();
    let consumed = consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 12).unwrap();
    assert_eq!(consumed, ticket);
    assert!(consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 12).is_err());
    assert!(
        dir.path()
            .join(format!("used-{}", ticket.ticket_id))
            .is_file()
    );
}

#[test]
fn retirement_blocks_unclaimed_tickets_and_refuses_an_existing_claim() {
    let (dir, intent, slot) = fixture();
    let ticket = issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 60).unwrap();
    retire_unclaimed_tickets(fd(&dir), intent.uid).unwrap();
    let expected = binding(&intent, &slot, SteamVerb::Run).unwrap();
    assert!(consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 12).is_err());

    let (dir, intent, slot) = fixture();
    let ticket = issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 60).unwrap();
    let expected = binding(&intent, &slot, SteamVerb::Run).unwrap();
    consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 12).unwrap();
    assert!(retire_unclaimed_tickets(fd(&dir), intent.uid).is_err());
}

#[test]
fn retirement_covers_partial_ticket_files_and_rejects_orphan_claims() {
    let (dir, intent, _) = fixture();
    let id = "a".repeat(64);
    std::fs::write(dir.path().join(format!("ticket-{id}.json")), b"partial").unwrap();
    retire_unclaimed_tickets(fd(&dir), intent.uid).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join(format!("used-{id}"))).unwrap(),
        b"revoked-v1"
    );

    let (dir, intent, _) = fixture();
    std::fs::write(dir.path().join(format!("used-{id}")), b"claimed").unwrap();
    assert!(retire_unclaimed_tickets(fd(&dir), intent.uid).is_err());
}

#[test]
fn concurrent_retirement_and_dispatch_cannot_both_win() {
    let (dir, intent, slot) = fixture();
    let ticket = issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 60).unwrap();
    let expected = binding(&intent, &slot, SteamVerb::Run).unwrap();
    let retire_dir = fd(&dir);
    let consume_dir = fd(&dir);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let uid = intent.uid;
    let thread = std::thread::spawn(move || {
        other.wait();
        retire_unclaimed_tickets(retire_dir, uid).is_ok()
    });
    barrier.wait();
    let consumed = consume_launch_ticket(consume_dir, &ticket.ticket_id, &expected, 12).is_ok();
    let retired = thread.join().unwrap();
    assert_ne!(retired, consumed);
}

#[test]
fn wrong_user_app_build_tool_boot_session_verb_and_attempt_are_rejected_without_claiming() {
    let (dir, intent, slot) = fixture();
    let ticket = issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 60).unwrap();
    let expected = binding(&intent, &slot, SteamVerb::Run).unwrap();
    let mut variants = Vec::new();
    let mut wrong = expected.clone();
    wrong.uid += 1;
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.steam_app_id = 1;
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.game_build_id += 1;
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.tool.identity_sha256 = "a".repeat(64);
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.steam_client_identity_sha256 = "b".repeat(64);
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.steam_process.steam_start_ticks += 1;
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.steam_process.logged_in = false;
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.boot_id = "other-boot".into();
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.session_id = "other-session".into();
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.verb = SteamVerb::WaitForExitAndRun;
    variants.push(wrong);
    let mut wrong = expected.clone();
    wrong.attempt_sha256 = "b".repeat(64);
    variants.push(wrong);
    for wrong in variants {
        assert!(consume_launch_ticket(fd(&dir), &ticket.ticket_id, &wrong, 12).is_err());
    }
    assert!(consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 12).is_ok());
}

#[test]
fn expired_ticket_cannot_be_used_and_bad_ids_cannot_escape_the_ticket_directory() {
    let (dir, intent, slot) = fixture();
    let ticket = issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 60).unwrap();
    let expected = binding(&intent, &slot, SteamVerb::Run).unwrap();
    assert!(consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 71).is_err());
    assert!(consume_launch_ticket(fd(&dir), "../outside", &expected, 12).is_err());
    assert!(consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 12).is_ok());
}

#[test]
fn symlink_ticket_and_excessive_lifetime_fail_closed() {
    let (dir, intent, slot) = fixture();
    assert!(issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 301).is_err());
    let ticket = issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 60).unwrap();
    let name = format!("ticket-{}.json", ticket.ticket_id);
    let target = dir.path().join("target");
    std::fs::write(&target, serde_json::to_vec(&ticket).unwrap()).unwrap();
    std::fs::remove_file(dir.path().join(&name)).unwrap();
    symlink(target, dir.path().join(&name)).unwrap();
    let expected = binding(&intent, &slot, SteamVerb::Run).unwrap();
    assert!(consume_launch_ticket(fd(&dir), &ticket.ticket_id, &expected, 12).is_err());
}

#[test]
fn ticket_wire_contains_only_closed_authority_bindings() {
    let (dir, intent, slot) = fixture();
    let ticket = issue_launch_ticket(fd(&dir), &intent, &slot, SteamVerb::Run, 11, 60).unwrap();
    let text = serde_json::to_string(&ticket).unwrap();
    for forbidden in ["path", "command", "environment", "prefix", "executable"] {
        assert!(!text.contains(forbidden));
    }
    let wrong_app: ResearchLaunchTicketV1 =
        serde_json::from_str(&text.replace("\"steam_app_id\":1091500", "\"steam_app_id\":1"))
            .unwrap();
    assert!(validate_ticket(&wrong_app, 12).is_err());
    let mut unknown = serde_json::to_value(ticket).unwrap();
    unknown["command"] = "arbitrary".into();
    assert!(serde_json::from_value::<ResearchLaunchTicketV1>(unknown).is_err());
}

#[test]
fn unqualified_or_destructive_proton_verbs_are_rejected() {
    for verb in [
        "setup",
        "destroyprefix",
        "runinprefix",
        "getnativepath",
        "getcompatpath",
    ] {
        assert!(serde_json::from_value::<SteamVerb>(serde_json::json!(verb)).is_err());
        assert!(SteamVerb::from_steam_argument(verb).is_none());
    }
    for verb in [SteamVerb::Run, SteamVerb::WaitForExitAndRun] {
        assert_eq!(
            SteamVerb::from_steam_argument(verb.as_steam_argument()),
            Some(verb)
        );
    }
    assert!(SteamVerb::from_steam_argument("wait_for_exit_and_run").is_none());
}
