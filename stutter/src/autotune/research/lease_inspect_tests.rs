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
fn a_self_consistent_last_event_cannot_hide_bad_time_or_block_reason() {
    for corrupt in ["time", "reason"] {
        let (dir, uid) = settled();
        let path = dir.path().join("event-0005.json");
        let mut event: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match corrupt {
            "time" => event["at_unix_seconds"] = 0.into(),
            _ => event["block_reason"] = "gpu_health_incident".into(),
        }
        std::fs::write(path, serde_json::to_vec(&event).unwrap()).unwrap();
        let inspection = inspect(dir.path(), uid).unwrap();
        assert!(
            inspection.chain_valid,
            "the unchanged prior digest still links"
        );
        assert!(!inspection.transitions_valid, "accepted {corrupt}");
    }
}

#[test]
fn a_self_consistent_preflight_event_after_authorization_expires_is_invalid() {
    let (dir, intent, facts) = setup();
    let uid = intent.uid;
    let now = intent.issued_unix_seconds + 1;
    let mut lease = CampaignLease::create(fd(&dir), intent.clone(), facts, now, 123, 456).unwrap();
    lease.advance(ResearchPhase::Preflight, now).unwrap();
    drop(lease);
    let path = dir.path().join("event-0001.json");
    let mut event: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    event["at_unix_seconds"] = intent.deadline_unix_seconds.into();
    std::fs::write(path, serde_json::to_vec(&event).unwrap()).unwrap();
    let inspection = inspect(dir.path(), uid).unwrap();
    assert!(inspection.chain_valid);
    assert!(!inspection.transitions_valid);
}

#[test]
fn a_self_consistent_chain_cannot_exceed_its_intent_event_limit() {
    let (dir, intent, facts) = setup();
    let uid = intent.uid;
    let now = intent.issued_unix_seconds + 1;
    let limit = event_limit(&intent);
    let mut lease = CampaignLease::create(fd(&dir), intent, facts, now, 123, 456).unwrap();
    for phase in [
        ResearchPhase::Preflight,
        ResearchPhase::Prepared,
        ResearchPhase::AwaitSafeWindow,
    ] {
        lease.advance(phase, now).unwrap();
    }
    drop(lease);
    let mut previous =
        stutter_core::hash::sha256_hex(&std::fs::read(dir.path().join("event-0003.json")).unwrap());
    let mut phase = ResearchPhase::AwaitSafeWindow;
    for sequence in 4..=limit + 1 {
        let next = match phase {
            ResearchPhase::AwaitSafeWindow => ResearchPhase::Launching,
            ResearchPhase::Launching => ResearchPhase::Sealing,
            ResearchPhase::Sealing => ResearchPhase::Analysing,
            ResearchPhase::Analysing => ResearchPhase::NextArm,
            ResearchPhase::NextArm => ResearchPhase::AwaitSafeWindow,
            _ => unreachable!(),
        };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "sequence": sequence, "from": phase, "to": next,
            "previous_sha256": previous, "at_unix_seconds": now,
        }))
        .unwrap();
        let path = dir.path().join(format!("event-{sequence:04}.json"));
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        previous = stutter_core::hash::sha256_hex(&bytes);
        phase = next;
    }
    let inspection = inspect(dir.path(), uid).unwrap();
    assert!(!inspection.chain_valid);
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

#[test]
fn the_lock_table_is_read_for_a_flock_on_exactly_this_directory() {
    // /proc/locks: major and minor in hex, the inode in decimal; only FLOCK entries are leases' locks.
    let table = "1: FLOCK  ADVISORY  WRITE 2043 00:19:7340 0 EOF\n\
                 2: POSIX  ADVISORY  WRITE 2044 08:02:131 0 EOF\n\
                 3: FLOCK  ADVISORY  WRITE 2045 fd:01:99 0 EOF\n";
    // Device 0x19 = major 0, minor 25.
    assert!(locks_show_flock(table, 25, 7340));
    // Another inode and a POSIX lock on the right file fail; another anonymous device with the same inode
    // matches by design (see `an_anonymous_device_filesystem_is_matched_by_inode...`).
    assert!(!locks_show_flock(table, 25, 7341));
    assert!(!locks_show_flock(table, 0x802, 131));
    // A device with a major number above 255 (the high bits of `st_dev`) is decoded, not truncated.
    let wide = "1: FLOCK  ADVISORY  WRITE 1 103:01:5 0 EOF\n";
    assert!(locks_show_flock(wide, (0x103u64 << 8) | 1, 5));
    assert!(!locks_show_flock("", 25, 7340));
}

