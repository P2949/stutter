use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    Implemented,
    ViewOnly,
    Planned,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOverhead {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProbeCatalogEntry {
    pub key: &'static str,
    pub title: &'static str,
    pub status: ProbeStatus,
    pub answers_question: &'static str,
    pub cli_flag: Option<&'static str>,
    pub artifact_files: &'static [&'static str],
    pub default_enabled: bool,
    pub overhead: ProbeOverhead,
    pub requires_privilege_or_kernel_support: bool,
    pub validation_contract: &'static str,
}

pub const PROBE_CATALOG: &[ProbeCatalogEntry] = &[
    ProbeCatalogEntry {
        key: "scheduler_runnable_latency",
        title: "Scheduler runnable latency",
        status: ProbeStatus::Implemented,
        answers_question: "Was a monitored task ready to run but delayed before CPU time?",
        cli_flag: Some("core"),
        artifact_files: &["session.json", "spike_events.json", "interval.json"],
        default_enabled: true,
        overhead: ProbeOverhead::Medium,
        requires_privilege_or_kernel_support: true,
        validation_contract: "session, spike, and interval records are validated by stutter validate and reported through analysis JSON.",
    },
    ProbeCatalogEntry {
        key: "irq_latency",
        title: "IRQ latency",
        status: ProbeStatus::Implemented,
        answers_question: "Did a selected IRQ handler overlap scheduler or frame spikes?",
        cli_flag: Some("--irq-latency --irq <IRQ>"),
        artifact_files: &["irq_events.json"],
        default_enabled: false,
        overhead: ProbeOverhead::Medium,
        requires_privilege_or_kernel_support: true,
        validation_contract: "irq_events.json is optional NDJSON; missing files degrade gracefully and present files validate as IrqEventRecord.",
    },
    ProbeCatalogEntry {
        key: "gpu_hwmon",
        title: "GPU hwmon",
        status: ProbeStatus::Implemented,
        answers_question: "Was the GPU busy, thermally limited, or clock-limited near spikes?",
        cli_flag: Some("--hwmon"),
        artifact_files: &["gpu_samples.json"],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        requires_privilege_or_kernel_support: true,
        validation_contract: "gpu_samples.json is optional NDJSON; hwmon absence is a doctor/preflight condition and reports tolerate missing samples.",
    },
    ProbeCatalogEntry {
        key: "frame_log",
        title: "MangoHud frame log",
        status: ProbeStatus::Implemented,
        answers_question: "Did frame-time outliers line up with scheduler or system evidence?",
        cli_flag: Some("--mangohud-log <PATH>"),
        artifact_files: &["frame_correlation.json", "frame_events.json"],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        requires_privilege_or_kernel_support: false,
        validation_contract: "frame_correlation.json and frame_events.json are optional NDJSON frame contracts with timestamp-alignment data quality.",
    },
    ProbeCatalogEntry {
        key: "block_io",
        title: "Block I/O",
        status: ProbeStatus::Implemented,
        answers_question: "Did block I/O completion latency overlap spikes?",
        cli_flag: Some("--block-io"),
        artifact_files: &["io_events.json"],
        default_enabled: false,
        overhead: ProbeOverhead::Medium,
        requires_privilege_or_kernel_support: true,
        validation_contract: "io_events.json is optional NDJSON; correlation basis is documented and approximate matching downgrades data quality.",
    },
    ProbeCatalogEntry {
        key: "cpu_freq",
        title: "CPU frequency",
        status: ProbeStatus::Implemented,
        answers_question: "Were system CPU frequency samples low or changing near spikes?",
        cli_flag: Some("--cpu-freq / --no-cpu-freq"),
        artifact_files: &["cpu_freq_samples.json"],
        default_enabled: true,
        overhead: ProbeOverhead::Low,
        requires_privilege_or_kernel_support: true,
        validation_contract: "cpu_freq_samples.json is optional NDJSON; samples are system-wide context and reports tolerate missing tracepoint support.",
    },
    ProbeCatalogEntry {
        key: "faults",
        title: "Page faults and stat wait",
        status: ProbeStatus::Implemented,
        answers_question: "Did page-fault deltas or procfs stat wait rise around scheduler spikes?",
        cli_flag: Some("--faults"),
        artifact_files: &["interval.json"],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        requires_privilege_or_kernel_support: false,
        validation_contract: "fault and stat-wait deltas live in interval.json and default to zero when unavailable.",
    },
    ProbeCatalogEntry {
        key: "cpu_perf",
        title: "CPU perf counters",
        status: ProbeStatus::Implemented,
        answers_question: "Was the workload low IPC or cache-miss bound during sampled task intervals?",
        cli_flag: Some("--cpu-perf"),
        artifact_files: &["interval.json"],
        default_enabled: false,
        overhead: ProbeOverhead::High,
        requires_privilege_or_kernel_support: true,
        validation_contract: "CPU perf data lives in interval.json and session task summaries; open/read/skipped status is reflected in data quality.",
    },
    ProbeCatalogEntry {
        key: "psi_timeline",
        title: "PSI interval samples",
        status: ProbeStatus::Implemented,
        answers_question: "Was CPU/memory/I/O pressure present in interval summaries?",
        cli_flag: Some("interval sampling"),
        artifact_files: &["interval.json"],
        default_enabled: true,
        overhead: ProbeOverhead::Low,
        requires_privilege_or_kernel_support: false,
        validation_contract: "PSI fields live in interval.json with serde defaults so older recordings remain readable.",
    },
    ProbeCatalogEntry {
        key: "pressure_stall_timeline_overlay",
        title: "Pressure-stall timeline overlay",
        status: ProbeStatus::ViewOnly,
        answers_question: "Did CPU/memory/I/O pressure line up with spikes?",
        cli_flag: Some("report --analysis-json"),
        artifact_files: &["interval.json"],
        default_enabled: true,
        overhead: ProbeOverhead::Low,
        requires_privilege_or_kernel_support: false,
        validation_contract: "Derived report view from existing interval.json PSI fields; empty when interval data is unavailable.",
    },
    ProbeCatalogEntry {
        key: "drm_fence_latency",
        title: "DRM fence latency",
        status: ProbeStatus::Planned,
        answers_question: "Was frame stutter caused by GPU queue/fence delay rather than CPU runnable delay?",
        cli_flag: None,
        artifact_files: &[],
        default_enabled: false,
        overhead: ProbeOverhead::High,
        requires_privilege_or_kernel_support: true,
        validation_contract: "not implemented; must add schema/docs/fixtures before enabling",
    },
    ProbeCatalogEntry {
        key: "per_thread_runtime_slices",
        title: "Per-thread CPU runtime slices",
        status: ProbeStatus::Planned,
        answers_question: "Was the task ready but delayed, or running but consuming too much CPU time?",
        cli_flag: None,
        artifact_files: &[],
        default_enabled: false,
        overhead: ProbeOverhead::Medium,
        requires_privilege_or_kernel_support: false,
        validation_contract: "not implemented; must add schema/docs/fixtures before enabling",
    },
    ProbeCatalogEntry {
        key: "perf_counter_presets",
        title: "Perf counter presets",
        status: ProbeStatus::Planned,
        answers_question: "Which low-overhead preset should be used for IPC/cache diagnosis?",
        cli_flag: None,
        artifact_files: &[],
        default_enabled: false,
        overhead: ProbeOverhead::High,
        requires_privilege_or_kernel_support: true,
        validation_contract: "not implemented; must add schema/docs/fixtures before enabling",
    },
    ProbeCatalogEntry {
        key: "compositor_frame_pacing_views",
        title: "Compositor/frame-pacing views",
        status: ProbeStatus::Planned,
        answers_question: "Did frame pacing problems correlate with compositor/gamescope scheduler delay?",
        cli_flag: None,
        artifact_files: &[],
        default_enabled: false,
        overhead: ProbeOverhead::Low,
        requires_privilege_or_kernel_support: false,
        validation_contract: "not implemented; must add schema/docs/fixtures before enabling",
    },
];

