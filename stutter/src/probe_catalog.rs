use serde::Serialize;

use crate::{
    artifacts::artifact_spec,
    probe_registry::{PROBE_REGISTRY, ProbeCapability},
};

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

#[derive(Debug, Clone, Serialize)]
pub struct ProbeCatalogEntry {
    pub key: &'static str,
    pub title: &'static str,
    pub status: ProbeStatus,
    pub answers_question: &'static str,
    pub cli_flag: Option<&'static str>,
    pub artifact_files: Vec<&'static str>,
    pub default_enabled: bool,
    pub overhead: ProbeOverhead,
    pub requires_privilege_or_kernel_support: bool,
    pub validation_contract: &'static str,
}

pub fn probe_catalog_entries() -> Vec<ProbeCatalogEntry> {
    PROBE_REGISTRY
        .iter()
        .map(|spec| {
            let artifact_files = spec
                .artifacts
                .iter()
                .flat_map(|kind| {
                    let artifact = artifact_spec(*kind);
                    std::iter::once(artifact.file_name)
                        .chain(artifact.legacy_aliases.iter().copied())
                })
                .collect();

            ProbeCatalogEntry {
                key: spec.catalog_key,
                title: spec.title,
                status: spec.status,
                answers_question: spec.answers_question,
                cli_flag: spec.cli_flags.first().copied(),
                artifact_files,
                default_enabled: spec.default_enabled,
                overhead: spec.overhead,
                requires_privilege_or_kernel_support: spec.required_capabilities.iter().any(
                    |capability| {
                        matches!(
                            capability,
                            ProbeCapability::Ebpf
                                | ProbeCapability::Tracepoint
                                | ProbeCapability::PerfEvent
                                | ProbeCapability::Hwmon
                        )
                    },
                ),
                validation_contract: spec.validation_contract,
            }
        })
        .collect()
}

pub fn probes_command(json: bool) -> anyhow::Result<()> {
    let entries = probe_catalog_entries();
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        print!("{}", render_probe_catalog(&entries));
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
        let entries = probe_catalog_entries();
        let mut keys = BTreeSet::new();
        for entry in &entries {
            assert!(keys.insert(entry.key), "duplicate probe key {}", entry.key);
        }
    }

    #[test]
    fn implemented_probes_have_artifact_contracts() {
        let entries = probe_catalog_entries();
        for entry in entries
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
        let entries = probe_catalog_entries();
        for entry in entries
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
        let entries = probe_catalog_entries();
        for entry in entries
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
        let entries = probe_catalog_entries();
        let json = serde_json::to_string_pretty(&entries).unwrap();
        assert!(json.contains("scheduler_runnable_latency"));
        assert!(json.contains("implemented"));
    }

    #[test]
    fn render_probe_catalog_mentions_core_probe() {
        let entries = probe_catalog_entries();
        let output = render_probe_catalog(&entries);
        assert!(output.contains("scheduler_runnable_latency"));
        assert!(output.contains("implemented"));
        assert!(output.contains("core"));
    }

    #[test]
    fn probe_catalog_mentions_foreground_window_probe() {
        let entries = probe_catalog_entries();
        let entry = entries
            .iter()
            .find(|entry| entry.key == "foreground_window")
            .expect("foreground_window probe catalog entry must exist");

        assert_eq!(entry.title, "Foreground window context");
        assert_eq!(entry.status, ProbeStatus::Implemented);
        assert_eq!(
            entry.answers_question,
            "Which application/window was foreground near scheduler or frame spikes?"
        );
        assert_eq!(
            entry.cli_flag,
            Some("--foreground-window / --focus-source foreground")
        );
        assert_eq!(
            entry.artifact_files,
            &["foreground_events.json", "focus_events.json"]
        );
        assert!(!entry.default_enabled);
        assert_eq!(entry.overhead, ProbeOverhead::Low);
        assert!(!entry.requires_privilege_or_kernel_support);
        assert!(
            entry
                .validation_contract
                .contains("window titles are redacted by default")
        );

        let rendered = render_probe_catalog(&entries);
        assert!(rendered.contains("foreground_window"));
        assert!(rendered.contains("--foreground-window / --focus-source foreground"));
    }

    #[test]
    fn drm_fence_latency_is_implemented_but_default_off() {
        let entries = probe_catalog_entries();
        let entry = entries
            .iter()
            .find(|entry| entry.key == "drm_fence_latency")
            .expect("drm_fence_latency probe catalog entry must exist");

        assert_eq!(entry.status, ProbeStatus::Implemented);
        assert!(!entry.default_enabled);
        assert_eq!(entry.overhead, ProbeOverhead::High);
    }

    #[test]
    fn per_thread_runtime_slices_is_implemented_but_default_off() {
        let entries = probe_catalog_entries();
        let entry = entries
            .iter()
            .find(|entry| entry.key == "per_thread_runtime_slices")
            .expect("per_thread_runtime_slices probe catalog entry must exist");

        assert_eq!(entry.status, ProbeStatus::Implemented);
        assert_eq!(entry.cli_flag, Some("--runtime-slices"));
        assert_eq!(entry.artifact_files, &["runtime_slices.json"]);
        assert!(!entry.default_enabled);
    }

    #[test]
    fn catalog_entries_are_direct_views_of_registry_specs() {
        let entries = probe_catalog_entries();
        assert_eq!(entries.len(), PROBE_REGISTRY.len());

        for (entry, spec) in entries.iter().zip(PROBE_REGISTRY.iter()) {
            assert_eq!(entry.key, spec.catalog_key);
            assert_eq!(entry.title, spec.title);
            assert_eq!(entry.status, spec.status);
            assert_eq!(entry.default_enabled, spec.default_enabled);
        }
    }
}
