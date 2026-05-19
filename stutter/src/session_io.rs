use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    artifacts::{
        ArtifactCounter, ArtifactEncoding, ArtifactKind, ArtifactSelection, artifact_counter_label,
        artifact_file_name, artifact_kinds, artifact_path, artifact_primary_and_alias_paths,
        artifact_spec,
    },
    recorder::{
        BlockIoRecord, CpuFreqRecord, DrmFenceEventRecord, FocusEvent, ForegroundEvent, FrameEvent,
        GpuSample, IntervalRecord, IrqEventRecord, KmsFlipEventRecord, MetadataFile,
        MigrationEventRecord, RuntimeSliceRecord, SESSION_SCHEMA_VERSION, ScxEvent, SessionFile,
        SpikeEvent, TreeEvent, WaylandPresentationEventRecord,
    },
};

#[derive(Debug, Serialize, Default)]
pub struct RunArtifacts {
    pub run_dir: PathBuf,
    pub session: SessionFile,
    pub metadata: Option<MetadataFile>,

    pub intervals: Vec<IntervalRecord>,
    pub spikes: Vec<SpikeEvent>,
    pub tree_events: Vec<TreeEvent>,
    pub irq_events: Vec<IrqEventRecord>,
    pub gpu_samples: Vec<GpuSample>,
    pub frame_events: Vec<FrameEvent>,
    pub migration_events: Vec<MigrationEventRecord>,
    pub cpu_freq_events: Vec<CpuFreqRecord>,
    pub block_io_events: Vec<BlockIoRecord>,
    pub scx_events: Vec<ScxEvent>,
    pub runtime_slices: Vec<RuntimeSliceRecord>,
    pub focus_events: Vec<FocusEvent>,
    pub foreground_events: Vec<ForegroundEvent>,
    pub kms_flip_events: Vec<KmsFlipEventRecord>,
    pub drm_fence_events: Vec<DrmFenceEventRecord>,
    pub wayland_presentation_events: Vec<WaylandPresentationEventRecord>,

    pub validation: RunValidationReport,
}

#[derive(Debug, Clone, Default)]
pub struct CorrelationWindows {
    pub windows_ms: Vec<(u64, u64)>,
    pub windows_ns: Vec<(u64, u64)>,
}

impl CorrelationWindows {
    pub fn is_in_ms(&self, ms: u64) -> bool {
        self.windows_ms
            .iter()
            .any(|(min, max)| ms >= *min && ms <= *max)
    }

    pub fn is_in_ns(&self, ns: u64) -> bool {
        self.windows_ns
            .iter()
            .any(|(start, end)| ns >= *start && ns <= *end)
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RunValidationReport {
    pub run_dir: PathBuf,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub missing_optional_files: Vec<String>,
    pub present_files: Vec<String>,
}

impl RunValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

fn load_json_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    serde_json::from_reader(file).with_context(|| format!("failed to parse {}", path.display()))
}

fn load_ndjson_file_filtered<T: DeserializeOwned, F: Fn(&T) -> bool>(
    path: &Path,
    filter: F,
) -> Result<Vec<T>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut results = Vec::new();
    let deserializer = serde_json::Deserializer::from_reader(reader);
    let iter = deserializer.into_iter::<T>();
    for item in iter {
        let val = item.with_context(|| format!("failed to parse {}", path.display()))?;
        if filter(&val) {
            results.push(val);
        }
    }
    Ok(results)
}

fn count_ndjson_file<T: DeserializeOwned>(path: &Path) -> Result<usize> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let deserializer = serde_json::Deserializer::from_reader(reader);
    let iter = deserializer.into_iter::<T>();
    let mut count = 0usize;
    for item in iter {
        item.with_context(|| format!("failed to parse {}", path.display()))?;
        count += 1;
    }
    Ok(count)
}

fn run_dir_for(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }
}

fn artifact_input_path(path: &Path, kind: ArtifactKind) -> PathBuf {
    if path.is_dir() {
        artifact_path(path, kind)
    } else {
        path.to_path_buf()
    }
}

