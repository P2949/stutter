use super::{
    artifact_counts::check_present_loaded_artifact_count,
    drm_quality::check_drm_fence_data_quality, run_artifacts::RunArtifacts,
};
use crate::{artifacts::ArtifactKind, recorder::SESSION_SCHEMA_VERSION};

pub(super) fn check_consistency(artifacts: &mut RunArtifacts) {
    let session = &artifacts.session;
    let validation = &mut artifacts.validation;

    if session
        .core
        .schema_version
        .is_older_than(SESSION_SCHEMA_VERSION)
    {
        validation.warnings.push(format!(
            "session schema version {} is older than current {}",
            session.core.schema_version, SESSION_SCHEMA_VERSION
        ));
    } else if session
        .core
        .schema_version
        .is_newer_than(SESSION_SCHEMA_VERSION)
    {
        validation.errors.push(format!(
            "session schema version {} is newer than current {}",
            session.core.schema_version, SESSION_SCHEMA_VERSION
        ));
    }

    if let Some(metadata) = &artifacts.metadata {
        if metadata
            .core
            .schema_version
            .is_older_than(SESSION_SCHEMA_VERSION)
        {
            validation.warnings.push(format!(
                "metadata schema version {} is older than current {}",
                metadata.core.schema_version, SESSION_SCHEMA_VERSION
            ));
        } else if metadata
            .core
            .schema_version
            .is_newer_than(SESSION_SCHEMA_VERSION)
        {
            validation.errors.push(format!(
                "metadata schema version {} is newer than current {}",
                metadata.core.schema_version, SESSION_SCHEMA_VERSION
            ));
        }

        if metadata.core.spike_events_retained_count != session.core.spike_events_retained_count {
            validation.warnings.push(format!(
                "spike count mismatch: session reported {}, metadata reported {}",
                session.core.spike_events_retained_count, metadata.core.spike_events_retained_count
            ));
        }
    }

    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::Interval,
        artifacts.intervals.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::SpikeEvents,
        artifacts.spikes.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::IrqEvents,
        artifacts.irq_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::GpuSamples,
        artifacts.gpu_samples.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::FrameEvents,
        artifacts.frame_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::MigrationEvents,
        artifacts.migration_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::CpuFreqSamples,
        artifacts.cpu_freq_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::BlockIoEvents,
        artifacts.block_io_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::ScxEvents,
        artifacts.scx_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::RuntimeSlices,
        artifacts.runtime_slices.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::FocusEvents,
        artifacts.focus_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::ForegroundEvents,
        artifacts.foreground_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::KmsFlipEvents,
        artifacts.kms_flip_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::DrmFenceEvents,
        artifacts.drm_fence_events.len(),
    );
    check_present_loaded_artifact_count(
        validation,
        session,
        ArtifactKind::WaylandPresentationEvents,
        artifacts.wayland_presentation_events.len(),
    );
    check_drm_fence_data_quality(artifacts);
}
