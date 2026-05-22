use super::*;

pub(super) fn validate_action_request(
    action: &CgroupPlacementAction,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_cgroup_moves {
        anyhow::bail!("policy does not allow cgroup moves");
    }

    if action.targets.is_empty() {
        anyhow::bail!("cgroup placement requires at least one explicit target task");
    }

    let target_rel = normalize_cgroup_path(&action.target_cgroup)?;
    if !policy.allow_nested_cgroups && target_rel.components().count() > 2 {
        anyhow::bail!(
            "policy does not allow nested cgroups: {}",
            target_rel.display()
        );
    }

    if (action.cpuset_cpus.is_some() || action.cpuset_mems.is_some())
        && !policy.allow_cpuset_changes
    {
        anyhow::bail!("policy does not allow cpuset changes");
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
        anyhow::bail!("refusing to move system/critical task class {class}");
    }

    Ok(())
}

pub(super) fn preflight_cgroup_files(
    action: &CgroupPlacementAction,
    policy: &CgroupPlacementPolicy,
) -> anyhow::Result<()> {
    let target_abs = action.target_cgroup_abs()?;

    if !target_abs.is_dir() {
        anyhow::bail!("target cgroup does not exist: {}", target_abs.display());
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
        anyhow::bail!("policy does not allow cpuset changes");
    }

    ensure_writable_file(&target_abs.join(file_name))
}

pub(super) fn validate_cpuset_value(name: &str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{name} must not be empty");
    }

    if value.trim() != value {
        anyhow::bail!("{name} must not contain leading or trailing whitespace");
    }

    for ch in value.chars() {
        if !(ch.is_ascii_digit() || ch == ',' || ch == '-') {
            anyhow::bail!("{name} contains invalid character {ch:?}");
        }
    }

    Ok(())
}
