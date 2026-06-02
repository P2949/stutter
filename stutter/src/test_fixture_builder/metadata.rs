//! Fixture expectation and metadata construction.

use super::*;

mod real_matrix;

#[derive(serde::Serialize)]
pub(super) struct FixtureMetadata {
    name: String,
    schema_version: u32,
    source: String,
    quality_expectation: String,
    description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    platform: Option<FixturePlatform>,
    expected: FixtureExpected,
    privacy: FixturePrivacy,
}

#[derive(Clone, serde::Serialize)]
pub(super) struct FixturePlatform {
    gpu_vendor: String,
    gpu_driver: String,
    compositor: String,
    session_type: String,
    scenario: String,
    sanitized_capture_id: String,
}

#[derive(serde::Serialize)]
struct FixtureExpected {
    primary_cause: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    required_candidate: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_candidate_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    quality_reasons_contain: Vec<String>,
    accepted_confidence: Vec<String>,
    data_quality: String,
    artifacts: FixtureExpectedArtifacts,
    evidence: FixtureExpectedEvidence,
}

#[derive(Default, serde::Serialize)]
struct FixtureExpectedArtifacts {
    #[serde(skip_serializing_if = "Option::is_none")]
    spikes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spikes_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intervals: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    intervals_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    irq_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    irq_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_samples: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_samples_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_io_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_io_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kms_flip_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kms_flip_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    drm_fence_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    drm_fence_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wayland_presentation_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wayland_presentation_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dmabuf_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dmabuf_events_min: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_engine_samples: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_engine_samples_min: Option<u64>,
}

#[derive(serde::Serialize)]
struct FixtureExpectedEvidence {
    contains: Vec<String>,
}

#[derive(serde::Serialize)]
struct FixturePrivacy {
    titles_redacted: bool,
    paths_redacted: bool,
    hostnames_redacted: bool,
    usernames_redacted: bool,
}

macro_rules! fixture_metadata {
    (
        $name:expr,
        $source:expr,
        $quality_expectation:expr,
        $description:expr,
        $primary_cause:expr,
        $accepted_confidence:expr,
        $data_quality:expr,
        $evidence_contains:expr,
        $artifacts:expr $(,)?
    ) => {
        fixture_metadata(FixtureMetadataInput {
            name: $name,
            source: $source,
            quality_expectation: $quality_expectation,
            description: $description,
            primary_cause: $primary_cause,
            accepted_confidence: $accepted_confidence,
            data_quality: $data_quality,
            evidence_contains: $evidence_contains,
            artifacts: $artifacts,
        })
    };
}