pub fn probes_command(json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(PROBE_CATALOG)?);
    } else {
        print!("{}", render_probe_catalog(PROBE_CATALOG));
    }
    Ok(())
}

pub fn render_probe_catalog(entries: &[ProbeCatalogEntry]) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "{:<32} {:<12} {:<9} {:<8} {}\n",
        "probe", "status", "default", "overhead", "flag"
    ));
    for entry in entries {
        output.push_str(&format!(
            "{:<32} {:<12} {:<9} {:<8} {}\n",
            entry.key,
            render_status(entry.status),
            if entry.default_enabled { "yes" } else { "no" },
            render_overhead(entry.overhead),
            entry.cli_flag.unwrap_or("-")
        ));
    }
    output
}

fn render_status(status: ProbeStatus) -> &'static str {
    match status {
        ProbeStatus::Implemented => "implemented",
        ProbeStatus::ViewOnly => "view-only",
        ProbeStatus::Planned => "planned",
    }
}

fn render_overhead(overhead: ProbeOverhead) -> &'static str {
    match overhead {
        ProbeOverhead::Low => "low",
        ProbeOverhead::Medium => "medium",
        ProbeOverhead::High => "high",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn probe_catalog_keys_are_unique() {
        let mut keys = BTreeSet::new();
        for entry in PROBE_CATALOG {
            assert!(keys.insert(entry.key), "duplicate probe key {}", entry.key);
        }
    }

    #[test]
    fn implemented_probes_have_artifact_contracts() {
        for entry in PROBE_CATALOG
            .iter()
            .filter(|entry| entry.status == ProbeStatus::Implemented)
        {
            assert!(
                !entry.artifact_files.is_empty(),
                "implemented probe {} has no artifact files",
                entry.key
            );
            assert!(
                !entry.validation_contract.trim().is_empty(),
                "implemented probe {} has no validation contract",
                entry.key
            );
        }
    }

    #[test]
    fn planned_probes_are_default_off() {
        for entry in PROBE_CATALOG
            .iter()
            .filter(|entry| entry.status == ProbeStatus::Planned)
        {
            assert!(!entry.default_enabled, "planned probe {} is on", entry.key);
            assert!(
                entry.artifact_files.is_empty(),
                "planned probe has artifacts"
            );
            assert!(
                entry.validation_contract.contains("not implemented"),
                "planned probe {} must say it is not implemented",
                entry.key
            );
        }
    }

    #[test]
    fn high_overhead_probes_are_default_off() {
        for entry in PROBE_CATALOG
            .iter()
            .filter(|entry| entry.overhead == ProbeOverhead::High)
        {
            assert!(
                !entry.default_enabled,
                "high-overhead probe {} is default-enabled",
                entry.key
            );
        }
    }

    #[test]
    fn probe_catalog_json_serializes() {
        let json = serde_json::to_string_pretty(PROBE_CATALOG).unwrap();
        assert!(json.contains("scheduler_runnable_latency"));
        assert!(json.contains("implemented"));
    }

    #[test]
    fn render_probe_catalog_mentions_core_probe() {
        let output = render_probe_catalog(PROBE_CATALOG);
        assert!(output.contains("scheduler_runnable_latency"));
        assert!(output.contains("implemented"));
        assert!(output.contains("core"));
    }
}
