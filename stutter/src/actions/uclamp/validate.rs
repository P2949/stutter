use super::models::{UCLAMP_MAX_VALUE, UCLAMP_MIN_VALUE, UclampPolicy, UclampValues};
use crate::actions::ActionBoundaryError;

pub(crate) fn validate_policy_and_request(
    policy: &UclampPolicy,
    values: UclampValues,
) -> anyhow::Result<()> {
    if !policy.allow_uclamp_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "uclamp",
            requirement: "allow_uclamp_changes",
        }
        .into());
    }

    if !policy.allow_per_task {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "uclamp",
            requirement: "allow_per_task",
        }
        .into());
    }

    if values.is_empty() {
        return Err(ActionBoundaryError::InvalidRequest {
            action_kind: "uclamp",
            reason: "requires sched_util_min, sched_util_max, or both".to_owned(),
        }
        .into());
    }

    if policy.min_allowed_util_min > policy.max_allowed_util_min {
        return Err(ActionBoundaryError::InvalidPolicy {
            action_kind: "uclamp",
            reason: format!(
                "invalid uclamp policy min range {}..={}: min is greater than max",
                policy.min_allowed_util_min, policy.max_allowed_util_min
            ),
        }
        .into());
    }

    if policy.min_allowed_util_max > policy.max_allowed_util_max {
        return Err(ActionBoundaryError::InvalidPolicy {
            action_kind: "uclamp",
            reason: format!(
                "invalid uclamp policy max range {}..={}: min is greater than max",
                policy.min_allowed_util_max, policy.max_allowed_util_max
            ),
        }
        .into());
    }

    if policy.max_allowed_util_min > UCLAMP_MAX_VALUE
        || policy.max_allowed_util_max > UCLAMP_MAX_VALUE
    {
        return Err(ActionBoundaryError::InvalidPolicy {
            action_kind: "uclamp",
            reason: format!(
                "invalid uclamp policy range; uclamp values must be within {}..={}",
                UCLAMP_MIN_VALUE, UCLAMP_MAX_VALUE
            ),
        }
        .into());
    }

    if let Some(util_min) = values.sched_util_min {
        validate_uclamp_value("sched_util_min", util_min)?;
        if !(policy.min_allowed_util_min..=policy.max_allowed_util_min).contains(&util_min) {
            return Err(ActionBoundaryError::InvalidValue {
                action_kind: "uclamp",
                field: "sched_util_min".to_owned(),
                reason: format!(
                    "requested sched_util_min {} is outside policy range {}..={}",
                    util_min, policy.min_allowed_util_min, policy.max_allowed_util_min
                ),
            }
            .into());
        }
    }

    if let Some(util_max) = values.sched_util_max {
        validate_uclamp_value("sched_util_max", util_max)?;
        if !(policy.min_allowed_util_max..=policy.max_allowed_util_max).contains(&util_max) {
            return Err(ActionBoundaryError::InvalidValue {
                action_kind: "uclamp",
                field: "sched_util_max".to_owned(),
                reason: format!(
                    "requested sched_util_max {} is outside policy range {}..={}",
                    util_max, policy.min_allowed_util_max, policy.max_allowed_util_max
                ),
            }
            .into());
        }
    }

    if let (Some(util_min), Some(util_max)) = (values.sched_util_min, values.sched_util_max)
        && util_min > util_max
    {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "uclamp",
            field: "sched_util_min".to_owned(),
            reason: format!(
                "requested sched_util_min {} is greater than sched_util_max {}",
                util_min, util_max
            ),
        }
        .into());
    }

    Ok(())
}

pub(crate) fn validate_uclamp_value(name: &str, value: u32) -> anyhow::Result<()> {
    if !(UCLAMP_MIN_VALUE..=UCLAMP_MAX_VALUE).contains(&value) {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "uclamp",
            field: name.to_owned(),
            reason: format!(
                "requested {name} {} is outside uclamp range {}..={}",
                value, UCLAMP_MIN_VALUE, UCLAMP_MAX_VALUE
            ),
        }
        .into());
    }

    Ok(())
}
