use std::path::Path;

use anyhow::Result;

use super::{
    artifact_counts::warn_if_artifact_count_mismatch,
    load_json::{count_ndjson_file, load_json_file},
    paths::{artifact_input_path, file_name_for_path, push_unique_string, run_dir_for},
    required::load_session,
    run_artifacts::RunValidationReport,
};
use crate::{
    artifacts::{
        ArtifactEncoding, ArtifactKind, ArtifactSelection, artifact_file_name, artifact_kinds,
        artifact_path, artifact_primary_and_alias_paths, artifact_spec,
    },
    recorder::{
        BlockIoRecord, CpuFreqRecord, DmaBufEventRecord, DrmFenceEventRecord, FocusEvent,
        ForegroundEvent, FrameEvent, GpuEngineSample, GpuSample, IntervalRecord, IrqEventRecord,
        KmsFlipEventRecord, MetadataFile, MigrationEventRecord, RuntimeSliceRecord,
        SESSION_SCHEMA_VERSION, ScxEvent, SessionFile, SpikeEvent, TreeEvent,
        WaylandPresentationEventRecord,
    },
};

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
                if metadata
                    .core
                    .schema_version
                    .is_older_than(SESSION_SCHEMA_VERSION)
                {
                    report.warnings.push(format!(
                        "metadata schema version {} is older than current {}",
                        metadata.core.schema_version, SESSION_SCHEMA_VERSION
                    ));
                } else if metadata
                    .core
                    .schema_version
                    .is_newer_than(SESSION_SCHEMA_VERSION)
                {
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

    if session
        .core
        .schema_version
        .is_older_than(SESSION_SCHEMA_VERSION)
    {
        report.warnings.push(format!(
            "session schema version {} is older than current {}",
            session.core.schema_version, SESSION_SCHEMA_VERSION
        ));
    } else if session
        .core
        .schema_version
        .is_newer_than(SESSION_SCHEMA_VERSION)
    {
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
        ArtifactKind::DmaBufEvents => count_ndjson_file::<DmaBufEventRecord>(path),
        ArtifactKind::GpuEngineSamples => count_ndjson_file::<GpuEngineSample>(path),
        ArtifactKind::Session | ArtifactKind::Metadata | ArtifactKind::DisplayTopology => {
            anyhow::bail!("artifact {:?} is not an NDJSON stream", kind)
        }
    }
}
