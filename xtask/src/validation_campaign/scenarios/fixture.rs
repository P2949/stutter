//! Fixture metadata generated from the predeclared scenario, never from Stutter's diagnosis
//! output. The output reuses the existing fixture schema (`evidence_kind`, `[provenance]` with
//! its 18 required keys, `[coverage]`, `[platform]`, `[expected]`, `[privacy]`) so one corpus
//! contract serves both the calibrator and `fixture-coverage`.
use std::{collections::BTreeMap, fs, path::Path};

use serde::Serialize;
use stutter::api::recorder::SESSION_SCHEMA_VERSION;

use super::model::{ControlType, ScenarioSpec};
use crate::evidence_bundle::sha256::sha256_hex;

/// Facts about the host, bucketed so no machine-identifying detail reaches a fixture.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HostFacts {
    pub kernel: String,
    pub distribution: String,
    pub cpu_vendor: String,
    pub cpu_topology_bucket: String,
    pub gpu_vendor: String,
    pub gpu_driver: String,
    pub compositor: String,
    pub session_type: String,
    pub workload_version: String,
}

/// Everything fixture generation needs about one accepted capture.
#[derive(Clone, Debug)]
pub struct CaptureRecord<'a> {
    pub spec: &'a ScenarioSpec,
    pub campaign_id: &'a str,
    pub attempt: u32,
    pub raw_capture_id: &'a str,
    pub source_commit: &'a str,
    pub host: &'a HostFacts,
    pub probe_set: &'a str,
    pub drop_counters: u64,
    /// `data_quality.level` reported for the recording (a statement about the data).
    pub data_quality: &'a str,
    pub artifact_counts: &'a BTreeMap<String, u64>,
}

/// Stable, privacy-safe identity: a hash of campaign, scenario, attempt and the raw capture id.
pub fn sanitized_capture_id(
    campaign_id: &str,
    scenario: &str,
    attempt: u32,
    raw_capture_id: &str,
) -> String {
    let digest = sha256_hex(
        format!("{campaign_id}\u{1f}{scenario}\u{1f}{attempt}\u{1f}{raw_capture_id}").as_bytes(),
    );
    format!("sc-{}", &digest[..24])
}

/// Directory and fixture name: `real_<scenario>_<id suffix>`.
pub fn fixture_name(scenario: &str, capture_id: &str) -> String {
    format!(
        "real_{}_{}",
        scenario.replace('-', "_"),
        &capture_id[capture_id.len() - 8..]
    )
}

#[derive(Serialize)]
struct Platform {
    gpu_vendor: String,
    gpu_driver: String,
    compositor: String,
    session_type: String,
    scenario: String,
    sanitized_capture_id: String,
    kernel_family: String,
    cpu_vendor: String,
    cpu_topology_bucket: String,
    capture_features: Vec<String>,
}

#[derive(Serialize)]
struct Coverage {
    diagnoses: Vec<String>,
}

#[derive(Serialize)]
struct Expected {
    primary_cause: String,
    expected_behavior: String,
    accepted_confidence: Vec<String>,
    data_quality: String,
    artifacts: BTreeMap<String, u64>,
    evidence: Evidence,
}

#[derive(Serialize)]
struct Evidence {
    contains: Vec<String>,
}

#[derive(Serialize)]
struct Privacy {
    titles_redacted: bool,
    paths_redacted: bool,
    hostnames_redacted: bool,
    usernames_redacted: bool,
}

#[derive(Serialize)]
struct FixtureFile {
    name: String,
    schema_version: u32,
    source: String,
    evidence_kind: String,
    quality_expectation: String,
    description: String,
    provenance: BTreeMap<String, String>,
    coverage: Coverage,
    platform: Platform,
    expected: Expected,
    privacy: Privacy,
}

/// Artifact stream name in `fixture.toml` -> file in the recording.
const STREAMS: [(&str, &str); 6] = [
    ("spikes", "spike_events.json"),
    ("intervals", "interval.json"),
    ("irq_events", "irq_events.json"),
    ("gpu_samples", "gpu_samples.json"),
    ("frames", "frame_correlation.json"),
    ("block_io_events", "io_events.json"),
];

