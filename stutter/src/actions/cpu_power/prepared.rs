//! The prepared-before-mutation transaction of CPU power and the identity of the cpufreq resource a
//! restore record refers to.
//!
//! A sysfs path alone says nothing about *which* cpufreq policy it was when the original value was
//! read: after a driver reload, a CPU hotplug or a reboot the same path can belong to a different
//! policy, with different related CPUs, a different driver or different legal values. A record
//! therefore carries the policy, its related CPU set, the driver, the exact controlled attribute,
//! the legal values and the boot, and a restore rediscovers all of it first.
use std::path::Path;

use anyhow::Context;

use super::{
    CpuPowerAction, CpuPowerPolicy, CpuPowerTargetFile, read_trimmed, rollback_cpu_power_records,
    write_trimmed,
};
use crate::actions::{
    ActionBoundaryError, CpuPowerResourceIdentity, CpuPowerRestoreRecord, RollbackToken,
};

fn read_optional(path: &Path) -> Option<String> {
    read_trimmed(path).ok().filter(|value| !value.is_empty())
}

pub(crate) fn boot_id() -> Option<String> {
    read_optional(Path::new("/proc/sys/kernel/random/boot_id"))
}

/// The identity of the cpufreq resource a controlled attribute file belongs to.
pub(super) fn resource_identity(
    cpu: u32,
    attribute_path: &Path,
) -> Option<CpuPowerResourceIdentity> {
    let cpufreq = attribute_path.parent()?;
    let attribute = attribute_path.file_name()?.to_str()?.to_owned();
    let available = match attribute.as_str() {
        "energy_performance_preference" => "energy_performance_available_preferences",
        "scaling_governor" => "scaling_available_governors",
        _ => return None,
    };
    // `cpuN/cpufreq` is a link to the policy directory; its real name is the policy identity.
    let policy = std::fs::canonicalize(cpufreq)
        .ok()?
        .file_name()?
        .to_str()?
        .to_owned();
    let mut related: Vec<u32> = read_optional(&cpufreq.join("related_cpus"))
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|cpu| cpu.parse().ok())
        .collect();
    related.sort_unstable();
    Some(CpuPowerResourceIdentity {
        cpu,
        policy,
        related_cpus: related,
        attribute,
        driver: read_optional(&cpufreq.join("scaling_driver")),
        available_values: read_optional(&cpufreq.join(available)),
        boot_id: boot_id(),
    })
}

/// Rediscovers the resource a record names and compares it with what was recorded. `Err` carries
/// the reason the record must not be replayed.
pub(crate) fn verify_resource(record: &CpuPowerRestoreRecord) -> Result<(), String> {
    let Some(recorded) = &record.identity else {
        return Err("the record has no cpufreq resource identity".to_owned());
    };
    let current = resource_identity(recorded.cpu, &record.path)
        .ok_or_else(|| "the cpufreq resource can no longer be rediscovered".to_owned())?;
    if current.boot_id != recorded.boot_id {
        return Err("the system booted since the original value was read".to_owned());
    }
    for (name, same) in [
        ("policy", current.policy == recorded.policy),
        (
            "related CPUs",
            current.related_cpus == recorded.related_cpus,
        ),
        ("attribute", current.attribute == recorded.attribute),
        ("driver", current.driver == recorded.driver),
    ] {
        if !same {
            return Err(format!(
                "the cpufreq {name} is not the one that was recorded"
            ));
        }
    }
    // The original value must still be a legal value of this attribute.
    if let Some(legal) = &current.available_values
        && !legal
            .split_whitespace()
            .any(|value| value == record.original_value)
    {
        return Err(format!(
            "{:?} is no longer a legal value of {}",
            record.original_value, current.attribute
        ));
    }
    Ok(())
}

impl CpuPowerAction {
    fn plan_records(
        &self,
        policy: &CpuPowerPolicy,
    ) -> anyhow::Result<Vec<(CpuPowerTargetFile, CpuPowerRestoreRecord)>> {
        let files = self.collect_target_files(policy)?;
        files
            .into_iter()
            .map(|target| {
                let identity = resource_identity(target.cpu, &target.path).with_context(|| {
                    format!(
                        "cannot establish the cpufreq identity of {}",
                        target.path.display()
                    )
                })?;
                let record = CpuPowerRestoreRecord {
                    path: target.path.clone(),
                    original_value: target.original_value.clone(),
                    identity: Some(identity),
                };
                Ok((target, record))
            })
            .collect()
    }

    pub(crate) fn apply_prepared_with_policy(
        &self,
        policy: &CpuPowerPolicy,
        before_mutation: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> crate::actions::ApplyResult {
        let plan = self.plan_records(policy)?;
        let records: Vec<_> = plan.iter().map(|(_, record)| record.clone()).collect();
        let token = RollbackToken::CpuPowerRestore {
            records: records.clone(),
        };
        // The identity-complete original values are durable before the first write.
        before_mutation(&token)?;
        // Anything that moved between preparation and commit means the snapshot no longer
        // describes the host; abort with zero writes.
        let current: Vec<_> = self
            .plan_records(policy)?
            .into_iter()
            .map(|(_, record)| record)
            .collect();
        if current != records {
            return Err(anyhow::anyhow!(
                "CPU power state drifted between preparation and commit; nothing was written"
            )
            .into());
        }
        let failed = |source: anyhow::Error| crate::actions::PartialApplyError {
            source,
            // Restoring what was not yet changed writes the value it already has.
            rollback: Some(token.clone()),
        };
        for (target, _) in &plan {
            if target.original_value == target.requested_value {
                continue;
            }
            write_trimmed(&target.path, &target.requested_value)
                .with_context(|| format!("failed to write {}", target.path.display()))
                .map_err(failed)?;
            let actual = read_trimmed(&target.path)
                .with_context(|| format!("failed to verify write to {}", target.path.display()))
                .map_err(failed)?;
            if actual != target.requested_value {
                return Err(failed(
                    ActionBoundaryError::restore_failed(
                        "cpu_power",
                        format!(
                            "CPU power write verification failed for {}: requested={:?} actual={:?}",
                            target.path.display(),
                            target.requested_value,
                            actual
                        ),
                    )
                    .into(),
                ));
            }
        }
        Ok(token)
    }
}

/// Restores records after rediscovering the resource each names. A record that fails
/// verification is skipped and reported; with `require_identity` a record without any identity is
/// skipped too (a legacy record is never replayed automatically).
pub(crate) fn rollback_verified(
    records: &[CpuPowerRestoreRecord],
    require_identity: bool,
) -> anyhow::Result<CpuPowerRestoreOutcome> {
    let mut outcome = CpuPowerRestoreOutcome::default();
    let mut replayable = Vec::new();
    for record in records {
        let verdict = match (&record.identity, require_identity) {
            (None, false) => Ok(()),
            _ => verify_resource(record),
        };
        match verdict {
            Ok(()) => replayable.push(record.clone()),
            Err(reason) => {
                outcome.skipped_identity_mismatch += 1;
                outcome.messages.push(format!(
                    "cpu power restore skipped {}: {reason}",
                    record.path.display()
                ));
            }
        }
    }
    rollback_cpu_power_records(&replayable)?;
    outcome.restored = replayable.len();
    Ok(outcome)
}

#[derive(Debug, Default)]
pub(crate) struct CpuPowerRestoreOutcome {
    pub(crate) restored: usize,
    pub(crate) skipped_identity_mismatch: usize,
    pub(crate) messages: Vec<String>,
}
