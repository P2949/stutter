use super::run_artifacts::RunValidationReport;
use crate::{
    artifacts::{ArtifactCounter, ArtifactKind, artifact_counter_label, artifact_spec},
    recorder::SessionFile,
};

pub(super) fn validation_has_present_kind(
    validation: &RunValidationReport,
    kind: ArtifactKind,
) -> bool {
    let spec = artifact_spec(kind);
    validation
        .present_files
        .iter()
        .any(|file| file == spec.file_name || spec.legacy_aliases.iter().any(|alias| file == alias))
}

pub(super) fn expected_artifact_count_for_counter(
    session: &SessionFile,
    counter: ArtifactCounter,
) -> Option<u64> {
    match counter {
        ArtifactCounter::IntervalRecord => Some(session.core.interval_record_count),
        ArtifactCounter::SpikeEventsRetained => Some(session.core.spike_events_retained_count),
        ArtifactCounter::IrqEvent => Some(session.core.irq_event_count),
        ArtifactCounter::GpuSample => Some(session.core.gpu_sample_count),
        ArtifactCounter::FrameEvent => Some(session.core.frame_event_count),
        ArtifactCounter::BlockIoEvent => Some(session.core.block_io_event_count),
        ArtifactCounter::RuntimeSlice => Some(session.core.runtime_slice_count),
        ArtifactCounter::FocusEvent => Some(session.core.focus_event_count),
        ArtifactCounter::ForegroundEvent => Some(session.core.foreground_event_count),
        ArtifactCounter::MigrationEvent => session.core.migration_event_count,
        ArtifactCounter::CpuFreqSample => session.core.cpu_freq_sample_count,
        ArtifactCounter::ScxEvent => Some(session.core.scx_event_count),
        ArtifactCounter::KmsFlipEvent => Some(session.core.kms_flip_event_count),
        ArtifactCounter::DrmFenceEvent => Some(session.core.drm_fence_event_count),
        ArtifactCounter::WaylandPresentationEvent => {
            Some(session.core.wayland_presentation_event_count)
        }
        ArtifactCounter::DmaBufEvent => Some(session.core.dmabuf_event_count),
        ArtifactCounter::GpuEngineSample => Some(session.core.gpu_engine_sample_count),
    }
}

pub(super) fn present_name_for_kind(
    validation: &RunValidationReport,
    kind: ArtifactKind,
) -> &'static str {
    let spec = artifact_spec(kind);
    for file in &validation.present_files {
        if file == spec.file_name {
            return spec.file_name;
        }
        for alias in spec.legacy_aliases {
            if file == alias {
                return alias;
            }
        }
    }
    spec.file_name
}

pub(super) fn push_artifact_count_mismatch_warning(
    validation: &mut RunValidationReport,
    kind: ArtifactKind,
    expected_count: u64,
    actual_count: usize,
) {
    let Some(counter) = artifact_spec(kind).counter_field else {
        return;
    };

    let file_name = present_name_for_kind(validation, kind);
    if actual_count as u64 != expected_count {
        validation.warnings.push(format!(
            "{} count mismatch: session reported {}, found {} in {}",
            artifact_counter_label(counter),
            expected_count,
            actual_count,
            file_name
        ));
    }
}

pub(super) fn warn_if_artifact_count_mismatch(
    validation: &mut RunValidationReport,
    session: &SessionFile,
    kind: ArtifactKind,
    actual_count: usize,
) {
    let Some(counter) = artifact_spec(kind).counter_field else {
        return;
    };
    if let Some(expected_count) = expected_artifact_count_for_counter(session, counter) {
        push_artifact_count_mismatch_warning(validation, kind, expected_count, actual_count);
    }
}

pub(super) fn check_present_loaded_artifact_count(
    validation: &mut RunValidationReport,
    session: &SessionFile,
    kind: ArtifactKind,
    actual_count: usize,
) {
    if validation_has_present_kind(validation, kind) {
        warn_if_artifact_count_mismatch(validation, session, kind, actual_count);
    }
}
