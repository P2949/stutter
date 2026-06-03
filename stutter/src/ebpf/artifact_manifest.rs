use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EbpfArtifactManifest {
    pub schema_version: u32,
    pub stutter_version: String,
    pub ebpf_object_sha256: String,
    pub event_abi_version: u32,
    pub map_names: Vec<String>,
    pub program_names: Vec<String>,
}

pub(crate) const EBPF_ARTIFACT_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub(crate) const EBPF_EVENT_ABI_VERSION: u32 = 1;

pub(crate) const EBPF_MAP_NAMES: &[&str] = &[
    "BLOCK_START",
    "CPU_RUNNABLE_DEPTH",
    "DROP_COUNTERS",
    "EVENTS",
    "FENCE_SIGNAL_TIMES",
    "FENCE_WAIT_STARTS",
    "IRQ_START_TIMES",
    "KMS_FLIP_STARTS",
    "PREV_FAULTS",
    "RUNNABLE_TASK_CPU",
    "TARGET_CGROUP_IDS",
    "TARGET_IRQS",
    "TARGET_PENDING_WAKEUPS",
    "TARGET_PIDS",
    "WAKEUP_CONSUMED",
    "WAKEUP_DATA",
    "WAKEUP_SEQ",
];

pub(crate) const EBPF_PROGRAM_NAMES: &[&str] = &[
    "amdgpu_flip_done",
    "amdgpu_flip_request",
    "amdgpu_vblank_event",
    "block_rq_complete",
    "block_rq_issue",
    "cpu_frequency",
    "drm_fence_signal",
    "drm_fence_wait_done",
    "drm_fence_wait_start",
    "drm_flip_done",
    "drm_flip_request",
    "drm_vblank_event",
    "i915_flip_done",
    "i915_flip_request",
    "irq_handler_entry",
    "irq_handler_exit",
    "major_fault",
    "minor_fault",
    "sched_migrate_task",
    "sched_process_exec",
    "sched_process_exit",
    "sched_stat_wait",
    "sched_switch",
    "sched_wakeup",
    "sched_wakeup_new",
];

pub(crate) fn ebpf_artifact_manifest(
    stutter_version: impl Into<String>,
    ebpf_object_sha256: impl Into<String>,
) -> EbpfArtifactManifest {
    EbpfArtifactManifest {
        schema_version: EBPF_ARTIFACT_MANIFEST_SCHEMA_VERSION,
        stutter_version: stutter_version.into(),
        ebpf_object_sha256: ebpf_object_sha256.into(),
        event_abi_version: EBPF_EVENT_ABI_VERSION,
        map_names: EBPF_MAP_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        program_names: EBPF_PROGRAM_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ebpf_artifact_manifest_records_abi_maps_and_programs() {
        let manifest = ebpf_artifact_manifest("0.1.0", "abc123");

        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.stutter_version, "0.1.0");
        assert_eq!(manifest.ebpf_object_sha256, "abc123");
        assert_eq!(manifest.event_abi_version, EBPF_EVENT_ABI_VERSION);
        assert!(manifest.map_names.contains(&"EVENTS".to_owned()));
        assert!(manifest.map_names.contains(&"DROP_COUNTERS".to_owned()));
        assert!(manifest.program_names.contains(&"sched_switch".to_owned()));
        assert!(
            manifest
                .program_names
                .contains(&"drm_fence_signal".to_owned())
        );
    }
}
