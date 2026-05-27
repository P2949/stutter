use super::models::{UCLAMP_MAX_VALUE, UCLAMP_MIN_VALUE, UclampPolicy, UclampValues};

pub(crate) fn validate_policy_and_request(
    policy: &UclampPolicy,
    values: UclampValues,
) -> anyhow::Result<()> {
    if !policy.allow_uclamp_changes {
        anyhow::bail!("policy does not allow uclamp changes");
    }

    if !policy.allow_per_task {
        anyhow::bail!("policy does not allow per-task uclamp changes");
    }

    if values.is_empty() {
        anyhow::bail!("uclamp action requires sched_util_min, sched_util_max, or both");
    }

    if policy.min_allowed_util_min > policy.max_allowed_util_min {
        anyhow::bail!(
            "invalid uclamp policy min range {}..={}: min is greater than max",
            policy.min_allowed_util_min,
            policy.max_allowed_util_min
        );
    }

    if policy.min_allowed_util_max > policy.max_allowed_util_max {
        anyhow::bail!(
            "invalid uclamp policy max range {}..={}: min is greater than max",
            policy.min_allowed_util_max,
            policy.max_allowed_util_max
        );
    }

    if policy.max_allowed_util_min > UCLAMP_MAX_VALUE
        || policy.max_allowed_util_max > UCLAMP_MAX_VALUE
    {
        anyhow::bail!(
            "invalid uclamp policy range; uclamp values must be within {}..={}",
            UCLAMP_MIN_VALUE,
            UCLAMP_MAX_VALUE
        );
    }

    if let Some(util_min) = values.sched_util_min {
        validate_uclamp_value("sched_util_min", util_min)?;
        if !(policy.min_allowed_util_min..=policy.max_allowed_util_min).contains(&util_min) {
            anyhow::bail!(
                "requested sched_util_min {} is outside policy range {}..={}",
                util_min,
                policy.min_allowed_util_min,
                policy.max_allowed_util_min
            );
        }
    }

    if let Some(util_max) = values.sched_util_max {
        validate_uclamp_value("sched_util_max", util_max)?;
        if !(policy.min_allowed_util_max..=policy.max_allowed_util_max).contains(&util_max) {
            anyhow::bail!(
                "requested sched_util_max {} is outside policy range {}..={}",
                util_max,
                policy.min_allowed_util_max,
                policy.max_allowed_util_max
            );
        }
    }

    if let (Some(util_min), Some(util_max)) = (values.sched_util_min, values.sched_util_max)
        && util_min > util_max
    {
        anyhow::bail!(
            "requested sched_util_min {} is greater than sched_util_max {}",
            util_min,
            util_max
        );
    }

    Ok(())
}

pub(crate) fn validate_uclamp_value(name: &str, value: u32) -> anyhow::Result<()> {
    if !(UCLAMP_MIN_VALUE..=UCLAMP_MAX_VALUE).contains(&value) {
        anyhow::bail!(
            "requested {name} {} is outside uclamp range {}..={}",
            value,
            UCLAMP_MIN_VALUE,
            UCLAMP_MAX_VALUE
        );
    }

    Ok(())
}
