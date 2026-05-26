use std::path::Path;

use anyhow::Result;

use super::{
    consistency::check_consistency,
    paths::{push_unique_string, run_dir_for},
    required::{load_metadata, load_session},
    run_artifacts::{RunArtifacts, RunValidationReport},
};
use crate::{
    artifacts::{ArtifactKind, ArtifactSelection, artifact_file_name},
    display_topology::DisplayTopologySnapshot,
    recorder::SpikeEvent,
};

pub struct ArtifactLoader<'a> {
    pub(super) run_dir: &'a Path,
    pub(super) validation: &'a mut RunValidationReport,
}

impl<'a> ArtifactLoader<'a> {
    pub fn new(run_dir: &'a Path, validation: &'a mut RunValidationReport) -> Self {
        Self {
            run_dir,
            validation,
        }
    }
}

pub fn load_run_artifacts(path: &Path, selection: ArtifactSelection) -> Result<RunArtifacts> {
    let run_dir = run_dir_for(path);
    let mut validation = RunValidationReport {
        run_dir: run_dir.clone(),
        ..Default::default()
    };

    let session = load_session(path)?;
    push_unique_string(
        &mut validation.present_files,
        artifact_file_name(ArtifactKind::Session),
    );

    let metadata = load_metadata(&run_dir)?;
    if metadata.is_some() {
        push_unique_string(
            &mut validation.present_files,
            artifact_file_name(ArtifactKind::Metadata),
        );
    } else {
        push_unique_string(
            &mut validation.missing_optional_files,
            artifact_file_name(ArtifactKind::Metadata),
        );
    }

    let mut loader = ArtifactLoader::new(&run_dir, &mut validation);

    let intervals = if selection.contains(ArtifactKind::Interval) {
        loader.load_optional_ndjson(ArtifactKind::Interval)?
    } else {
        Vec::new()
    };

    let mut spikes = if selection.contains(ArtifactKind::SpikeEvents) {
        loader.load_optional_ndjson(ArtifactKind::SpikeEvents)?
    } else {
        Vec::new()
    };

    if selection.contains(ArtifactKind::SpikeEvents)
        && spikes.is_empty()
        && !session.top_spikes.is_empty()
    {
        spikes = session
            .top_spikes
            .iter()
            .map(|s| SpikeEvent {
                elapsed_ms: None,
                task: s.task.into(),
                active: s.active,
                class: s.class,
                process_pid: s.process_pid.map(stutter_core::ids::Pid::from),
                process_comm: s.process_comm.clone(),
                comm: s.comm.clone(),
                cpu: s.cpu,
                wakeup_target_cpu: s.wakeup_target_cpu,
                prio: s.prio,
                latency_ns: s.latency_ns,
                wakeup_ns: s.wakeup_ns,
                switch_ns: s.switch_ns,
                ..Default::default()
            })
            .collect();
    }

    let tree_events = if selection.contains(ArtifactKind::TreeEvents) {
        loader.load_optional_ndjson(ArtifactKind::TreeEvents)?
    } else {
        Vec::new()
    };

    let irq_events = if selection.contains(ArtifactKind::IrqEvents) {
        loader.load_optional_ndjson(ArtifactKind::IrqEvents)?
    } else {
        Vec::new()
    };

    let gpu_samples = if selection.contains(ArtifactKind::GpuSamples) {
        loader.load_optional_ndjson(ArtifactKind::GpuSamples)?
    } else {
        Vec::new()
    };

    let frame_events = if selection.contains(ArtifactKind::FrameEvents) {
        loader.load_optional_ndjson_with_aliases(ArtifactKind::FrameEvents)?
    } else {
        Vec::new()
    };

    let migration_events = if selection.contains(ArtifactKind::MigrationEvents) {
        loader.load_optional_ndjson(ArtifactKind::MigrationEvents)?
    } else {
        Vec::new()
    };

    let cpu_freq_events = if selection.contains(ArtifactKind::CpuFreqSamples) {
        loader.load_optional_ndjson(ArtifactKind::CpuFreqSamples)?
    } else {
        Vec::new()
    };

    let block_io_events = if selection.contains(ArtifactKind::BlockIoEvents) {
        loader.load_optional_ndjson(ArtifactKind::BlockIoEvents)?
    } else {
        Vec::new()
    };

    let scx_events = if selection.contains(ArtifactKind::ScxEvents) {
        loader.load_optional_ndjson(ArtifactKind::ScxEvents)?
    } else {
        Vec::new()
    };

    let runtime_slices = if selection.contains(ArtifactKind::RuntimeSlices) {
        loader.load_optional_ndjson(ArtifactKind::RuntimeSlices)?
    } else {
        Vec::new()
    };

    let focus_events = if selection.contains(ArtifactKind::FocusEvents) {
        loader.load_optional_ndjson(ArtifactKind::FocusEvents)?
    } else {
        Vec::new()
    };

    let foreground_events = if selection.contains(ArtifactKind::ForegroundEvents) {
        loader.load_optional_ndjson(ArtifactKind::ForegroundEvents)?
    } else {
        Vec::new()
    };

    let kms_flip_events = if selection.contains(ArtifactKind::KmsFlipEvents) {
        loader.load_optional_ndjson(ArtifactKind::KmsFlipEvents)?
    } else {
        Vec::new()
    };

    let drm_fence_events = if selection.contains(ArtifactKind::DrmFenceEvents) {
        loader.load_optional_ndjson(ArtifactKind::DrmFenceEvents)?
    } else {
        Vec::new()
    };

    let wayland_presentation_events = if selection.contains(ArtifactKind::WaylandPresentationEvents)
    {
        loader.load_optional_ndjson(ArtifactKind::WaylandPresentationEvents)?
    } else {
        Vec::new()
    };

    let display_topology = if selection.contains(ArtifactKind::DisplayTopology) {
        loader.load_optional_json::<DisplayTopologySnapshot>(ArtifactKind::DisplayTopology)?
    } else {
        None
    };

    let dmabuf_events = if selection.contains(ArtifactKind::DmaBufEvents) {
        loader.load_optional_ndjson(ArtifactKind::DmaBufEvents)?
    } else {
        Vec::new()
    };

    let gpu_engine_samples = if selection.contains(ArtifactKind::GpuEngineSamples) {
        loader.load_optional_ndjson(ArtifactKind::GpuEngineSamples)?
    } else {
        Vec::new()
    };

    let mut artifacts = RunArtifacts {
        run_dir,
        session,
        metadata,
        intervals,
        spikes,
        tree_events,
        irq_events,
        gpu_samples,
        frame_events,
        migration_events,
        cpu_freq_events,
        block_io_events,
        scx_events,
        runtime_slices,
        focus_events,
        foreground_events,
        kms_flip_events,
        drm_fence_events,
        wayland_presentation_events,
        display_topology,
        dmabuf_events,
        gpu_engine_samples,
        validation,
    };

    check_consistency(&mut artifacts);

    Ok(artifacts)
}
