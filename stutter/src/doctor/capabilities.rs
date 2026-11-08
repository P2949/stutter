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
    use crate::integrations::probe;
    let report = probe(proc_root, sys_root);
    let mut details = BTreeMap::new();
    details.insert("sched_ext".to_owned(), format!("{:?}", report.sched_ext));
    for adapter in crate::integrations::adapters::adapter_reports(&report) {
        details.insert(
            format!("{:?}", adapter.owner).to_lowercase(),
            format!(
                "{:?} via {:?}; delegation {}",
                adapter.status, adapter.interface, adapter.delegation
            ),
        );
    }
    let owned: Vec<String> = crate::integrations::ALL_DOMAINS
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
            "no other tuning owner detected for any resource domain".to_owned(),
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

/// The cache, die, capacity and locality facts the planner may use for placement hypotheses,
/// read natively from sysfs. Informational: an absent fact is reported as absent, never guessed.
pub(crate) fn topology_check() -> DoctorCheck {
    topology_check_at(
        std::path::Path::new("/sys/devices/system"),
        std::path::Path::new("/sys/class"),
    )
}

pub(crate) fn topology_check_at(
    system_root: &std::path::Path,
    class_root: &std::path::Path,
) -> DoctorCheck {
    use crate::{topology::TopologyModel, topology_v2::TopologyV2};
    let mut details = BTreeMap::new();
    let base = match TopologyModel::read_at(system_root) {
        Ok(base) => base,
        Err(error) => {
            return DoctorCheck {
                name: "topology".to_owned(),
                status: DoctorStatus::Warn,
                message: format!("the base CPU topology could not be read: {error:#}"),
                details,
            };
        }
    };
    let effective = base.online_cpu_ids();
    let topo = TopologyV2::read_at(system_root, class_root, &base, effective);
    details.insert(
        "online_cpus".to_owned(),
        base.online_cpu_ids().len().to_string(),
    );
    details.insert("llc_domains".to_owned(), topo.llc_domains.len().to_string());
    details.insert(
        "capacity_classes".to_owned(),
        if topo.capacity_classes.is_empty() {
            "none reported (homogeneous or not exposed)".to_owned()
        } else {
            topo.capacity_classes.len().to_string()
        },
    );
    details.insert(
        "larger_cache_cpus".to_owned(),
        if topo.larger_cache_cpus.is_empty() {
            "none verified".to_owned()
        } else {
            crate::topology::cpus_to_range_string(&topo.larger_cache_cpus)
        },
    );
    details.insert(
        "frequency_domains".to_owned(),
        topo.frequency_domains.len().to_string(),
    );
    details.insert(
        "numa_nodes".to_owned(),
        topo.numa_distance.len().to_string(),
    );
    details.insert(
        "gpu_devices".to_owned(),
        topo.gpu_locality.len().to_string(),
    );
    details.insert("fingerprint".to_owned(), topo.fingerprint.clone());
    DoctorCheck {
        name: "topology".to_owned(),
        status: DoctorStatus::Pass,
        message: format!(
            "{} LLC domain(s), {} capacity class(es), fingerprint {}",
            topo.llc_domains.len(),
            topo.capacity_classes.len(),
            topo.fingerprint
        ),
        details,
    }
}

/// The boot environment's identity: informational, and the basis for not carrying a result across
/// a different kernel, isolation setup or generation. The command line itself is not shown.
pub(crate) fn boot_environment_check() -> DoctorCheck {
    use crate::boot_environment::BootEnvironment;
    let environment = BootEnvironment::read_at(
        std::path::Path::new("/proc"),
        std::path::Path::new("/sys"),
        std::path::Path::new("/run"),
    );
    let mut details = BTreeMap::new();
    details.insert(
        "kernel_release".to_owned(),
        environment.kernel_release.clone(),
    );
    details.insert("preempt".to_owned(), format!("{:?}", environment.preempt));
    details.insert(
        "nohz_full".to_owned(),
        format!("{:?}", environment.isolation.nohz_full),
    );
    details.insert(
        "isolcpus".to_owned(),
        format!("{:?}", environment.isolation.isolcpus),
    );
    details.insert(
        "rcu_nocbs".to_owned(),
        format!("{:?}", environment.isolation.rcu_nocbs),
    );
    details.insert(
        "microcode".to_owned(),
        environment
            .microcode
            .clone()
            .unwrap_or_else(|| "unknown".to_owned()),
    );
    details.insert(
        "generation".to_owned(),
        environment
            .generation
            .clone()
            .unwrap_or_else(|| "unknown".to_owned()),
    );
    details.insert(
        "environment_hash".to_owned(),
        environment.environment_hash.clone(),
    );
    DoctorCheck {
        name: "boot_environment".to_owned(),
        status: DoctorStatus::Pass,
        message: format!(
            "kernel {} ({:?}), environment {}",
            environment.kernel_release, environment.preempt, environment.environment_hash
        ),
        details,
    }
}

/// What GameMode, `scx_loader` and power-profiles-daemon say about themselves over D-Bus.
pub(crate) fn exact_owner_states_check() -> DoctorCheck {
    use crate::integrations::exact::{Exact, ExactOwnerStates};
    fn show<T: std::fmt::Debug>(state: &Exact<T>) -> String {
        match state {
            Exact::Absent => "absent (the bus name has no owner)".to_owned(),
            Exact::Present(value) => format!("{value:?}"),
            Exact::Unreachable(reason) => format!("unreachable: {reason}"),
        }
    }
    let states = ExactOwnerStates::read();
    let mut details = BTreeMap::new();
    details.insert(
        "gamemode_clients".to_owned(),
        show(&states.gamemode_clients),
    );
    details.insert(
        "scx_loader_scheduler".to_owned(),
        show(&states.scx_loader_scheduler),
    );
    details.insert("power_profile".to_owned(), show(&states.power_profile));
    DoctorCheck {
        name: "exact_owner_states".to_owned(),
        status: DoctorStatus::Pass,
        message: "owner state through each tool's own D-Bus interface".to_owned(),
        details,
    }
}
