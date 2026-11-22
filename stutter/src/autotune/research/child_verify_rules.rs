use std::{fs::File, os::fd::AsFd};

use super::{
    ResearchIntentV1,
    child_verify::{AttemptProcess, ChildVerification},
    report_store::read_private,
};

/// Compare the ledger's claimed consumption with the independently rebuilt child report.
pub(super) fn attempt_count_matches(
    counts: &std::collections::BTreeMap<String, u32>,
    dispositions: &[super::child_ledger::Disposition],
    name: &str,
    report: &serde_json::Value,
) -> anyhow::Result<()> {
    use super::child_study::StudyResult;
    let study = dispositions
        .iter()
        .find(|study| study.study_sha256.starts_with(name))
        .ok_or_else(|| anyhow::anyhow!("child is outside the frozen plan"))?;
    anyhow::ensure!(
        matches!(
            study.result,
            Some(StudyResult::Qualified | StudyResult::Unqualified | StudyResult::Failed)
        ),
        "child has no recorded executed disposition"
    );
    let claimed = counts
        .get(&study.study_sha256)
        .ok_or_else(|| anyhow::anyhow!("child attempt count is absent from the ledger"))?;
    anyhow::ensure!(
        report["attempts_sealed"].as_u64() == Some(u64::from(*claimed)),
        "child attempt count disagrees with its verified journal"
    );
    Ok(())
}

/// Every planned holdout must have its own independently reconstructed replication judgement.
pub(super) fn holdout_replication(
    children: &[super::child_verify::ChildVerification],
    planned: &[super::child_ledger::Disposition],
) -> Option<bool> {
    use super::child_study::{Stage, StudyResult};
    let holdouts: Vec<_> = planned
        .iter()
        .filter(|study| study.stage == Stage::Holdout)
        .collect();
    if holdouts.is_empty() {
        return None;
    }
    let mut replicates = true;
    for study in holdouts {
        if study.result != Some(StudyResult::Qualified) {
            return None;
        }
        let matching: Vec<_> = children
            .iter()
            .filter(|child| study.study_sha256.get(..16) == Some(child.study.as_str()))
            .collect();
        let [child] = matching.as_slice() else {
            return None;
        };
        if !child.reconstruction {
            return None;
        }
        let judgement = child.replication.as_deref()?;
        replicates &= super::replication::Replication::name_supports(judgement);
    }
    Some(replicates)
}