/// Exact record counts per stream (non-empty lines); a missing stream counts zero.
pub fn count_artifacts(dir: &Path) -> BTreeMap<String, u64> {
    STREAMS
        .iter()
        .map(|(name, file)| {
            let count = fs::read_to_string(dir.join(file))
                .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count() as u64)
                .unwrap_or(0);
            ((*name).to_owned(), count)
        })
        .collect()
}

fn description(spec: &ScenarioSpec) -> String {
    match spec.control {
        ControlType::Positive => format!(
            "Real capture of a known intervention ({}): ground truth is the applied intervention.",
            spec.id
        ),
        ControlType::NegativeControl => {
            format!(
                "Real matched negative control ({}): the intervention is absent.",
                spec.id
            )
        }
        ControlType::MixedConfounded => format!(
            "Real capture of deliberately combined interventions ({}); the primary cause is predeclared.",
            spec.id
        ),
    }
}

pub fn render(record: &CaptureRecord<'_>) -> Result<String, toml::ser::Error> {
    let spec = record.spec;
    let capture_id = sanitized_capture_id(
        record.campaign_id,
        &spec.id,
        record.attempt,
        record.raw_capture_id,
    );
    let host = record.host;
    let provenance = [
        ("source_commit", record.source_commit.to_owned()),
        ("capture_id", capture_id.clone()),
        ("kernel", host.kernel.clone()),
        ("distribution", host.distribution.clone()),
        ("cpu", format!("{} {}", host.cpu_vendor, host.cpu_topology_bucket)),
        ("gpu", format!("{} {}", host.gpu_vendor, host.gpu_driver)),
        ("compositor", host.compositor.clone()),
        ("workload", format!("{:?}", spec.target)),
        ("workload_version", host.workload_version.clone()),
        ("scenario", spec.id.clone()),
        ("expected_cause", spec.expected_primary_cause.clone()),
        (
            "expected_confidence",
            match spec.control {
                ControlType::NegativeControl => "none",
                _ => "medium_or_high",
            }
            .to_owned(),
        ),
        ("control_type", spec.control.as_str().to_owned()),
        ("probe_set", record.probe_set.to_owned()),
        ("drop_counters", record.drop_counters.to_string()),
        ("artifact_schema", SESSION_SCHEMA_VERSION.get().to_string()),
        (
            "limitations",
            "single host; ground truth is the predeclared intervention; overlap is not proven causality".to_owned(),
        ),
        ("privacy_state", "sanitized".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    let accepted = match spec.control {
        ControlType::NegativeControl => Vec::new(),
        _ => vec!["Medium".to_owned(), "High".to_owned()],
    };
    let file = FixtureFile {
        name: fixture_name(&spec.id, &capture_id),
        schema_version: SESSION_SCHEMA_VERSION.get(),
        source: "sanitized-real-recording".to_owned(),
        evidence_kind: "fixture_real".to_owned(),
        quality_expectation: record.data_quality.to_owned(),
        description: description(spec),
        provenance,
        coverage: Coverage {
            diagnoses: spec
                .covers
                .iter()
                .map(|f| f.positive_cause().to_owned())
                .collect(),
        },
        platform: Platform {
            gpu_vendor: host.gpu_vendor.clone(),
            gpu_driver: host.gpu_driver.clone(),
            compositor: host.compositor.clone(),
            session_type: host.session_type.clone(),
            scenario: spec.id.clone(),
            sanitized_capture_id: capture_id,
            kernel_family: "linux".to_owned(),
            cpu_vendor: host.cpu_vendor.clone(),
            cpu_topology_bucket: host.cpu_topology_bucket.clone(),
            capture_features: record
                .artifact_counts
                .iter()
                .filter(|(_, count)| **count > 0)
                .map(|(name, _)| name.clone())
                .collect(),
        },
        expected: Expected {
            primary_cause: spec.expected_primary_cause.clone(),
            expected_behavior: if spec.informational {
                "informational"
            } else {
                "must_diagnose"
            }
            .to_owned(),
            accepted_confidence: accepted,
            data_quality: record.data_quality.to_owned(),
            artifacts: record.artifact_counts.clone(),
            evidence: Evidence {
                contains: Vec::new(),
            },
        },
        privacy: Privacy {
            titles_redacted: true,
            paths_redacted: true,
            hostnames_redacted: true,
            usernames_redacted: true,
        },
    };
    toml::to_string(&file)
}
