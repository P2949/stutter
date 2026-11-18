use std::{fs::File, os::fd::AsFd};

use super::{ResearchIntentV1, report_store::read_private};

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
    let runs: Vec<_> = super::report_store::read_evidence(&evidence, uid)?
        .values()
        .filter_map(|item| item.native_completion.as_deref())
        .map(|native| (native.proton_source, native.metrics.average_fps))
        .collect();
    anyhow::ensure!(
        !runs.is_empty(),
        "the pilot's runs do not support a rule: PilotTooThin"
    );
    let child: ResearchIntentV1 = serde_json::from_slice(&read_private(&dir, "intent.json", uid)?)?;
    let sources: Vec<_> = child.tools.iter().map(|tool| tool.source).collect();
    let recomputed = super::pilot_rule::rule_from_all_pilots(&sources, &runs)
        .map_err(|code| anyhow::anyhow!("the pilot's runs do not support a rule: {code}"))?;
    anyhow::ensure!(
        recomputed == *recorded,
        "the recorded confirmation rule is not what the pilot's sealed runs freeze"
    );
    Ok(())
}