/// Every receipt names the permitted action, the frozen process (start time and owner), its
/// experiment, and shows the same placement before and after: the same cgroup and leader CPUs, the
/// same process identity, and every thread the original saw (that still exists) back where it was.
pub(super) fn receipts_valid(
    receipts: &serde_json::Value,
    action: super::ResearchAction,
    uid: u32,
) -> anyhow::Result<()> {
    let items = receipts
        .as_array()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| anyhow::anyhow!("the receipts are empty"))?;
    for item in items {
        anyhow::ensure!(
            item["action"] == format!("{action:?}"),
            "a receipt names another action"
        );
        let pid = item["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .ok_or_else(|| anyhow::anyhow!("a receipt does not identify the frozen process"))?;
        let start_ticks = item["start_ticks"]
            .as_u64()
            .filter(|ticks| *ticks > 0)
            .ok_or_else(|| anyhow::anyhow!("a receipt does not identify the frozen process"))?;
        anyhow::ensure!(
            item["uid"].as_u64() == Some(u64::from(uid)),
            "a receipt does not identify the frozen process"
        );
        anyhow::ensure!(
            item["transaction"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "a receipt names no experiment"
        );
        let before: super::action_gate::KernelState =
            serde_json::from_value(item["original"].clone())?;
        let after: super::action_gate::KernelState =
            serde_json::from_value(item["restored"].clone())?;
        let expected_identity = (start_ticks, uid);
        anyhow::ensure!(
            !before.cgroup.is_empty()
                && !before.cpus_allowed.is_empty()
                && before.cgroup == after.cgroup
                && before.cpus_allowed == after.cpus_allowed
                && before.identity == Some(expected_identity)
                && after.identity == Some(expected_identity)
                && !before.tasks.is_empty()
                && before.tasks.contains_key(&pid)
                && before.tasks.values().all(|cpus| !cpus.is_empty())
                && after.tasks.values().all(|cpus| !cpus.is_empty()),
            "a receipt shows a different placement or process after the restore"
        );
        // An absent task is not proof that it exited; that would need a separate identity witness.
        let moved = before
            .tasks
            .iter()
            .any(|(tid, cpus)| after.tasks.get(tid) != Some(cpus));
        anyhow::ensure!(!moved, "a thread was not restored");
    }
    Ok(())
}

/// A pilot's recorded rule is recomputed from its sealed whole-run results; any difference is refused.
pub(super) fn rule_matches_pilot(
    rules: &std::collections::BTreeMap<String, super::pilot_rule::FrozenConfirmationRule>,
    name: &str,
    studies: std::os::fd::BorrowedFd<'_>,
    uid: u32,
) -> anyhow::Result<()> {
    let Some(recorded) = rules
        .iter()
        .find(|(digest, _)| digest.get(..16) == Some(name))
        .map(|(_, rule)| rule)
    else {
        return Ok(());
    };
    let dir = File::from(crate::syscall::openat_dir_nofollow(studies, name)?);
    let evidence = File::from(crate::syscall::openat_dir_nofollow(
        dir.as_fd(),
        "evidence",
    )?);
    let items = super::report_store::read_evidence(&evidence, uid)?;
    let (tail, runs) = super::metric_plan::run_values(
        items
            .values()
            .filter_map(|item| item.native_completion.as_deref()),
    );
    anyhow::ensure!(
        !runs.is_empty(),
        "the pilot's runs do not support a rule: PilotTooThin"
    );
    let child: ResearchIntentV1 = serde_json::from_slice(&read_private(&dir, "intent.json", uid)?)?;
    let sources: Vec<_> = child.tools.iter().map(|tool| tool.source).collect();
    let recomputed = super::pilot_rule::rule_from_all_pilots(&sources, &runs)
        .map(|rule| rule.with_tail_metric(tail))
        .map_err(|code| anyhow::anyhow!("the pilot's runs do not support a rule: {code}"))?;
    anyhow::ensure!(
        recomputed == *recorded,
        "the recorded confirmation rule is not what the pilot's sealed runs freeze"
    );
    Ok(())
}

/// Studies the ledger decided after running them whose directory is absent.
pub(super) fn missing_children(
    campaign: &File,
    uid: u32,
    parent: &ResearchIntentV1,
    present: &[String],
) -> anyhow::Result<Vec<String>> {
    use super::child_study::StudyResult::{Failed, Qualified, Unqualified};
    Ok(
        super::child_ledger::read_dispositions(campaign, uid, parent)?
            .into_iter()
            .filter(|study| matches!(study.result, Some(Qualified | Unqualified | Failed)))
            .map(|study| study.study_sha256[..16].to_owned())
            .filter(|name| !present.contains(name))
            .collect(),
    )
}

/// Studies the ledger decided after running them, whose directory is absent: a subset of valid
/// children must not stand for the whole recorded funnel.
/// The receipts an action child recorded in its study directory, if any.
pub(super) fn action_receipts(studies: &File, name: &str, uid: u32) -> Option<serde_json::Value> {
    study_file(studies, name, uid, "action-receipts.json")
}

/// The journal slots of a child's completed attempts, from its sealed evidence directory.
pub(super) fn completed_attempts(studies: &File, name: &str, uid: u32) -> Vec<u32> {
    let Ok(dir) = crate::syscall::openat_dir_nofollow(studies.as_fd(), name) else {
        return Vec::new();
    };
    let dir = File::from(dir);
    let Ok(evidence) = crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence") else {
        return Vec::new();
    };
    super::report_store::read_evidence(&File::from(evidence), uid)
        .map(|items| items.into_keys().collect())
        .unwrap_or_default()
}

/// What a study's optional JSON file is: not there, there but unreadable or not JSON, or a document.
pub(super) enum FileState {
    Absent,
    Invalid,
    Present(serde_json::Value),
}

/// As `study_file`, but telling a file that is absent from one that is present and corrupt.
pub(super) fn study_file_state(studies: &File, name: &str, uid: u32, file: &str) -> FileState {
    let Ok(dir) = crate::syscall::openat_dir_nofollow(studies.as_fd(), name) else {
        return FileState::Invalid;
    };
    let dir = File::from(dir);
    match crate::syscall::openat_nofollow(dir.as_fd(), file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileState::Absent,
        Err(_) => FileState::Invalid,
        Ok(_) => read_private(&dir, file, uid)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .map_or(FileState::Invalid, FileState::Present),
    }
}

/// A JSON file a study recorded in its own directory, if present and private.
pub(super) fn study_file(
    studies: &File,
    name: &str,
    uid: u32,
    file: &str,
) -> Option<serde_json::Value> {
    let dir = File::from(crate::syscall::openat_dir_nofollow(studies.as_fd(), name).ok()?);
    serde_json::from_slice(&read_private(&dir, file, uid).ok()?).ok()
}

/// One child's mandatory resident observation against its completed attempts' processes.
pub(super) fn resident_status(
    studies: &File,
    name: &str,
    uid: u32,
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    match study_file_state(studies, name, uid, super::resident_observation::FILE) {
        FileState::Absent => Err("ResidentObservationNotCollected"),
        FileState::Invalid => Err("ResidentObservationInvalid"),
        FileState::Present(value) => super::resident_observation::verify(
            &serde_json::to_vec(&value).map_err(|_| "ResidentObservationInvalid")?,
            processes,
        ),
    }
}

