pub(crate) fn force_for_watch_apply(initial: bool, user_force: bool) -> bool {
    initial && user_force
}

pub fn profile_apply_policy(
    dry_run: bool,
    allow_medium_risk: bool,
    allow_persistent_effects: bool,
    source: crate::daemon_policy::ActionSource,
) -> crate::daemon_policy::DaemonPolicy {
    let mut policy = if dry_run {
        crate::daemon_policy::DaemonPolicy::observe(source)
    } else if allow_medium_risk {
        crate::daemon_policy::DaemonPolicy::apply_medium_risk(source)
    } else {
        crate::daemon_policy::DaemonPolicy::apply_low_risk(source)
    };
    policy.allow_persistent_effects = allow_persistent_effects;
    policy
}

pub(crate) fn validate_apply_profile_policy(
    profile: &crate::profiles::Profile,
    tree_pid: u32,
    force: bool,
    dry_run: bool,
    allow_medium_risk: bool,
    persistent_effect: bool,
    source: crate::daemon_policy::ActionSource,
) -> anyhow::Result<crate::daemon_policy::DaemonPolicy> {
    let policy = profile_apply_policy(dry_run, allow_medium_risk, persistent_effect, source);
    let action = crate::actions::cpu_affinity::CpuAffinityProfileAction {
        tree_pid,
        profile: profile.clone(),
        force_restore_overwrite: force,
    };
    let descriptor = action.descriptor_with_persistent_effect(persistent_effect);
    let intent = if dry_run {
        crate::daemon_policy::PolicyIntent::DryRun
    } else {
        crate::daemon_policy::PolicyIntent::Apply
    };

    policy.check_action(intent, &descriptor).map_err(|err| {
        anyhow::anyhow!(
            "profile '{}' rejected by daemon policy: {err}",
            profile.name
        )
    })?;

    Ok(policy)
}

pub(crate) fn validate_apply_profile_mode(
    dry_run: bool,
    watch: bool,
    explain: bool,
) -> anyhow::Result<()> {
    if dry_run && watch {
        anyhow::bail!(
            "apply-profile --dry-run cannot be combined with --watch; run a one-shot dry-run without --watch"
        );
    }

    if explain && !dry_run {
        anyhow::bail!(
            "apply-profile --explain requires --dry-run; use --dry-run --explain to preview profile matches before applying"
        );
    }

    Ok(())
}