fn push_unique_string(values: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    if !values.contains(&value) {
        values.push(value);
    }
}

fn file_name_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

pub struct ArtifactLoader<'a> {
    run_dir: &'a Path,
    validation: &'a mut RunValidationReport,
}

impl<'a> ArtifactLoader<'a> {
    pub fn new(run_dir: &'a Path, validation: &'a mut RunValidationReport) -> Self {
        Self {
            run_dir,
            validation,
        }
    }

    pub fn load_required_json<T: DeserializeOwned>(&mut self, kind: ArtifactKind) -> Result<T> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::JsonObject {
            anyhow::bail!("artifact {:?} is not a JSON object", kind);
        }

        let path = artifact_path(self.run_dir, kind);
        if !path.exists() {
            anyhow::bail!(
                "missing mandatory {} (searched {})",
                artifact_file_name(kind),
                path.display()
            );
        }

        push_unique_string(&mut self.validation.present_files, artifact_file_name(kind));
        load_json_file(&path)
    }

    pub fn load_optional_json<T: DeserializeOwned>(
        &mut self,
        kind: ArtifactKind,
    ) -> Result<Option<T>> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::JsonObject {
            anyhow::bail!("artifact {:?} is not a JSON object", kind);
        }

        let path = artifact_path(self.run_dir, kind);
        if !path.exists() {
            push_unique_string(
                &mut self.validation.missing_optional_files,
                artifact_file_name(kind),
            );
            return Ok(None);
        }

        push_unique_string(&mut self.validation.present_files, artifact_file_name(kind));
        load_json_file(&path).map(Some)
    }

    pub fn load_optional_ndjson<T: DeserializeOwned>(
        &mut self,
        kind: ArtifactKind,
    ) -> Result<Vec<T>> {
        self.load_optional_ndjson_filtered(kind, |_| true)
    }

    pub fn load_optional_ndjson_with_aliases<T: DeserializeOwned>(
        &mut self,
        kind: ArtifactKind,
    ) -> Result<Vec<T>> {
        self.load_optional_ndjson_filtered_with_aliases(kind, |_| true)
    }

    pub fn load_optional_ndjson_filtered<T: DeserializeOwned, F: Fn(&T) -> bool>(
        &mut self,
        kind: ArtifactKind,
        filter: F,
    ) -> Result<Vec<T>> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::Ndjson {
            anyhow::bail!("artifact {:?} is not an NDJSON stream", kind);
        }

        let file_name = artifact_file_name(kind);
        let path = artifact_path(self.run_dir, kind);
        if !path.exists() {
            push_unique_string(&mut self.validation.missing_optional_files, file_name);
            return Ok(Vec::new());
        }

        push_unique_string(&mut self.validation.present_files, file_name);
        load_ndjson_file_filtered(&path, filter)
    }

    pub fn load_optional_ndjson_filtered_with_aliases<T: DeserializeOwned, F: Fn(&T) -> bool>(
        &mut self,
        kind: ArtifactKind,
        filter: F,
    ) -> Result<Vec<T>> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::Ndjson {
            anyhow::bail!("artifact {:?} is not an NDJSON stream", kind);
        }

        for path in artifact_primary_and_alias_paths(self.run_dir, kind) {
            if path.exists() {
                let file_name = file_name_for_path(&path);
                push_unique_string(&mut self.validation.present_files, file_name);
                self.validation
                    .missing_optional_files
                    .retain(|missing| missing != artifact_file_name(kind));
                return load_ndjson_file_filtered(&path, filter);
            }
        }

        push_unique_string(
            &mut self.validation.missing_optional_files,
            artifact_file_name(kind),
        );
        Ok(Vec::new())
    }
}

pub fn load_session(path: &Path) -> Result<SessionFile> {
    let session_path = artifact_input_path(path, ArtifactKind::Session);
    load_json_file(&session_path)
}

