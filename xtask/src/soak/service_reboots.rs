//! Real reboots through the typed runner. A reboot is a two-phase stage: the pre-reboot attempt
//! (`WaitingForReboot`) and the campaign state are durable before the runner is asked to reboot,
//! and the attempt after the boot proves the machine and the service came back correctly.
use std::fs;

use anyhow::{Context, ensure};
use serde_json::{Value, json};
use stutter_validation_runner::model::{BoundedServiceAction as Action, Request};

use super::{plan::Operations, service::Binding};
use crate::{
    service_campaign::{
        driver::StageFailure,
        model::{Stage, State},
        runner,
    },
    validation_campaign::{
        model::{AttemptOutcome, FailureClass},
        state::{CampaignDir, write_atomic},
    },
};

fn receipt_path(dir: &CampaignDir, kind: &str, hop: usize) -> std::path::PathBuf {
    dir.0.join("reboots").join(format!("{kind}-{hop}.json"))
}

pub(crate) fn step(
    state: &mut State,
    dir: &CampaignDir,
    _attempt: u32,
) -> anyhow::Result<AttemptOutcome> {
    fs::create_dir_all(dir.0.join("reboots"))?;
    if state.reboot_pending {
        after_boot(state, dir)?;
    }
    let completed = state.boot_ids.len() - 1;
    if completed >= state.required_reboots as usize {
        return Ok(AttemptOutcome::Passed);
    }
    let hop = completed + 1;
    let pre = super::service_recovery::applied_before_reboot(state, &dir.0.join("reboot-prep"))?;
    write_atomic(
        &receipt_path(dir, "pre", hop),
        &serde_json::to_vec_pretty(&pre)?,
    )?;
    state.reboot_pending = true;
    Ok(AttemptOutcome::WaitingForReboot)
}

/// The first attempt after a boot: prove the reboot happened and the product survived it.
fn after_boot(state: &mut State, dir: &CampaignDir) -> anyhow::Result<()> {
    let hop = state.boot_ids.len();
    let current = stutter_core::boot::current_boot_id()?;
    ensure!(
        !state.boot_ids.contains(&current),
        "a reboot was requested but the boot identity is unchanged"
    );
    let status = runner::request_campaign(state, Request::Status)?;
    ensure!(
        status["reboots_used"].as_u64() == Some(hop as u64),
        "the runner's reboot count does not match the campaign: {status}"
    );
    let pre: Value = serde_json::from_slice(
        &fs::read(receipt_path(dir, "pre", hop)).context("pre-reboot receipt missing")?,
    )?;
    ensure!(
        pre["boot_id"] == json!(state.boot_ids.last()),
        "the pre-reboot receipt belongs to another boot"
    );
    let binding = Binding::new(state, Stage::Reboots)?;
    // Volatile `/run` units and fixture configuration did not survive; the pinned install did
    // or is recreated from the repository, and the root ledger must have.
    binding.action(Action::InstallPinned)?;
    binding.prepare(super::pick_cpus()?, Operations::Both)?;
    let process = binding.start()?;
    let report = wait_settled(&binding, &pre)?;
    super::service::attest(&report, state)?;
    let post = json!({
        "hop": hop,
        "boot_id_before": pre["boot_id"],
        "boot_id_after": current,
        "outstanding_before": pre["outstanding"],
        "outstanding_after": report["worker_tokens"]["outstanding"],
        "recovery_audit_before": pre["recovery_audit_completed"],
        "recovery_audit_after": report["worker_tokens"]["recovery_audit"]["completed"],
        "controller_pid": process.pid,
        "runner": status,
    });
    write_atomic(
        &receipt_path(dir, "post", hop),
        &serde_json::to_vec_pretty(&post)?,
    )?;
    binding.action(Action::Stop)?;
    state.boot_ids.push(current);
    state.reboot_pending = false;
    // Only now does the runner stop expecting a resume.
    runner::request_campaign(state, Request::ResumeCampaign)?;
    Ok(())
}

fn wait_settled(binding: &Binding, pre: &Value) -> anyhow::Result<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    loop {
        let report = binding.action(Action::Query)?;
        let audited = report["worker_tokens"]["recovery_audit"]["completed"].as_u64()
            > pre["recovery_audit_completed"].as_u64();
        if report["worker_tokens"]["readable"] == true
            && report["worker_tokens"]["outstanding"] == 0
            && audited
        {
            return Ok(report);
        }
        if std::time::Instant::now() >= deadline {
            return Err(StageFailure::error(
                FailureClass::SoftwareDefect,
                format!("the root-issued token was not retired after the reboot: {report}"),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// The verified reboot receipts of a service campaign, for the final soak check.
pub(crate) fn verified_receipts(campaign: &std::path::Path, required: usize) -> (usize, String) {
    let mut boots = std::collections::BTreeSet::new();
    let mut good = 0;
    for hop in 1..=required {
        let read = |kind: &str| -> Option<Value> {
            serde_json::from_slice(
                &fs::read(campaign.join("reboots").join(format!("{kind}-{hop}.json"))).ok()?,
            )
            .ok()
        };
        let (Some(pre), Some(post)) = (read("pre"), read("post")) else {
            continue;
        };
        let after = post["boot_id_after"].as_str().map(str::to_owned);
        let ok = post["boot_id_before"] == pre["boot_id"]
            && after.is_some_and(|id| boots.insert(id))
            && pre["outstanding"].as_u64() >= Some(1)
            && post["outstanding_after"] == 0
            && post["recovery_audit_after"].as_u64() > pre["recovery_audit_completed"].as_u64();
        if ok {
            good += 1;
        }
    }
    (
        good,
        format!("{good} of {required} verified reboot receipts with distinct boots"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipts(root: &std::path::Path, hop: usize, before: &str, after: &str, outstanding: u64) {
        let dir = root.join("reboots");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("pre-{hop}.json")),
            json!({"boot_id": before, "outstanding": outstanding, "recovery_audit_completed": 3})
                .to_string(),
        )
        .unwrap();
        fs::write(
            dir.join(format!("post-{hop}.json")),
            json!({"boot_id_before": before, "boot_id_after": after, "outstanding_after": 0,
                   "recovery_audit_after": 4})
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn only_distinct_boots_that_retired_a_real_token_count() {
        let root =
            std::env::temp_dir().join(format!("stutter-reboot-receipts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        receipts(&root, 1, "boot-a", "boot-b", 1);
        receipts(&root, 2, "boot-b", "boot-c", 1);
        assert_eq!(verified_receipts(&root, 2).0, 2);
        // The same boot twice, or a reboot with nothing outstanding, proves nothing.
        receipts(&root, 2, "boot-b", "boot-b", 1);
        assert_eq!(verified_receipts(&root, 2).0, 1);
        receipts(&root, 2, "boot-b", "boot-c", 0);
        assert_eq!(verified_receipts(&root, 2).0, 1);
        assert_eq!(verified_receipts(&root.join("missing"), 2).0, 0);
        let _ = fs::remove_dir_all(&root);
    }
}
