use anyhow::Context;

use super::{ioprio::profile_ioprio_policy, plan::ProfileApplyPlan};
use crate::{
    actions::{
        TuningAction,
        ioprio::IoPrioAction,
        nice::{NiceAction, NicePolicy},
    },
    affinity,
};

pub(super) fn preflight_profile_plan(plan: &ProfileApplyPlan) -> anyhow::Result<()> {
    for (nice, targets) in &plan.nice_groups {
        NiceAction {
            targets: targets.clone(),
            nice: *nice,
            policy: NicePolicy::default(),
        }
        .preflight()
        .with_context(|| format!("nice profile action preflight failed for nice={nice}"))?;
    }

    for (ioprio, targets) in &plan.ionice_groups {
        IoPrioAction {
            targets: targets.clone(),
            ioprio: *ioprio,
            policy: profile_ioprio_policy(),
        }
        .preflight()
        .with_context(|| {
            format!(
                "I/O priority profile action preflight failed for ionice={}",
                ioprio.label()
            )
        })?;
    }

    Ok(())
}

pub(super) fn verify_affinity_plan(plan: &ProfileApplyPlan) -> anyhow::Result<()> {
    for planned in &plan.affinity_changes {
        match affinity::read_allowed_mask(planned.record.tid) {
            Ok(mask) if mask == planned.record.applied_mask => {}
            Ok(mask) => {
                anyhow::bail!(
                    "affinity verify failed for TID {}: requested={} actual={}",
                    planned.record.tid,
                    planned.record.applied_mask.to_range_string(),
                    mask.to_range_string()
                );
            }
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {}
            Err(err) => {
                anyhow::bail!(
                    "failed to verify affinity for TID {}: {err}",
                    planned.record.tid
                );
            }
        }
    }

    Ok(())
}
