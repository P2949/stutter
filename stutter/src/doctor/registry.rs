use std::collections::BTreeMap;
use crate::probe_catalog::ProbeStatus;
use crate::probe_registry::PROBE_REGISTRY;
use super::model::{DoctorCheck, DoctorInput, DoctorStatus};

pub(crate) fn probe_registry_check(input: &DoctorInput) -> DoctorCheck {
    let mut details = BTreeMap::new();
    let implemented = PROBE_REGISTRY
        .iter()
        .filter(|spec| spec.status == ProbeStatus::Implemented)
        .map(|spec| spec.catalog_key)
        .collect::<Vec<_>>()
        .join(",");

    let requested = PROBE_REGISTRY
        .iter()
        .filter(|spec| match spec.catalog_key {
            "scheduler_runnable_latency" => true,
            "cpu_freq" => true,
            "psi_timeline" => true,
            "irq_latency" => input.irq_latency,
            "gpu_hwmon" => input.hwmon,
            "block_io" => input.block_io,
            "kms_pageflip_timing" => input.kms_timing,
            "faults" => input.faults,
            "cpu_perf" => input.cpu_perf,
            "frame_log" => input.mangohud_log.is_some(),
            _ => false,
        })
        .map(|spec| spec.catalog_key)
        .collect::<Vec<_>>()
        .join(",");

    details.insert("implemented_registry_probes".to_owned(), implemented);
    details.insert("requested_registry_probes".to_owned(), requested);

    DoctorCheck {
        name: "probe_registry".to_owned(),
        status: DoctorStatus::Pass,
        message: "probe metadata is loaded from PROBE_REGISTRY".to_owned(),
        details,
    }
}