#[test]
fn a_live_owner_of_the_lease_is_seen_without_inspection_taking_any_lock() {
    use std::os::fd::AsFd;
    let (dir, uid) = settled();
    // Nothing holds the lease: no live owner.
    assert_eq!(inspect(dir.path(), uid).unwrap().owner_live, Some(false));
    // A process that holds the lease's exclusive lock on the directory (what `CampaignLease` does).
    let held = std::fs::File::open(dir.path()).unwrap();
    assert!(crate::syscall::flock_exclusive_nonblocking(held.as_fd()));
    assert_eq!(inspect(dir.path(), uid).unwrap().owner_live, Some(true));
    // Inspecting twice changed nothing about who holds it: the owner still does, and releasing frees it.
    assert_eq!(inspect(dir.path(), uid).unwrap().owner_live, Some(true));
    drop(held);
    assert_eq!(inspect(dir.path(), uid).unwrap().owner_live, Some(false));
}

#[test]
fn an_anonymous_device_filesystem_is_matched_by_inode_and_a_real_device_is_not() {
    // btrfs: `stat` says device 0x24 (anonymous, major 0) but the lock table names the filesystem's 0x20.
    let table = "1: FLOCK  ADVISORY  WRITE 7 00:20:7449307 0 EOF\n";
    assert!(locks_show_flock(table, 0x24, 7_449_307));
    assert!(!locks_show_flock(table, 0x24, 7_449_308));
    // A file on a real block device (major != 0) is never matched by inode alone.
    let real = "1: FLOCK  ADVISORY  WRITE 7 08:02:131 0 EOF\n";
    assert!(!locks_show_flock(real, (0x8u64 << 8) | 3, 131));
    assert!(locks_show_flock(real, (0x8u64 << 8) | 2, 131));
}

#[test]
fn a_real_lease_records_an_identified_peer_intact_facts_and_an_admission_inside_the_window() {
    let (dir, uid) = settled();
    let admission = inspect(dir.path(), uid).unwrap().admission;
    assert_eq!(
        admission,
        Admission {
            peer_identified: true,
            facts_consistent: true,
            admitted_in_window: true
        }
    );
}

#[test]
fn each_part_of_the_recorded_admission_fails_on_its_own() {
    let admission = |edit: &dyn Fn(String) -> String| {
        let (dir, uid) = settled();
        rewrite(&dir, "intent.json", edit);
        inspect(dir.path(), uid).unwrap().admission
    };
    // No peer was identified (the submitter's pid or start time recorded as zero).
    let unidentified = admission(&|text| text.replace("\"peer_pid\":123", "\"peer_pid\":0"));
    assert_eq!(
        unidentified,
        Admission {
            peer_identified: false,
            facts_consistent: true,
            admitted_in_window: true
        }
    );
    let no_start =
        admission(&|text| text.replace("\"peer_start_ticks\":456", "\"peer_start_ticks\":0"));
    assert!(!no_start.peer_identified);
    // The recorded facts were edited after their digest was taken.
    let edited = admission(&|text| text.replacen("\"build_id\":", "\"build_id\":9", 1));
    assert!(!edited.facts_consistent && edited.peer_identified);
    // Admitted before the intent was issued, and at its deadline.
    let early = admission(&|text| {
        let at = text
            .split("\"admitted_unix_seconds\":")
            .nth(1)
            .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
            .unwrap()
            .to_owned();
        text.replace(
            &format!("\"admitted_unix_seconds\":{at}"),
            "\"admitted_unix_seconds\":1",
        )
    });
    assert!(!early.admitted_in_window && early.facts_consistent);
    // A header that records no admission at all (an older lease) says so rather than passing.
    let bare = admission(&|text| {
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("peer_pid");
        object.remove("facts");
        serde_json::to_string(&value).unwrap()
    });
    assert_eq!(
        bare,
        Admission {
            peer_identified: false,
            facts_consistent: false,
            admitted_in_window: false
        }
    );
}