pub(super) fn fixture_metadata_for(name: &str, artifacts: &FixtureArtifacts) -> FixtureMetadata {
    if let Some(metadata) = real_matrix::fixture_metadata_for_real_matrix(name, artifacts) {
        return metadata;
    }

    match name {
        "clean_run" => fixture_metadata!(
            name,
            "synthetic-contract",
            "High",
            "Synthetic clean run fixture that should remain high quality and produce no strong diagnosis.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "cpu_pressure" => fixture_metadata!(
            name,
            "synthetic-contract",
            "High",
            "Synthetic CPU pressure fixture with high CPU PSI near a scheduler-latency spike.",
            "CpuPressureCandidate",
            &["Medium", "High"],
            "High",
            &["high CPU PSI"],
            exact_artifacts(artifacts),
        ),
        "block_io_stall" => fixture_metadata!(
            name,
            "synthetic-contract",
            "High",
            "Synthetic block I/O fixture with a long request overlapping a scheduler-latency spike.",
            "BlockIoCandidate",
            &["Medium", "High"],
            "High",
            &["block I/O"],
            exact_artifacts(artifacts),
        ),
        "irq_heavy" => fixture_metadata!(
            name,
            "synthetic-contract",
            "High",
            "Synthetic IRQ fixture with a long IRQ handler overlapping scheduler-latency spikes.",
            "IrqDelayCandidate",
            &["Medium", "High"],
            "High",
            &["IRQ"],
            exact_artifacts(artifacts),
        ),
        "gpu_bound_clean_cpu" => fixture_metadata!(
            name,
            "synthetic-contract",
            "High",
            "Synthetic GPU-bound fixture with high GPU busy and clean CPU pressure.",
            "GpuBoundCandidate",
            &["Low", "Medium", "High"],
            "High",
            &["GPU busy"],
            exact_artifacts(artifacts),
        ),
        "truncated_drop_counters" => with_quality_reasons(
            fixture_metadata!(
                name,
                "synthetic-contract",
                "Medium",
                "Synthetic low-quality fixture with truncated spike events and non-zero drop counters.",
                "Unknown",
                &[],
                "Medium",
                &[],
                exact_artifacts(artifacts),
            ),
            &["truncated", "drop"],
        ),
        "reused_tid_no_contamination" => fixture_metadata!(
            name,
            "synthetic-contract",
            "High",
            "Synthetic reused-TID fixture that verifies separate logical tasks are not merged.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "old_schema_warning" => with_quality_reasons(
            fixture_metadata!(
                name,
                "synthetic-contract",
                "Medium",
                "Synthetic old-schema fixture that should warn without being rejected.",
                "Unknown",
                &[],
                "Medium",
                &[],
                exact_artifacts(artifacts),
            ),
            &["older than current"],
        ),
        "game_thread_scheduler_delay" => fixture_metadata!(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture for game main/render thread scheduler delay during a visible frame spike.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "compositor_scheduler_delay" => fixture_metadata!(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture for compositor thread scheduler delay during a visible frame spike.",
            "CompositorSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["compositor thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "real_gpu_bound_looking" => {
            let mut metadata = fixture_metadata!(
                name,
                "sanitized-real-recording",
                "High",
                "GPU busy was high during a visible frame spike; scheduler evidence may also exist, so GPU-bound is required as a candidate rather than always primary.",
                "Any",
                &[],
                "High",
                &[],
                exact_artifacts(artifacts),
            );
            metadata.expected.required_candidate = Some("GpuBoundCandidate".to_owned());
            metadata.expected.required_candidate_evidence = vec!["GPU busy".to_owned()];
            metadata
        }
        "real_block_io_overlap" => fixture_metadata!(
            name,
            "sanitized-real-recording",
            "High",
            "Block I/O request overlapped the scheduler-latency cluster while unrelated block I/O occurred outside the correlation window.",
            "BlockIoCandidate",
            &["Medium", "High"],
            "High",
            &["block I/O"],
            exact_artifacts(artifacts),
        ),
        "real_truncated_low_quality" => with_quality_reasons(
            fixture_metadata!(
                name,
                "sanitized-real-recording",
                "Medium",
                "Sanitized low-quality recording with truncated spike events and nonzero drop counters; quality handling is the regression target, not diagnosis cause detection.",
                "Unknown",
                &[],
                "Medium",
                &[],
                exact_artifacts(artifacts),
            ),
            &["truncated", "drop"],
        ),
        "real_foreground_window" => fixture_metadata!(
            name,
            "sanitized-real-recording",
            "High",
            "Sanitized foreground-window recording with a scheduler cluster near a foreground event; title is redacted while PID/app/class remain available.",
            "Any",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "real_community_rules_classification" => fixture_metadata!(
            name,
            "sanitized-real-recording",
            "High",
            "Sanitized community-rules classification fixture where an originally unknown game process is represented in the final artifact stream as TaskClass::Game.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "foreground_window" => fixture_metadata!(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture that verifies foreground PID/app/class are preserved while title is redacted.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "community_rules_classification" => fixture_metadata!(
            name,
            "synthetic-edge-case",
            "High",
            "Synthetic edge-case fixture that verifies a community-classified game task remains classified as Game.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "direct_gpu_clean" => fixture_metadata!(
            name,
            "synthetic-display-path",
            "High",
            "Synthetic display-path fixture for a clean direct render-and-scanout run.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "uhd630_cross_gpu_fence_wait" => fixture_metadata!(
            name,
            "synthetic-display-path",
            "High",
            "Synthetic display-path fixture with UHD630/i915 scanout and high-confidence cross-GPU fence waits near a frame outlier.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "uhd630_composited_blitter" => fixture_metadata!(
            name,
            "synthetic-display-path",
            "High",
            "Synthetic display-path fixture with composited presentation and iGPU blitter activity near a frame outlier.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "uhd630_kms_delay" => fixture_metadata!(
            name,
            "synthetic-display-path",
            "High",
            "Synthetic display-path fixture with UHD630/i915 scanout and KMS/pageflip delay near a frame outlier.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "wayland_zero_copy_good" => fixture_metadata!(
            name,
            "synthetic-display-path",
            "High",
            "Synthetic display-path fixture where cooperative Wayland evidence reports zero-copy/direct scanout.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "dmabuf_modifier_mismatch" => fixture_metadata!(
            name,
            "synthetic-display-path",
            "High",
            "Synthetic display-path fixture where cooperative DMABUF evidence reports a modifier mismatch and copy-required candidate.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "missing_evidence_unknown" => fixture_metadata!(
            name,
            "synthetic-display-path",
            "High",
            "Synthetic display-path fixture with no optional display-path evidence, which should keep suspicion low-confidence.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "clean_baseline" => fixture_metadata!(
            name,
            "public-example",
            "High",
            "Small public clean baseline example.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "game_thread_scheduler_delay_public" => fixture_metadata!(
            name,
            "public-example",
            "High",
            "Small public game-thread scheduler delay example.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "low_quality_truncated" => with_quality_reasons(
            fixture_metadata!(
                name,
                "public-example",
                "Medium",
                "Small public low-quality truncated example.",
                "Unknown",
                &[],
                "Medium",
                &[],
                exact_artifacts(artifacts),
            ),
            &["truncated", "drop"],
        ),
        "game_scheduler_pressure" => fixture_metadata!(
            name,
            "autotune-replay",
            "High",
            "Autotune replay fixture with game scheduler pressure.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread"],
            exact_artifacts(artifacts),
        ),
        "gpu_bound" => fixture_metadata!(
            name,
            "autotune-replay",
            "High",
            "Autotune replay fixture for a GPU-bound run.",
            "GpuBoundCandidate",
            &["Low", "Medium", "High"],
            "High",
            &["GPU busy"],
            exact_artifacts(artifacts),
        ),
        "low_quality" => with_quality_reasons(
            fixture_metadata!(
                name,
                "autotune-replay",
                "Medium",
                "Autotune replay fixture for a low-quality run with dropped or truncated data.",
                "Unknown",
                &[],
                "Medium",
                &[],
                exact_artifacts(artifacts),
            ),
            &["truncated", "drop"],
        ),
        "real_clean_baseline" => fixture_metadata!(
            name,
            "validation-corpus",
            "High",
            "Real clean baseline example.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
        "real_compositor_scheduler_delay" => fixture_metadata!(
            name,
            "sanitized-real-recording",
            "High",
            "Compositor or gamescope thread had scheduler delay during a visible frame spike.",
            "CompositorSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["compositor thread"],
            exact_artifacts(artifacts),
        ),
        "real_game_thread_scheduler_delay" => fixture_metadata!(
            name,
            "validation-corpus",
            "High",
            "Real game-thread scheduler delay example.",
            "GameThreadSchedulerDelay",
            &["Medium", "High"],
            "High",
            &["game thread", "delayed"],
            exact_artifacts(artifacts),
        ),
        "real_irq_overlap" => fixture_metadata!(
            name,
            "sanitized-real-recording",
            "High",
            "IRQ handler activity overlapped the scheduler-latency cluster while unrelated IRQ noise occurred outside the correlation window.",
            "IrqDelayCandidate",
            &["Medium", "High"],
            "High",
            &["IRQ"],
            exact_artifacts(artifacts),
        ),
        other => fixture_metadata!(
            other,
            "synthetic-contract",
            "High",
            "Generated synthetic fixture.",
            "Unknown",
            &[],
            "High",
            &[],
            exact_artifacts(artifacts),
        ),
    }
}

fn with_quality_reasons(
    mut metadata: FixtureMetadata,
    quality_reasons_contain: &[&str],
) -> FixtureMetadata {
    metadata.expected.quality_reasons_contain = quality_reasons_contain
        .iter()
        .map(|item| (*item).to_owned())
        .collect();
    metadata
}

fn with_platform(mut metadata: FixtureMetadata, platform: FixturePlatform) -> FixtureMetadata {
    metadata.platform = Some(platform);
    metadata
}

fn real_platform(
    gpu_vendor: &str,
    gpu_driver: &str,
    compositor: &str,
    session_type: &str,
    scenario: &str,
    sanitized_capture_id: &str,
) -> FixturePlatform {
    FixturePlatform {
        gpu_vendor: gpu_vendor.to_owned(),
        gpu_driver: gpu_driver.to_owned(),
        compositor: compositor.to_owned(),
        session_type: session_type.to_owned(),
        scenario: scenario.to_owned(),
        sanitized_capture_id: sanitized_capture_id.to_owned(),
    }
}

struct FixtureMetadataInput<'a> {
    name: &'a str,
    source: &'a str,
    quality_expectation: &'a str,
    description: &'a str,
    primary_cause: &'a str,
    accepted_confidence: &'a [&'a str],
    data_quality: &'a str,
    evidence_contains: &'a [&'a str],
    artifacts: FixtureExpectedArtifacts,
}

