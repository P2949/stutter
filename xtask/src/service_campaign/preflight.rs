//! Host checks are read-only and retain measured prerequisites instead of assuming Nix state.
use std::{fs, os::unix::fs::MetadataExt, path::Path, process::Command};

use anyhow::{Context, ensure};
use serde_json::{Value, json};

use super::{model::State, runner};
use crate::validation_campaign::model::{AttemptOutcome, BlockReason, FailureClass};

pub(super) fn check(state: &State) -> anyhow::Result<(AttemptOutcome, Value)> {
    runner::integrity(&state.repository, state)?;
    for path in [
        "/run/systemd/system",
        "/sys/kernel/btf/vmlinux",
        "/sys/fs/cgroup/cgroup.controllers",
    ] {
        ensure!(
            Path::new(path).exists(),
            "service prerequisite {path} missing"
        );
    }
    let user = stutter_validation_runner::service::account_name(
        &fs::read_to_string("/etc/passwd")?,
        state.uid,
        state.gid,
    )
    .map_err(|(_, message)| anyhow::anyhow!(message))?;
    let mut persistence = Vec::new();
    for (kind, uid) in [("autotune", state.uid), ("worker", 0)] {
        let path = Path::new("/var/lib/stutter").join(format!("{kind}-{user}"));
        let metadata = fs::metadata(&path)?;
        ensure!(
            metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o777 == 0o700,
            "service state directory has incorrect ownership or mode: {}",
            path.display()
        );
        let output = Command::new("findmnt")
            .args(["--json", "--target"])
            .arg(&path)
            .args(["--output", "TARGET,SOURCE,FSTYPE,FSROOT,OPTIONS"])
            .output()?;
        ensure!(
            output.status.success(),
            "could not inspect service state backing mount"
        );
        let mount: Value = serde_json::from_slice(&output.stdout)?;
        let mounts = mount["filesystems"]
            .as_array()
            .context("missing state mount readback")?;
        ensure!(
            mounts.len() == 1,
            "service state has ambiguous mount backing"
        );
        let record = &mounts[0];
        ensure!(
            record["target"].as_str() == path.to_str()
                && record["fsroot"]
                    .as_str()
                    .is_some_and(|root| root.starts_with("/@persist/")),
            "this host qualification requires the explicit persisted state mount"
        );
        persistence
            .push(json!({"role":kind,"uid":uid,"mode":metadata.mode() & 0o777,"mount":record}));
    }
    let mut cache = stutter::api::focus::FocusCache::default();
    stutter::api::focus::focus_snapshot_at(Path::new("/proc"), &mut cache, 0, None);
    std::thread::sleep(std::time::Duration::from_secs(1));
    let snapshot =
        stutter::api::focus::focus_snapshot_at(Path::new("/proc"), &mut cache, 1000, None);
    let games: Vec<_> = snapshot
        .groups
        .iter()
        .filter(|group| {
            group.kind == stutter::api::focus::FocusGroupKind::Game && group.confidence >= 0.85
        })
        .map(|group| json!({"root_pids":group.root_pids,"confidence":group.confidence}))
        .collect();
    let outcome = if games.is_empty() {
        AttemptOutcome::Passed
    } else {
        AttemptOutcome::Blocked(BlockReason { class: FailureClass::BlockedResource, detail: "an unrelated qualified game is active; preserve its session and defer controlled focus acceptance".into() })
    };
    Ok((
        outcome,
        json!({"source_commit":state.source_commit,"boot_id":stutter_core::boot::current_boot_id()?,"persistence":persistence,"unrelated_games":games}),
    ))
}
