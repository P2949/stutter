//! Metadata for sanitized-real validation matrix fixtures.

use super::*;

pub(super) fn fixture_metadata_for_real_matrix(
    name: &str,
    artifacts: &FixtureArtifacts,
) -> Option<FixtureMetadata> {
    Some(match name {
        "real_amd_hyprland_clean" => real_case(RealCaseInput {
            name,
            gpu_vendor: "AMD",
            gpu_driver: "amdgpu",
            compositor: "Hyprland",
            session_type: "wayland",
            scenario: "clean",
            sanitized_capture_id: "sanitized-amd-hyprland-clean-v1",
            description: "Sanitized AMD/Hyprland clean recording with normal frames and no scheduler spike diagnosis.",
            primary_cause: "Unknown",
            accepted_confidence: &[],
            data_quality: "High",
            evidence_contains: &[],
            artifacts,
        }),
        "real_nvidia_gnome_false_positive" => real_case(RealCaseInput {
            name,
            gpu_vendor: "NVIDIA",
            gpu_driver: "nvidia",
            compositor: "GNOME",
            session_type: "wayland",
            scenario: "false-positive",
            sanitized_capture_id: "sanitized-nvidia-gnome-false-positive-v1",
            description: "Sanitized NVIDIA/GNOME recording with harmless GPU/frame noise that must not become a strong diagnosis.",
            primary_cause: "Unknown",
            accepted_confidence: &[],
            data_quality: "High",
            evidence_contains: &[],
            artifacts,
        }),
        "real_intel_kwin_cpu_bound" => real_case(RealCaseInput {
            name,
            gpu_vendor: "Intel",
            gpu_driver: "i915",
            compositor: "KWin",
            session_type: "wayland",
            scenario: "cpu-bound",
            sanitized_capture_id: "sanitized-intel-kwin-cpu-bound-v1",
            description: "Sanitized Intel/KWin CPU-pressure recording with CPU PSI near scheduler-latency spikes.",
            primary_cause: "CpuPressureCandidate",
            accepted_confidence: &["Medium", "High"],
            data_quality: "High",
            evidence_contains: &["high CPU PSI"],
            artifacts,
        }),
        "real_amd_gamescope_gpu_bound" => {
            let mut metadata = real_case(RealCaseInput {
                name,
                gpu_vendor: "AMD",
                gpu_driver: "amdgpu",
                compositor: "Gamescope",
                session_type: "wayland",
                scenario: "gpu-bound",
                sanitized_capture_id: "sanitized-amd-gamescope-gpu-bound-v1",
                description: "Sanitized AMD/Gamescope GPU-bound recording with high GPU busy near a visible frame spike.",
                primary_cause: "Any",
                accepted_confidence: &[],
                data_quality: "High",
                evidence_contains: &[],
                artifacts,
            });
            metadata.expected.required_candidate = Some("GpuBoundCandidate".to_owned());
            metadata.expected.required_candidate_evidence = vec!["GPU busy".to_owned()];
            metadata
        }
        "real_nvidia_kwin_irq_overlap" => real_case(RealCaseInput {
            name,
            gpu_vendor: "NVIDIA",
            gpu_driver: "nvidia",
            compositor: "KWin",
            session_type: "wayland",
            scenario: "irq",
            sanitized_capture_id: "sanitized-nvidia-kwin-irq-overlap-v1",
            description: "Sanitized NVIDIA/KWin recording with a GPU IRQ handler overlapping a scheduler-latency cluster.",
            primary_cause: "IrqDelayCandidate",
            accepted_confidence: &["Medium", "High"],
            data_quality: "High",
            evidence_contains: &["IRQ"],
            artifacts,
        }),
        "real_intel_sway_compositor_delay" => real_case(RealCaseInput {
            name,
            gpu_vendor: "Intel",
            gpu_driver: "i915",
            compositor: "Sway",
            session_type: "wayland",
            scenario: "compositor",
            sanitized_capture_id: "sanitized-intel-sway-compositor-delay-v1",
            description: "Sanitized Intel/Sway recording with compositor scheduler delay during a visible frame spike.",
            primary_cause: "CompositorSchedulerDelay",
            accepted_confidence: &["Medium", "High"],
            data_quality: "High",
            evidence_contains: &["compositor thread"],
            artifacts,
        }),
        _ => return None,
    })
}

struct RealCaseInput<'a> {
    name: &'a str,
    gpu_vendor: &'a str,
    gpu_driver: &'a str,
    compositor: &'a str,
    session_type: &'a str,
    scenario: &'a str,
    sanitized_capture_id: &'a str,
    description: &'a str,
    primary_cause: &'a str,
    accepted_confidence: &'a [&'a str],
    data_quality: &'a str,
    evidence_contains: &'a [&'a str],
    artifacts: &'a FixtureArtifacts,
}

fn real_case(input: RealCaseInput<'_>) -> FixtureMetadata {
    with_platform(
        fixture_metadata(FixtureMetadataInput {
            name: input.name,
            source: "sanitized-real-recording",
            quality_expectation: input.data_quality,
            description: input.description,
            primary_cause: input.primary_cause,
            accepted_confidence: input.accepted_confidence,
            data_quality: input.data_quality,
            evidence_contains: input.evidence_contains,
            artifacts: exact_artifacts(input.artifacts),
        }),
        real_platform(
            input.gpu_vendor,
            input.gpu_driver,
            input.compositor,
            input.session_type,
            input.scenario,
            input.sanitized_capture_id,
        ),
    )
}
