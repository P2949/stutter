use std::collections::BTreeMap;

use super::{
    model::{DoctorCheck, DoctorStatus},
    utils::yes_no,
};
use crate::daemon::capabilities::{CapabilityProbe, DaemonCapabilities};

pub(crate) fn daemon_capabilities_check() -> DoctorCheck {
    let capabilities = CapabilityProbe::default().probe();
    daemon_capabilities_check_from_snapshot(capabilities)
}

pub(crate) fn daemon_capabilities_check_from_snapshot(
    capabilities: DaemonCapabilities,
) -> DoctorCheck {
    let unavailable = capabilities.unavailable_features();
    let mut details = BTreeMap::new();

    details.insert(
        "kernel_release".to_owned(),
        capabilities
            .kernel_release
            .as_deref()
            .unwrap_or("unknown")
            .to_owned(),
    );
    details.insert(
        "btf_available".to_owned(),
        yes_no(capabilities.btf_available),
    );
    details.insert(
        "sched_tracepoints_available".to_owned(),
        yes_no(capabilities.sched_tracepoints_available),
    );
    details.insert(
        "perf_permissions_likely".to_owned(),
        yes_no(capabilities.perf_permissions_likely),
    );
    details.insert(
        "perf_event_paranoid".to_owned(),
        capabilities
            .perf_event_paranoid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned()),
    );
    details.insert(
        "cgroup_v2_available".to_owned(),
        yes_no(capabilities.cgroup_v2_available),
    );
    details.insert(
        "sched_ext_available".to_owned(),
        yes_no(capabilities.sched_ext_available),
    );
    details.insert(
        "uclamp_available".to_owned(),
        yes_no(capabilities.uclamp_available),
    );
    details.insert(
        "ionice_available".to_owned(),
        yes_no(capabilities.ionice_available),
    );
    details.insert(
        "irq_affinity_available".to_owned(),
        yes_no(capabilities.irq_affinity_available),
    );
    details.insert(
        "gpu_sysfs_available".to_owned(),
        yes_no(capabilities.gpu_sysfs_available),
    );

    let required_missing = !capabilities.btf_available || !capabilities.sched_tracepoints_available;
    let status = if required_missing {
        DoctorStatus::Warn
    } else {
        DoctorStatus::Pass
    };
    let message = if unavailable.is_empty() {
        "daemon capability probe found all known optional features".to_owned()
    } else {
        format!(
            "daemon capability probe missing or cannot confirm: {}",
            unavailable.join(", ")
        )
    };

    DoctorCheck {
        name: "daemon_capabilities".to_owned(),
        status,
        message,
        details,
    }
}

/// Who else holds tuning resources on this host: read-only, from kernel state and process
/// identity. An owner (or an unreadable state) is a warning, never a failure: the autotuner
/// simply denies the matching domain.
pub(crate) fn external_owners_check() -> DoctorCheck {
    external_owners_check_at(std::path::Path::new("/proc"), std::path::Path::new("/sys"))
}

pub(crate) fn external_owners_check_at(
    proc_root: &std::path::Path,
    sys_root: &std::path::Path,
) -> DoctorCheck {
    use crate::integrations::{ResourceDomain, probe};
    let report = probe(proc_root, sys_root);
    let mut details = BTreeMap::new();
    details.insert("sched_ext".to_owned(), format!("{:?}", report.sched_ext));
    details.insert(
        "scx_manager".to_owned(),
        format!("{:?}", report.scx_manager),
    );
    details.insert("gamemode".to_owned(), format!("{:?}", report.gamemode));
    details.insert("lact".to_owned(), format!("{:?}", report.lact));
    let owned: Vec<String> = [
        ResourceDomain::Scheduler,
        ResourceDomain::GpuPower,
        ResourceDomain::TaskPriorityAndPower,
    ]
    .into_iter()
    .filter_map(|domain| {
        report
            .external_owner(domain)
            .map(|reason| format!("{domain:?}: {reason}"))
    })
    .collect();
    let (status, message) = if owned.is_empty() {
        (
            DoctorStatus::Pass,
            "no other tuning owner detected for the scheduler, GPU power or task priority"
                .to_owned(),
        )
    } else {
        (
            DoctorStatus::Warn,
            format!(
                "externally owned, stutter will deny these: {}",
                owned.join("; ")
            ),
        )
    };
    DoctorCheck {
        name: "external_owners".to_owned(),
        status,
        message,
        details,
    }
}
