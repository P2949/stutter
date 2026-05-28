use std::path::Path;

use super::{
    fs_io::{ensure_path_under_root, ensure_writable_file, normalize_cgroup_path},
    model::{CgroupPlacementAction, CgroupPlacementPolicy},
};
use crate::{actions::ActionBoundaryError, process_tree::TaskClass};

pub(super) fn validate_action_request(
    action: &CgroupPlacementAction,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_cgroup_moves {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "cgroup",
            requirement: "allow_cgroup_moves",
        }
        .into());
    }

    if action.targets.is_empty() {
        return Err(ActionBoundaryError::MissingExplicitTargets {
            action_kind: "cgroup",
        }
        .into());
    }

    let target_rel = normalize_cgroup_path(&action.target_cgroup)?;
    if !policy.allow_nested_cgroups && target_rel.components().count() > 2 {
        return Err(ActionBoundaryError::InvalidPolicy {
            action_kind: "cgroup",
            reason: format!(
                "policy does not allow nested cgroups: {}",
                target_rel.display()
            ),
        }
        .into());
    }

    if (action.cpuset_cpus.is_some() || action.cpuset_mems.is_some())
        && !policy.allow_cpuset_changes
    {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "cgroup",
            requirement: "allow_cpuset_changes",
        }
        .into());
    }

    if let Some(cpuset_cpus) = &action.cpuset_cpus {
        validate_cpuset_value("cpuset.cpus", cpuset_cpus)?;
    }

    if let Some(cpuset_mems) = &action.cpuset_mems {
        validate_cpuset_value("cpuset.mems", cpuset_mems)?;
    }

    Ok(())
}

pub(super) fn validate_target_class(class: TaskClass) -> anyhow::Result<()> {
    if matches!(
        class,
        TaskClass::AudioRealtime
            | TaskClass::Input
            | TaskClass::KernelThread
            | TaskClass::IrqThread
            | TaskClass::Service
            | TaskClass::NetworkDaemon
            | TaskClass::StorageDaemon
            | TaskClass::Unknown
    ) {
        return Err(ActionBoundaryError::InvalidRequest {
            action_kind: "cgroup",
            reason: format!("refusing to move system/critical task class {class}"),
        }
        .into());
    }

    Ok(())
}

pub(super) fn preflight_cgroup_files(
    action: &CgroupPlacementAction,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    let target_abs = action.target_cgroup_abs()?;

    if !target_abs.is_dir() {
        return Err(ActionBoundaryError::MissingPath {
            action_kind: "cgroup",
            path: target_abs,
        }
        .into());
    }

    ensure_path_under_root(&action.cgroup_root, &target_abs)?;
    ensure_writable_file(&target_abs.join("cgroup.procs"))?;

    if action.cpuset_cpus.is_some() {
        ensure_cpuset_available(&target_abs, "cpuset.cpus", policy)?;
    }

    if action.cpuset_mems.is_some() {
        ensure_cpuset_available(&target_abs, "cpuset.mems", policy)?;
    }

    Ok(())
}

pub(super) fn ensure_cpuset_available(
    target_abs: &Path,
    file_name: &str,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_cpuset_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "cgroup",
            requirement: "allow_cpuset_changes",
        }
        .into());
    }

    ensure_writable_file(&target_abs.join(file_name))
}

pub(super) fn validate_cpuset_value(name: &str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "cgroup",
            field: name.to_owned(),
            reason: format!("{name} must not be empty"),
        }
        .into());
    }

    if value.trim() != value {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "cgroup",
            field: name.to_owned(),
            reason: format!("{name} must not contain leading or trailing whitespace"),
        }
        .into());
    }

    for ch in value.chars() {
        if !(ch.is_ascii_digit() || ch == ',' || ch == '-') {
            return Err(ActionBoundaryError::InvalidValue {
                action_kind: "cgroup",
                field: name.to_owned(),
                reason: format!("{name} contains invalid character {ch:?}"),
            }
            .into());
        }
    }

    Ok(())
}
