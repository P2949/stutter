//! Action-family matching and daemon capability checks used by policy evaluation.

use crate::daemon::capabilities::DaemonCapabilities;

pub(in crate::daemon::policy) fn action_kind_matches_family(
    action_kind: &str,
    family: &str,
) -> bool {
    action_kind == family
        || action_kind.strip_prefix(family).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}

pub(in crate::daemon::policy) fn unavailable_capability_for_action(
    action_kind: &str,
    capabilities: &DaemonCapabilities,
) -> Option<&'static str> {
    if action_kind_matches_family(action_kind, "ionice") && !capabilities.ionice_available {
        Some("ionice")
    } else if action_kind_matches_family(action_kind, "uclamp") && !capabilities.uclamp_available {
        Some("uclamp")
    } else if (action_kind_matches_family(action_kind, "irq")
        || action_kind_matches_family(action_kind, "irq_affinity"))
        && !capabilities.irq_affinity_available
    {
        Some("irq_affinity")
    } else if (action_kind_matches_family(action_kind, "cgroup")
        || action_kind_matches_family(action_kind, "cpuset"))
        && !capabilities.cgroup_v2_available
    {
        Some("cgroup_v2")
    } else if (action_kind_matches_family(action_kind, "gpu")
        || action_kind_matches_family(action_kind, "gpu_power"))
        && !capabilities.gpu_sysfs_available
    {
        Some("gpu_sysfs")
    } else if action_kind_matches_family(action_kind, "scx") && !capabilities.sched_ext_available {
        Some("sched_ext")
    } else if (action_kind_matches_family(action_kind, "cpu_perf")
        || action_kind_matches_family(action_kind, "perf"))
        && !capabilities.perf_permissions_likely
    {
        Some("perf_permissions")
    } else {
        None
    }
}
