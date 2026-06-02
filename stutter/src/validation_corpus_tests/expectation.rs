use serde::Deserialize;

use crate::{
    diagnosis::{Confidence, StutterCause},
    report::DataQualityLevel,
};

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct ExpectedArtifacts {
    pub(super) spikes: Option<u64>,
    pub(super) spikes_min: Option<u64>,
    pub(super) intervals: Option<u64>,
    pub(super) intervals_min: Option<u64>,
    pub(super) irq_events: Option<u64>,
    pub(super) irq_events_min: Option<u64>,
    pub(super) gpu_samples: Option<u64>,
    pub(super) gpu_samples_min: Option<u64>,
    pub(super) frames: Option<u64>,
    pub(super) frames_min: Option<u64>,
    pub(super) block_io_events: Option<u64>,
    pub(super) block_io_events_min: Option<u64>,
    pub(super) foreground_events: Option<u64>,
    pub(super) foreground_events_min: Option<u64>,
    pub(super) kms_flip_events: Option<u64>,
    pub(super) kms_flip_events_min: Option<u64>,
    pub(super) drm_fence_events: Option<u64>,
    pub(super) drm_fence_events_min: Option<u64>,
    pub(super) wayland_presentation_events: Option<u64>,
    pub(super) wayland_presentation_events_min: Option<u64>,
    pub(super) dmabuf_events: Option<u64>,
    pub(super) dmabuf_events_min: Option<u64>,
    pub(super) gpu_engine_samples: Option<u64>,
    pub(super) gpu_engine_samples_min: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FixtureExpectationFile {
    pub(super) name: String,
    pub(super) schema_version: u32,
    pub(super) source: String,
    #[serde(default)]
    pub(super) quality_expectation: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) platform: Option<PlatformExpectations>,
    pub(super) expected: ExpectedFromToml,
    #[serde(default)]
    pub(super) privacy: Option<PrivacyExpectations>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct PlatformExpectations {
    #[serde(default)]
    pub(super) gpu_vendor: String,
    #[serde(default)]
    pub(super) gpu_driver: String,
    #[serde(default)]
    pub(super) compositor: String,
    #[serde(default)]
    pub(super) session_type: String,
    #[serde(default)]
    pub(super) scenario: String,
    #[serde(default)]
    pub(super) sanitized_capture_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ExpectedFromToml {
    pub(super) primary_cause: String,
    #[serde(default)]
    pub(super) required_candidate: Option<String>,
    #[serde(default)]
    pub(super) required_candidate_evidence: Vec<String>,
    #[serde(default)]
    pub(super) accepted_confidence: Vec<String>,
    #[serde(default)]
    pub(super) quality_reasons_contain: Vec<String>,
    pub(super) data_quality: String,
    #[serde(default)]
    pub(super) artifacts: ExpectedArtifactsFromToml,
    #[serde(default)]
    pub(super) evidence: ExpectedEvidenceFromToml,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct ExpectedArtifactsFromToml {
    pub(super) spikes: Option<u64>,
    pub(super) spikes_min: Option<u64>,
    pub(super) intervals: Option<u64>,
    pub(super) intervals_min: Option<u64>,
    pub(super) irq_events: Option<u64>,
    pub(super) irq_events_min: Option<u64>,
    pub(super) gpu_samples: Option<u64>,
    pub(super) gpu_samples_min: Option<u64>,
    pub(super) frames: Option<u64>,
    pub(super) frames_min: Option<u64>,
    pub(super) block_io_events: Option<u64>,
    pub(super) block_io_events_min: Option<u64>,
    pub(super) foreground_events: Option<u64>,
    pub(super) foreground_events_min: Option<u64>,
    pub(super) kms_flip_events: Option<u64>,
    pub(super) kms_flip_events_min: Option<u64>,
    pub(super) drm_fence_events: Option<u64>,
    pub(super) drm_fence_events_min: Option<u64>,
    pub(super) wayland_presentation_events: Option<u64>,
    pub(super) wayland_presentation_events_min: Option<u64>,
    pub(super) dmabuf_events: Option<u64>,
    pub(super) dmabuf_events_min: Option<u64>,
    pub(super) gpu_engine_samples: Option<u64>,
    pub(super) gpu_engine_samples_min: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct ExpectedEvidenceFromToml {
    pub(super) contains: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct PrivacyExpectations {
    #[serde(default)]
    pub(super) titles_redacted: bool,
    #[serde(default)]
    pub(super) paths_redacted: bool,
    #[serde(default)]
    pub(super) hostnames_redacted: bool,
    #[serde(default)]
    pub(super) usernames_redacted: bool,
}

pub(super) fn parse_data_quality(value: &str) -> DataQualityLevel {
    match value {
        "High" => DataQualityLevel::High,
        "Medium" => DataQualityLevel::Medium,
        "Low" => DataQualityLevel::Low,
        other => panic!("unknown data quality level in fixture metadata: {other}"),
    }
}

pub(super) fn parse_confidence(value: &str) -> Confidence {
    match value {
        "Low" => Confidence::Low,
        "Medium" => Confidence::Medium,
        "High" => Confidence::High,
        other => panic!("unknown confidence level in fixture metadata: {other}"),
    }
}

pub(super) fn parse_stutter_cause(value: &str) -> StutterCause {
    match value {
        "CompositorSchedulerDelay" => StutterCause::CompositorSchedulerDelay,
        "GameThreadSchedulerDelay" => StutterCause::GameThreadSchedulerDelay,
        "IrqDelayCandidate" => StutterCause::IrqDelayCandidate,
        "GpuBoundCandidate" => StutterCause::GpuBoundCandidate,
        "BlockIoCandidate" => StutterCause::BlockIoCandidate,
        "CpuPressureCandidate" => StutterCause::CpuPressureCandidate,
        other => panic!("unknown stutter cause in fixture metadata: {other}"),
    }
}

pub(super) enum ExpectedPrimaryCause {
    Any,
    NoneOrUnknown,
    Cause(StutterCause),
}

pub(super) fn parse_primary_cause(value: &str) -> ExpectedPrimaryCause {
    match value {
        "Any" => ExpectedPrimaryCause::Any,
        "Unknown" => ExpectedPrimaryCause::NoneOrUnknown,
        other => ExpectedPrimaryCause::Cause(parse_stutter_cause(other)),
    }
}
