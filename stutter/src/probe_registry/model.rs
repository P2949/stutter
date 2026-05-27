use serde::Serialize;

use super::probe_key::ProbeKey;
use crate::{
    artifacts::ArtifactKind,
    probe_catalog::{ProbeOverhead, ProbeStatus},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCapability {
    Ebpf,
    Tracepoint,
    PerfEvent,
    Procfs,
    Hwmon,
    ExternalLog,
    WindowSystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EbpfProgramSpec {
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TracepointSpec {
    pub category: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PerfEventSpec {
    pub name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DataQualityRule {
    pub key: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProbeSpec {
    pub key: ProbeKey,
    pub catalog_key: &'static str,
    pub title: &'static str,
    pub status: ProbeStatus,
    pub answers_question: &'static str,
    pub cli_flags: &'static [&'static str],
    pub artifacts: &'static [ArtifactKind],
    pub default_enabled: bool,
    pub overhead: ProbeOverhead,
    pub required_capabilities: &'static [ProbeCapability],
    pub ebpf_programs: &'static [EbpfProgramSpec],
    pub tracepoints: &'static [TracepointSpec],
    pub perf_events: &'static [PerfEventSpec],
    pub data_quality_rules: &'static [DataQualityRule],
    pub validation_contract: &'static str,
}