pub fn load_metadata(path: &Path) -> Result<Option<MetadataFile>> {
    let metadata_path = artifact_input_path(path, ArtifactKind::Metadata);
    if !metadata_path.exists() {
        return Ok(None);
    }
    load_json_file(&metadata_path).map(Some)
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
                task: s.task,
                active: s.active,
                class: s.class,
                process_pid: s.process_pid,
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
        validation,
    };

    check_consistency(&mut artifacts);

    Ok(artifacts)
}

fn validation_has_present_kind(validation: &RunValidationReport, kind: ArtifactKind) -> bool {
    let spec = artifact_spec(kind);
    validation
        .present_files
        .iter()
        .any(|file| file == spec.file_name || spec.legacy_aliases.iter().any(|alias| file == alias))
}

fn expected_artifact_count_for_counter(
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
    }
}

fn present_name_for_kind(validation: &RunValidationReport, kind: ArtifactKind) -> &'static str {
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

fn push_artifact_count_mismatch_warning(
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

fn warn_if_artifact_count_mismatch(
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

fn check_present_loaded_artifact_count(
    validation: &mut RunValidationReport,
    session: &SessionFile,
    kind: ArtifactKind,
    actual_count: usize,
) {
    if validation_has_present_kind(validation, kind) {
        warn_if_artifact_count_mismatch(validation, session, kind, actual_count);
    }
}

fn check_consistency(artifacts: &mut RunArtifacts) {
    let session = &artifacts.session;
    let validation = &mut artifacts.validation;

    if session.core.schema_version < SESSION_SCHEMA_VERSION {
        validation.warnings.push(format!(
            "session schema version {} is older than current {}",
            session.core.schema_version, SESSION_SCHEMA_VERSION
        ));
    } else if session.core.schema_version > SESSION_SCHEMA_VERSION {
        validation.errors.push(format!(
            "session schema version {} is newer than current {}",
            session.core.schema_version, SESSION_SCHEMA_VERSION
        ));
    }

    if let Some(metadata) = &artifacts.metadata {
        if metadata.core.schema_version < SESSION_SCHEMA_VERSION {
            validation.warnings.push(format!(
                "metadata schema version {} is older than current {}",
                metadata.core.schema_version, SESSION_SCHEMA_VERSION
            ));
        } else if metadata.core.schema_version > SESSION_SCHEMA_VERSION {
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

fn check_drm_fence_data_quality(artifacts: &mut RunArtifacts) {
    if !artifacts.session.config.drm_fence_latency {
        return;
    }

    let validation = &mut artifacts.validation;
    let events = &artifacts.drm_fence_events;
    let artifact_missing = validation
        .missing_optional_files
        .iter()
        .any(|file| file == artifact_file_name(ArtifactKind::DrmFenceEvents));

    if artifact_missing {
        validation.warnings.push(
            "DRM fence latency was requested but drm_fence_events.json is missing; tracepoints may have been unavailable"
                .to_owned(),
        );
    }

    if events.is_empty() {
        validation.warnings.push(
            "DRM fence latency was requested but no fence events were recorded; absence is not proof of no GPU wait"
                .to_owned(),
        );
    } else {
        if events
            .iter()
            .all(|event| event.event_kind != "wait_interval" || event.duration_ns.is_none())
        {
            validation.warnings.push(
                "DRM fence events contain only signal/marker evidence; wait duration attribution is low confidence"
                    .to_owned(),
            );
        }
        if events
            .iter()
            .any(|event| event.correlation_basis == "unknown")
        {
            validation.warnings.push(
                "DRM fence events include records without a stable context/seqno or timeline/seqno key"
                    .to_owned(),
            );
        }
        if events.iter().any(|event| {
            event.source == "unknown" || matches!(event.gpu_role.as_deref(), None | Some("unknown"))
        }) {
            validation.warnings.push(
                "DRM fence driver or GPU-role mapping is incomplete for some events".to_owned(),
            );
        }
    }

    if artifacts
        .session
        .config
        .drm_fence_render_card
        .as_deref()
        .is_none_or(str::is_empty)
        || artifacts
            .session
            .config
            .drm_fence_display_card
            .as_deref()
            .is_none_or(str::is_empty)
    {
        validation.warnings.push(
            "DRM fence render/display cards were not both identified; cross-GPU attribution is approximate"
                .to_owned(),
        );
    }
}

impl RunArtifacts {
    pub fn load_correlations(&mut self, windows: CorrelationWindows) -> Result<()> {
        if windows.windows_ms.is_empty() && windows.windows_ns.is_empty() {
            return Ok(());
        }

        let run_dir = &self.run_dir;
        let validation = &mut self.validation;
        let mut loader = ArtifactLoader::new(run_dir, validation);

        self.intervals = loader
            .load_optional_ndjson_filtered(ArtifactKind::Interval, |r: &IntervalRecord| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.tree_events = loader.load_optional_ndjson(ArtifactKind::TreeEvents)?;

        self.irq_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::IrqEvents,
            |r: &IrqEventRecord| {
                windows
                    .windows_ns
                    .iter()
                    .any(|(start, end)| r.exit_ns >= *start && r.enter_ns <= *end)
            },
        )?;

        self.gpu_samples = loader
            .load_optional_ndjson_filtered(ArtifactKind::GpuSamples, |r: &GpuSample| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.migration_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::MigrationEvents,
            |r: &MigrationEventRecord| windows.is_in_ns(r.timestamp_ns),
        )?;

        self.cpu_freq_events = loader
            .load_optional_ndjson_filtered(ArtifactKind::CpuFreqSamples, |r: &CpuFreqRecord| {
                windows.is_in_ns(r.timestamp_ns)
            })?;

        self.block_io_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::BlockIoEvents,
            |r: &BlockIoRecord| {
                let start_ns = r.timestamp_ns.saturating_sub(r.duration_ns);
                let end_ns = r.timestamp_ns;

                windows
                    .windows_ns
                    .iter()
                    .any(|(start, end)| end_ns >= *start && start_ns <= *end)
            },
        )?;

        self.scx_events = loader
            .load_optional_ndjson_filtered(ArtifactKind::ScxEvents, |r: &ScxEvent| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.runtime_slices = loader.load_optional_ndjson_filtered(
            ArtifactKind::RuntimeSlices,
            |r: &RuntimeSliceRecord| windows.is_in_ms(r.elapsed_ms),
        )?;

        self.focus_events = loader
            .load_optional_ndjson_filtered(ArtifactKind::FocusEvents, |r: &FocusEvent| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.foreground_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::ForegroundEvents,
            |r: &ForegroundEvent| windows.is_in_ms(r.elapsed_ms),
        )?;

        self.kms_flip_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::KmsFlipEvents,
            |r: &KmsFlipEventRecord| {
                if windows.is_in_ns(r.timestamp_ns) {
                    return true;
                }
                if let (Some(start_ns), Some(done_ns)) = (r.request_ns, r.done_ns) {
                    return windows
                        .windows_ns
                        .iter()
                        .any(|(start, end)| done_ns >= *start && start_ns <= *end);
                }
                false
            },
        )?;

        self.drm_fence_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::DrmFenceEvents,
            |r: &DrmFenceEventRecord| {
                if windows.is_in_ns(r.timestamp_ns) {
                    return true;
                }
                if let (Some(start_ns), Some(done_ns)) = (r.wait_start_ns, r.wait_done_ns) {
                    return windows
                        .windows_ns
                        .iter()
                        .any(|(start, end)| done_ns >= *start && start_ns <= *end);
                }
                false
            },
        )?;

        self.wayland_presentation_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::WaylandPresentationEvents,
            |r: &WaylandPresentationEventRecord| {
                if r.presented_ns
                    .is_some_and(|presented| windows.is_in_ns(presented))
                {
                    return true;
                }
                if let (Some(commit_ns), Some(presented_ns)) = (r.commit_ns, r.presented_ns) {
                    return windows
                        .windows_ns
                        .iter()
                        .any(|(start, end)| presented_ns >= *start && commit_ns <= *end);
                }
                windows.is_in_ms(r.elapsed_ms)
            },
        )?;

        Ok(())
    }
}

pub fn validate_run_dir(path: &Path) -> Result<RunValidationReport> {
    let _selection = ArtifactSelection::validate_only();
    let run_dir = run_dir_for(path);

    let mut report = RunValidationReport {
        run_dir: run_dir.clone(),
        ..Default::default()
    };

    validate_metadata_file(&mut report);
    let session = validate_session_file(path, &mut report)?;
    validate_optional_artifacts(&mut report, &session);

    Ok(report)
}

pub fn validate_run_dir_shallow(path: &Path) -> Result<RunValidationReport> {
    let run_dir = run_dir_for(path);

    let mut report = RunValidationReport {
        run_dir: run_dir.clone(),
        ..Default::default()
    };

    validate_metadata_file(&mut report);
    let _session = validate_session_file(path, &mut report)?;

    Ok(report)
}

fn validate_metadata_file(report: &mut RunValidationReport) {
    let path = artifact_path(&report.run_dir, ArtifactKind::Metadata);
    let file_name = artifact_file_name(ArtifactKind::Metadata);

    if path.exists() {
        match load_json_file::<MetadataFile>(&path) {
            Ok(metadata) => {
                push_unique_string(&mut report.present_files, file_name);
                if metadata.core.schema_version < SESSION_SCHEMA_VERSION {
                    report.warnings.push(format!(
                        "metadata schema version {} is older than current {}",
                        metadata.core.schema_version, SESSION_SCHEMA_VERSION
                    ));
                } else if metadata.core.schema_version > SESSION_SCHEMA_VERSION {
                    report.errors.push(format!(
                        "metadata schema version {} is newer than current {}",
                        metadata.core.schema_version, SESSION_SCHEMA_VERSION
                    ));
                }
            }
            Err(e) => {
                report.errors.push(format!("{file_name} invalid: {e:#}"));
            }
        }
    } else {
        push_unique_string(&mut report.missing_optional_files, file_name);
    }
}

fn validate_session_file(path: &Path, report: &mut RunValidationReport) -> Result<SessionFile> {
    let session_path = artifact_input_path(path, ArtifactKind::Session);
    let file_name = artifact_file_name(ArtifactKind::Session);

    if !session_path.exists() {
        report.errors.push(format!(
            "missing mandatory {file_name} (searched {})",
            session_path.display()
        ));
        return Ok(SessionFile::default());
    }

    let session = load_session(path)?;
    push_unique_string(&mut report.present_files, file_name);

    if session.core.schema_version < SESSION_SCHEMA_VERSION {
        report.warnings.push(format!(
            "session schema version {} is older than current {}",
            session.core.schema_version, SESSION_SCHEMA_VERSION
        ));
    } else if session.core.schema_version > SESSION_SCHEMA_VERSION {
        report.errors.push(format!(
            "session schema version {} is newer than current {}",
            session.core.schema_version, SESSION_SCHEMA_VERSION
        ));
    }

    Ok(session)
}

fn validate_optional_artifacts(report: &mut RunValidationReport, session: &SessionFile) {
    for kind in artifact_kinds() {
        if matches!(
            kind,
            ArtifactKind::Session | ArtifactKind::Metadata | ArtifactKind::FrameCorrelation
        ) {
            continue;
        }

        if artifact_spec(kind).encoding != ArtifactEncoding::Ndjson {
            continue;
        }

        match count_optional_artifact(report, kind) {
            Ok(Some(count)) => warn_if_artifact_count_mismatch(report, session, kind, count),
            Ok(None) => {}
            Err(err) => {
                report
                    .errors
                    .push(format!("{} invalid: {err:#}", artifact_file_name(kind)));
            }
        }
    }
}

fn count_optional_artifact(
    report: &mut RunValidationReport,
    kind: ArtifactKind,
) -> Result<Option<usize>> {
    for path in artifact_primary_and_alias_paths(&report.run_dir, kind) {
        if path.exists() {
            let file_name = file_name_for_path(&path);
            let count = count_artifact_kind(kind, &path)?;
            push_unique_string(&mut report.present_files, file_name);
            report
                .missing_optional_files
                .retain(|missing| missing != artifact_file_name(kind));
            return Ok(Some(count));
        }
    }

    push_unique_string(&mut report.missing_optional_files, artifact_file_name(kind));
    Ok(None)
}

fn count_artifact_kind(kind: ArtifactKind, path: &Path) -> Result<usize> {
    match kind {
        ArtifactKind::Interval => count_ndjson_file::<IntervalRecord>(path),
        ArtifactKind::SpikeEvents => count_ndjson_file::<SpikeEvent>(path),
        ArtifactKind::TreeEvents => count_ndjson_file::<TreeEvent>(path),
        ArtifactKind::IrqEvents => count_ndjson_file::<IrqEventRecord>(path),
        ArtifactKind::GpuSamples => count_ndjson_file::<GpuSample>(path),
        ArtifactKind::FrameEvents | ArtifactKind::FrameCorrelation => {
            count_ndjson_file::<FrameEvent>(path)
        }
        ArtifactKind::MigrationEvents => count_ndjson_file::<MigrationEventRecord>(path),
        ArtifactKind::CpuFreqSamples => count_ndjson_file::<CpuFreqRecord>(path),
        ArtifactKind::BlockIoEvents => count_ndjson_file::<BlockIoRecord>(path),
        ArtifactKind::ScxEvents => count_ndjson_file::<ScxEvent>(path),
        ArtifactKind::RuntimeSlices => count_ndjson_file::<RuntimeSliceRecord>(path),
        ArtifactKind::FocusEvents => count_ndjson_file::<FocusEvent>(path),
        ArtifactKind::ForegroundEvents => count_ndjson_file::<ForegroundEvent>(path),
        ArtifactKind::KmsFlipEvents => count_ndjson_file::<KmsFlipEventRecord>(path),
        ArtifactKind::DrmFenceEvents => count_ndjson_file::<DrmFenceEventRecord>(path),
        ArtifactKind::WaylandPresentationEvents => {
            count_ndjson_file::<WaylandPresentationEventRecord>(path)
        }
        ArtifactKind::Session | ArtifactKind::Metadata => {
            anyhow::bail!("artifact {:?} is not an NDJSON stream", kind)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drm_fence_requested_without_stable_waits_adds_validation_warnings() {
        let mut artifacts = RunArtifacts::default();
        artifacts.session.config.drm_fence_latency = true;
        artifacts.session.config.drm_fence_render_card = Some("card1".to_owned());
        artifacts.session.config.drm_fence_display_card = Some("card0".to_owned());
        artifacts.drm_fence_events.push(DrmFenceEventRecord {
            source: "amdgpu".to_owned(),
            event_kind: "signal".to_owned(),
            gpu_role: Some("render".to_owned()),
            correlation_basis: "unknown".to_owned(),
            ..Default::default()
        });

        check_drm_fence_data_quality(&mut artifacts);

        assert!(
            artifacts
                .validation
                .warnings
                .iter()
                .any(|warning| { warning.contains("only signal/marker evidence") })
        );
        assert!(
            artifacts
                .validation
                .warnings
                .iter()
                .any(|warning| { warning.contains("without a stable context/seqno") })
        );
    }

    #[test]
    fn drm_fence_requested_without_card_mapping_adds_warning() {
        let mut artifacts = RunArtifacts::default();
        artifacts.session.config.drm_fence_latency = true;

        check_drm_fence_data_quality(&mut artifacts);

        assert!(
            artifacts.validation.warnings.iter().any(|warning| {
                warning.contains("render/display cards were not both identified")
            })
        );
    }
}