/// G3's mandatory input over every child: the first reason any measuring child lacks it, or `Ok`. With no
/// measuring child there is nothing observed, which is not a pass.
pub(super) fn resident_summary(children: &[ChildVerification]) -> Result<(), &'static str> {
    let mut any = false;
    for child in children {
        if let Some(status) = child.resident {
            any = true;
            status?;
        }
    }
    if any {
        Ok(())
    } else {
        Err("ResidentObservationNotCollected")
    }
}

/// The telemetry summary of `ChildrenVerification::telemetry_summary`.
pub(super) fn telemetry_summary(children: &[ChildVerification]) -> Option<(u32, u32)> {
    let (mut eligible, mut collected) = (0u32, 0u32);
    for child in children {
        if child.completed_attempts.is_empty() {
            continue;
        }
        // Corrupt receipts are collected evidence that cannot qualify: every completed attempt counts against it.
        if child
            .telemetry_receipts
            .as_ref()
            .is_some_and(|value| !value.is_array())
        {
            collected = collected
                .saturating_add(u32::try_from(child.completed_attempts.len()).unwrap_or(u32::MAX));
            continue;
        }
        let receipts: Vec<&serde_json::Value> = child
            .telemetry_receipts
            .as_ref()
            .and_then(serde_json::Value::as_array)
            .map(|items| items.iter().collect())
            .unwrap_or_default();
        for index in &child.completed_attempts {
            // One-to-one: exactly one receipt names this attempt, and it is eligible and bound
            // to the game. Missing, duplicated or conflicting receipts do not count.
            let mine: Vec<_> = receipts
                .iter()
                .filter(|item| item["attempt"].as_u64() == Some(u64::from(*index)))
                .collect();
            // The receipt must be about the very process the sealed native completion names:
            // matching only the attempt number would let another process's receipt stand in.
            // An attempt with no receipt at all had no direct-frame probe (optional) and counts
            // neither way; one with a receipt must be sound, so a duplicate or bad one is
            // collected but not eligible.
            if !mine.is_empty() {
                collected = collected.saturating_add(1);
            }
            let process = child
                .completed_processes
                .iter()
                .find(|process| process.slot == *index);
            if let [only] = mine.as_slice()
                && only["frame_verdict"] == "Eligible"
                && only["source_bound_to_game"] == true
                && process.is_some_and(|process| {
                    only["pid"].as_u64() == Some(u64::from(process.pid.as_u32()))
                        && only["start_ticks"].as_u64() == Some(process.start_ticks)
                        && only["uid"].as_u64() == Some(u64::from(process.uid))
                        && frames_in_window(only, process) >= super::quality::MIN_DIRECT_FRAMES
                })
            {
                eligible = eligible.saturating_add(1);
            }
        }
    }
    (collected > 0).then_some((eligible, collected))
}

/// The frames a receipt counted inside the attempt's native benchmark window. A receipt whose frame
/// buckets were truncated, or that has none, counts none: it cannot show how many were in the window.
fn frames_in_window(receipt: &serde_json::Value, process: &AttemptProcess) -> u64 {
    if receipt["frame_seconds_truncated"] != false {
        return 0;
    }
    receipt["frame_seconds"]
        .as_array()
        .map(|buckets| {
            buckets
                .iter()
                .filter_map(|bucket| Some((bucket[0].as_u64()?, bucket[1].as_u64()?)))
                .filter(|(second, _)| {
                    (process.started_unix..=process.completed_unix).contains(second)
                })
                .fold(0u64, |total, (_, count)| total.saturating_add(count))
        })
        .unwrap_or(0)
}

/// The game process of every completed attempt, from its sealed record: a native completion's process, or a
/// long session's. A long attempt has no native result, so without this its action receipts could never be
/// attributed to an attempt.
pub(super) fn processes_of(
    evidence: &std::collections::BTreeMap<u32, super::report::AttemptEvidenceV1>,
) -> Vec<AttemptProcess> {
    evidence
        .iter()
        .filter_map(|(slot, item)| {
            if let Some(native) = item.native_completion.as_ref() {
                let process = &native.process;
                return Some(AttemptProcess {
                    slot: *slot,
                    pid: process.pid,
                    start_ticks: process.start_ticks,
                    uid: process.uid,
                    started_unix: u64::try_from(native.started_unix_nanos / 1_000_000_000)
                        .unwrap_or(u64::MAX),
                    completed_unix: u64::try_from(native.completed_unix_nanos / 1_000_000_000)
                        .unwrap_or(u64::MAX),
                });
            }
            let session = item.long_session.as_ref()?;
            Some(AttemptProcess {
                slot: *slot,
                pid: session.process.pid,
                start_ticks: session.process.start_ticks,
                uid: session.process.uid,
                started_unix: session.started_unix,
                completed_unix: session.ended_unix,
            })
        })
        .collect()
}