fn fixture_metadata(input: FixtureMetadataInput<'_>) -> FixtureMetadata {
    FixtureMetadata {
        name: input.name.to_owned(),
        schema_version: SESSION_SCHEMA_VERSION.get(),
        source: input.source.to_owned(),
        quality_expectation: input.quality_expectation.to_owned(),
        description: input.description.to_owned(),
        platform: None,
        expected: FixtureExpected {
            primary_cause: input.primary_cause.to_owned(),
            required_candidate: None,
            required_candidate_evidence: Vec::new(),
            quality_reasons_contain: Vec::new(),
            accepted_confidence: input
                .accepted_confidence
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            data_quality: input.data_quality.to_owned(),
            artifacts: input.artifacts,
            evidence: FixtureExpectedEvidence {
                contains: input
                    .evidence_contains
                    .iter()
                    .map(|item| (*item).to_owned())
                    .collect(),
            },
        },
        privacy: FixturePrivacy {
            titles_redacted: true,
            paths_redacted: true,
            hostnames_redacted: true,
            usernames_redacted: true,
        },
    }
}

fn exact_artifacts(artifacts: &FixtureArtifacts) -> FixtureExpectedArtifacts {
    FixtureExpectedArtifacts {
        spikes: Some(artifacts.spikes.len() as u64),
        spikes_min: None,
        intervals: Some(artifacts.intervals.len() as u64),
        intervals_min: None,
        irq_events: Some(artifacts.irq_events.len() as u64),
        irq_events_min: None,
        gpu_samples: Some(artifacts.gpu_samples.len() as u64),
        gpu_samples_min: None,
        frames: Some(artifacts.frame_events.len() as u64),
        frames_min: None,
        block_io_events: Some(artifacts.block_io_events.len() as u64),
        block_io_events_min: None,
        foreground_events: Some(artifacts.foreground_events.len() as u64),
        foreground_events_min: None,
        kms_flip_events: Some(artifacts.kms_flip_events.len() as u64),
        kms_flip_events_min: None,
        drm_fence_events: Some(artifacts.drm_fence_events.len() as u64),
        drm_fence_events_min: None,
        wayland_presentation_events: Some(artifacts.wayland_presentation_events.len() as u64),
        wayland_presentation_events_min: None,
        dmabuf_events: Some(artifacts.dmabuf_events.len() as u64),
        dmabuf_events_min: None,
        gpu_engine_samples: Some(artifacts.gpu_engine_samples.len() as u64),
        gpu_engine_samples_min: None,
    }
}
