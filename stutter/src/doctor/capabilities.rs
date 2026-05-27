use std::collections::BTreeMap;
use crate::daemon::capabilities::{CapabilityProbe, DaemonCapabilities};
use super::model::{DoctorCheck, DoctorStatus};
use super::utils::yes_no;

pub(crate) fn daemon_capabilities_check() -> DoctorCheck {
    let capabilities = CapabilityProbe::default().probe();
    daemon_capabilities_check_from_snapshot(capabilities)
}

pub(crate) fn daemon_capabilities_check_from_snapshot(capabilities: DaemonCapabilities) -> DoctorCheck {
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
