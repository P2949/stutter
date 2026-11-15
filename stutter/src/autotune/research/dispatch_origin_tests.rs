//! The dispatcher must prove Steam started it before it spends a ticket or executes Proton.
use std::path::Path;

use super::*;
use crate::autotune::research::{AttemptSlot, SteamProcessIdentity, dispatch_context::ArmSpec};

fn add(proc: &Path, pid: u32, parent: u32, ticks: u64, comm: &str) {
    let path = proc.join(pid.to_string());
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("stat"),
        format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0"),
    )
    .unwrap();
}

fn context(steam_pid: u32, steam_ticks: u64) -> DispatchContextV1 {
    let mut intent = crate::autotune::research::tests::intent();
    intent.uid = crate::syscall::geteuid();
    intent.steam_client.process = Some(SteamProcessIdentity {
        steam_pid,
        steam_start_ticks: steam_ticks,
        webhelper_pid: steam_pid + 1,
        webhelper_start_ticks: 2,
        logged_in: true,
    });
    let slot = AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index: 0,
        arm: intent.tools[0].source,
    };
    DispatchContextV1 {
        schema_version: 1,
        intent,
        slot,
        tickets: Default::default(),
        spec: ArmSpec {
            tool_dir: "/t".into(),
            study_compat_data: "/s".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        },
    }
}

#[test]
fn only_a_descendant_of_the_exact_frozen_steam_client_is_accepted() {
    let proc = tempfile::tempdir().unwrap();
    // Steam -> reaper -> container wrapper -> dispatcher, as a real launch nests them.
    add(proc.path(), 100, 1, 11, "steam");
    add(proc.path(), 101, 100, 12, "reaper");
    add(proc.path(), 102, 101, 13, "pressure-vessel");
    add(proc.path(), 103, 102, 14, "proton");
    assert!(verify_steam_origin(&context(100, 11), proc.path(), 103).is_ok());
    assert!(verify_steam_origin(&context(100, 11), proc.path(), 101).is_ok());

    // A caller of the same user with a perfectly valid context but no Steam lineage is refused.
    add(proc.path(), 300, 1, 30, "bash");
    add(proc.path(), 301, 300, 31, "proton");
    assert!(verify_steam_origin(&context(100, 11), proc.path(), 301).is_err());
    // Steam itself, or a process outside it, is not a dispatcher launched by Steam.
    assert!(verify_steam_origin(&context(100, 11), proc.path(), 100).is_err());
    assert!(verify_steam_origin(&context(100, 11), proc.path(), 999).is_err());
}

#[test]
fn a_replaced_missing_or_unidentified_steam_client_is_refused() {
    let proc = tempfile::tempdir().unwrap();
    add(proc.path(), 100, 1, 11, "steam");
    add(proc.path(), 101, 100, 12, "proton");
    // The frozen client's start time differs: the PID was reused by another process.
    assert!(verify_steam_origin(&context(100, 12), proc.path(), 101).is_err());
    // The frozen client is not running at all.
    assert!(verify_steam_origin(&context(500, 11), proc.path(), 101).is_err());
    // The intent froze no client process.
    let mut unidentified = context(100, 11);
    unidentified.intent.steam_client.process = None;
    assert!(verify_steam_origin(&unidentified, proc.path(), 101).is_err());
    // A context of another user is refused even with a correct lineage.
    let mut foreign = context(100, 11);
    foreign.intent.uid += 1;
    assert!(verify_steam_origin(&foreign, proc.path(), 101).is_err());
}
