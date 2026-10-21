use super::{DataQualityRule, ProbeCapability, ProbeKey, ProbeSpec};
use crate::{
    artifacts::ArtifactKind,
    probe_catalog::{ProbeOverhead, ProbeStatus},
};

pub(super) const PROBE: ProbeSpec = ProbeSpec {
    key: ProbeKey::WorkloadProgress,
    catalog_key: "workload_progress",
    title: "Fixed-work completion markers",
    status: ProbeStatus::Implemented,
    answers_question: "Did the identified fixed workload complete successfully, and how long did it take?",
    cli_flags: &["--workload-progress-log"],
    artifacts: &[ArtifactKind::WorkloadProgress],
    default_enabled: false,
    overhead: ProbeOverhead::Low,
    required_capabilities: &[ProbeCapability::ExternalLog],
    ebpf_programs: &[],
    tracepoints: &[],
    perf_events: &[],
    data_quality_rules: &[DataQualityRule {
        key: "fixed_work_completion_identity",
        description: "Requires exactly one successful ordered start/completion pair with matching workload and process identity; missing markers are unavailable evidence, never zero elapsed time",
    }],
    validation_contract: "workload_progress.json is optional NDJSON from an explicitly configured cooperative producer; activity counters cannot substitute for successful fixed-work completion.",
};
