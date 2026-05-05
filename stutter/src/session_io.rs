use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};

use crate::recorder::{
    BlockIoRecord, CpuFreqRecord, FrameEvent, GpuSample, IntervalRecord, IrqEventRecord,
    MetadataFile, MigrationEventRecord, SESSION_SCHEMA_VERSION, ScxEvent, SessionFile, SpikeEvent,
    TreeEvent,
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

    pub validation: RunValidationReport,
}

#[derive(Debug, Clone, Copy)]
pub struct ArtifactLoadOptions {
    pub load_intervals: bool,
    pub load_spikes: bool,
    pub load_tree_events: bool,
    pub load_irq_events: bool,
    pub load_gpu_samples: bool,
    pub load_frame_events: bool,
    pub load_migration_events: bool,
    pub load_cpu_freq_events: bool,
    pub load_block_io_events: bool,
    pub load_scx_events: bool,
}

impl ArtifactLoadOptions {
    pub const REPORT: Self = Self {
        load_intervals: true,
        load_spikes: true,
        load_tree_events: true,
        load_irq_events: true,
        load_gpu_samples: true,
        load_frame_events: true,
        load_migration_events: true,
        load_cpu_freq_events: true,
        load_block_io_events: true,
        load_scx_events: true,
    };

    pub const TUNE: Self = Self {
        load_intervals: true,
        load_spikes: false,
        load_tree_events: false,
        load_irq_events: false,
        load_gpu_samples: false,
        load_frame_events: true,
        load_migration_events: false,
        load_cpu_freq_events: false,
        load_block_io_events: false,
        load_scx_events: false,
    };

    pub const VALIDATE_ONLY: Self = Self {
        load_intervals: false,
        load_spikes: false,
        load_tree_events: false,
        load_irq_events: false,
        load_gpu_samples: false,
        load_frame_events: false,
        load_migration_events: false,
        load_cpu_freq_events: false,
        load_block_io_events: false,
        load_scx_events: false,
    };
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

const SESSION_FILE: &str = "session.json";
const METADATA_FILE: &str = "metadata.json";
const INTERVALS_FILE: &str = "interval.json";
const SPIKES_FILE: &str = "spike_events.json";
const TREE_EVENTS_FILE: &str = "tree_events.json";
const IRQ_EVENTS_FILE: &str = "irq_events.json";
const GPU_SAMPLES_FILE: &str = "gpu_samples.json";
const FRAME_EVENTS_FILE: &str = "frame_correlation.json";
const MIGRATION_EVENTS_FILE: &str = "migration_events.json";
const CPU_FREQ_EVENTS_FILE: &str = "cpu_freq_samples.json";
const BLOCK_IO_EVENTS_FILE: &str = "io_events.json";
const SCX_EVENTS_FILE: &str = "scx_events.json";

fn load_json_file<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    serde_json::from_reader(file).with_context(|| format!("failed to parse {}", path.display()))
}

fn load_ndjson_file<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut results = Vec::new();
    let deserializer = serde_json::Deserializer::from_reader(reader);
    let iter = deserializer.into_iter::<T>();
    for item in iter {
        results.push(item.with_context(|| format!("failed to parse {}", path.display()))?);
    }
    Ok(results)
}

fn session_path_for(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(SESSION_FILE)
    } else {
        path.to_path_buf()
    }
}

fn metadata_path_for(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(METADATA_FILE)
    } else {
        path.to_path_buf()
    }
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

fn json_path_for(run_dir: &Path, file_name: &str) -> PathBuf {
    run_dir.join(file_name)
}

fn load_optional_json_vec<T: DeserializeOwned>(
    run_dir: &Path,
    file_name: &str,
    validation: &mut RunValidationReport,
) -> Result<Vec<T>> {
    let path = json_path_for(run_dir, file_name);

    if !path.exists() {
        validation.missing_optional_files.push(file_name.to_owned());
        return Ok(Vec::new());
    }

    validation.present_files.push(file_name.to_owned());
    load_ndjson_file(&path)
}

pub fn load_session(path: &Path) -> Result<SessionFile> {
    let session_path = session_path_for(path);
    load_json_file(&session_path)
}

pub fn load_metadata(path: &Path) -> Result<Option<MetadataFile>> {
    let metadata_path = metadata_path_for(path);
    if !metadata_path.exists() {
        return Ok(None);
    }
    load_json_file(&metadata_path).map(Some)
}

pub fn load_run_artifacts(path: &Path, options: ArtifactLoadOptions) -> Result<RunArtifacts> {
    let run_dir = run_dir_for(path);
    let session = load_session(path)?;
    let metadata = load_metadata(&run_dir)?;

    let mut validation = RunValidationReport {
        run_dir: run_dir.clone(),
        ..Default::default()
    };

    validation.present_files.push(SESSION_FILE.to_owned());
    if metadata.is_some() {
        validation.present_files.push(METADATA_FILE.to_owned());
    } else {
        validation
            .missing_optional_files
            .push(METADATA_FILE.to_owned());
    }

    let intervals = if options.load_intervals {
        load_optional_json_vec(&run_dir, INTERVALS_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let spikes = if options.load_spikes {
        load_optional_json_vec(&run_dir, SPIKES_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let tree_events = if options.load_tree_events {
        load_optional_json_vec(&run_dir, TREE_EVENTS_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let irq_events = if options.load_irq_events {
        load_optional_json_vec(&run_dir, IRQ_EVENTS_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let gpu_samples = if options.load_gpu_samples {
        load_optional_json_vec(&run_dir, GPU_SAMPLES_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let frame_events = if options.load_frame_events {
        load_optional_json_vec(&run_dir, FRAME_EVENTS_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let migration_events = if options.load_migration_events {
        load_optional_json_vec(&run_dir, MIGRATION_EVENTS_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let cpu_freq_events = if options.load_cpu_freq_events {
        load_optional_json_vec(&run_dir, CPU_FREQ_EVENTS_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let block_io_events = if options.load_block_io_events {
        load_optional_json_vec(&run_dir, BLOCK_IO_EVENTS_FILE, &mut validation)?
    } else {
        Vec::new()
    };

    let scx_events = if options.load_scx_events {
        load_optional_json_vec(&run_dir, SCX_EVENTS_FILE, &mut validation)?
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
        validation,
    };

    check_consistency(&mut artifacts);

    Ok(artifacts)
}

fn check_consistency(artifacts: &mut RunArtifacts) {
    let session = &artifacts.session;
    let validation = &mut artifacts.validation;

    // Schema validation
    if session.schema_version < SESSION_SCHEMA_VERSION {
        validation.warnings.push(format!(
            "session schema version {} is older than current {}",
            session.schema_version, SESSION_SCHEMA_VERSION
        ));
    } else if session.schema_version > SESSION_SCHEMA_VERSION {
        validation.errors.push(format!(
            "session schema version {} is newer than current {}",
            session.schema_version, SESSION_SCHEMA_VERSION
        ));
    }

    if let Some(metadata) = &artifacts.metadata {
        if metadata.schema_version < SESSION_SCHEMA_VERSION {
            validation.warnings.push(format!(
                "metadata schema version {} is older than current {}",
                metadata.schema_version, SESSION_SCHEMA_VERSION
            ));
        } else if metadata.schema_version > SESSION_SCHEMA_VERSION {
            validation.errors.push(format!(
                "metadata schema version {} is newer than current {}",
                metadata.schema_version, SESSION_SCHEMA_VERSION
            ));
        }

        if metadata.spike_events_retained_count != session.spike_events_retained_count {
            validation.warnings.push(format!(
                "spike count mismatch: session reported {}, metadata reported {}",
                session.spike_events_retained_count, metadata.spike_events_retained_count
            ));
        }
    }

    // Spike count consistency
    if validation.present_files.contains(&SPIKES_FILE.to_owned())
        && artifacts.spikes.len() as u64 != session.spike_events_retained_count
    {
        validation.warnings.push(format!(
            "spike count mismatch: session reported {}, found {} in artifact",
            session.spike_events_retained_count,
            artifacts.spikes.len()
        ));
    }
}

pub fn validate_run_dir(path: &Path) -> Result<RunValidationReport> {
    let _options = ArtifactLoadOptions::VALIDATE_ONLY;
    let run_dir = run_dir_for(path);

    let mut report = RunValidationReport {
        run_dir: run_dir.clone(),
        ..Default::default()
    };

    let metadata_path = run_dir.join(METADATA_FILE);
    if metadata_path.exists() {
        match load_json_file::<MetadataFile>(&metadata_path) {
            Ok(metadata) => {
                report.present_files.push(METADATA_FILE.to_owned());
                if metadata.schema_version < SESSION_SCHEMA_VERSION {
                    report.warnings.push(format!(
                        "metadata schema version {} is older than current {}",
                        metadata.schema_version, SESSION_SCHEMA_VERSION
                    ));
                } else if metadata.schema_version > SESSION_SCHEMA_VERSION {
                    report.errors.push(format!(
                        "metadata schema version {} is newer than current {}",
                        metadata.schema_version, SESSION_SCHEMA_VERSION
                    ));
                }
            }
            Err(e) => {
                report
                    .errors
                    .push(format!("{METADATA_FILE} invalid: {e:#}"));
            }
        }
    } else {
        report.missing_optional_files.push(METADATA_FILE.to_owned());
    }

    let session_path = session_path_for(path);
    if !session_path.exists() {
        report.errors.push(format!(
            "missing mandatory {SESSION_FILE} (searched {})",
            session_path.display()
        ));
        return Ok(report);
    }

    let session = load_session(path)?;
    report.present_files.push(SESSION_FILE.to_owned());

    if session.schema_version < SESSION_SCHEMA_VERSION {
        report.warnings.push(format!(
            "session schema version {} is older than current {}",
            session.schema_version, SESSION_SCHEMA_VERSION
        ));
    } else if session.schema_version > SESSION_SCHEMA_VERSION {
        report.errors.push(format!(
            "session schema version {} is newer than current {}",
            session.schema_version, SESSION_SCHEMA_VERSION
        ));
    }

    let optional_artifacts = [
        (INTERVALS_FILE, "IntervalRecord"),
        (SPIKES_FILE, "SpikeEvent"),
        (TREE_EVENTS_FILE, "TreeEvent"),
        (IRQ_EVENTS_FILE, "IrqEventRecord"),
        (GPU_SAMPLES_FILE, "GpuSample"),
        (FRAME_EVENTS_FILE, "FrameEvent"),
        (MIGRATION_EVENTS_FILE, "MigrationEventRecord"),
        (CPU_FREQ_EVENTS_FILE, "CpuFreqRecord"),
        (BLOCK_IO_EVENTS_FILE, "BlockIoRecord"),
        (SCX_EVENTS_FILE, "ScxEvent"),
    ];

    for (file_name, type_name) in optional_artifacts {
        let path = report.run_dir.join(file_name);
        if path.exists() {
            let res = match type_name {
                "IntervalRecord" => load_ndjson_file::<IntervalRecord>(&path).map(|_| ()),
                "SpikeEvent" => load_ndjson_file::<SpikeEvent>(&path).map(|_| ()),
                "TreeEvent" => load_ndjson_file::<TreeEvent>(&path).map(|_| ()),
                "IrqEventRecord" => load_ndjson_file::<IrqEventRecord>(&path).map(|_| ()),
                "GpuSample" => load_ndjson_file::<GpuSample>(&path).map(|_| ()),
                "FrameEvent" => load_ndjson_file::<FrameEvent>(&path).map(|_| ()),
                "MigrationEventRecord" => {
                    load_ndjson_file::<MigrationEventRecord>(&path).map(|_| ())
                }
                "CpuFreqRecord" => load_ndjson_file::<CpuFreqRecord>(&path).map(|_| ()),
                "BlockIoRecord" => load_ndjson_file::<BlockIoRecord>(&path).map(|_| ()),
                "ScxEvent" => load_ndjson_file::<ScxEvent>(&path).map(|_| ()),
                _ => unreachable!(),
            };

            match res {
                Ok(_) => {
                    report.present_files.push(file_name.to_owned());
                }
                Err(e) => {
                    report.errors.push(format!("{file_name} invalid: {e:#}"));
                }
            }
        } else {
            report.missing_optional_files.push(file_name.to_owned());
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        ebpf_loader::DropCountersSnapshot,
        metadata::SystemMetadata,
        recorder::{RecordedConfig, RecordedTime},
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-test-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_minimal_session(dir: &Path) {
        let session = SessionFile {
            schema_version: crate::recorder::SESSION_SCHEMA_VERSION,
            run_name: None,
            started_at: RecordedTime {
                unix_seconds: 0,
                unix_nanos: 0,
                system_time_debug: "".into(),
            },
            ended_at: RecordedTime {
                unix_seconds: 1,
                unix_nanos: 0,
                system_time_debug: "".into(),
            },
            monotonic_start_ns: None,
            monotonic_end_ns: None,
            duration_ms: 1000,
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
            stop_reason: "test".into(),
            config: RecordedConfig {
                manual_pids: vec![],
                tree_roots: vec![],
                cgroupv2: None,
                exclude_tree_pids: vec![],
                include_comm: vec![],
                exclude_comm: vec![],
                watch_process: None,
                persistent: false,
                keep_missing_pid: false,
                watch_poll_ms: 1000,
                watch_timeout_ms: None,
                csv_path: None,
                irq_latency: false,
                irqs: vec![],
                hwmon: false,
                hwmon_root: None,
                hwmon_drm_card: None,
                hwmon_render_node: None,
                mangohud_log: None,
                tui: false,
                summary_period_ms: 1000,
                epoch_period_ms: None,
                retain_intervals: None,
                max_tasks: 1024,
                spike_threshold_ns: 1000000,
                alert_threshold_ns: None,
                alert_webhook_url: None,
                follow_exec: true,
                verbose: false,
                faults: false,
                block_io: false,
                stat_wait: false,
            },
            metadata: SystemMetadata::default(),
            target_pids_max: 1024,
            active_target_pids_count: 0,
            active_expanded_tasks: vec![],
            interval_record_count: 0,
            intervals_dropped: 0,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            spike_events_truncated: false,
            scx_event_count: 0,
            irq_event_count: 0,
            migration_event_count: None,
            cpu_freq_sample_count: None,
            gpu_sample_count: 0,
            frame_event_count: 0,
            block_io_event_count: 0,
            event_stream_write_errors: 0,
            alert_events_dropped_count: 0,
            alert_channel_closed_count: 0,
            first_event_stream_write_error: None,
            block_io_correlation_basis: "dev+sector".into(),
            drop_counters: DropCountersSnapshot::default(),
            tasks: vec![],
            top_spikes: vec![],
        };
        fs::write(
            dir.join(SESSION_FILE),
            serde_json::to_string(&session).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn load_metadata_missing_is_ok() {
        let dir = temp_dir("missing-metadata");
        let result = load_metadata(&dir).unwrap();
        assert!(result.is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn validate_run_dir_reports_missing_session() {
        let dir = temp_dir("missing-session");
        let report = validate_run_dir(&dir).unwrap();
        assert!(!report.is_ok());
        assert!(
            report.errors[0]
                .to_lowercase()
                .contains("missing mandatory session.json")
        );
        assert!(
            report
                .missing_optional_files
                .contains(&METADATA_FILE.to_owned())
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_run_artifacts_missing_optional_files_warns() {
        let dir = temp_dir("missing-optional");
        write_minimal_session(&dir);
        let artifacts = load_run_artifacts(&dir, ArtifactLoadOptions::REPORT).unwrap();
        assert!(
            artifacts
                .validation
                .missing_optional_files
                .contains(&INTERVALS_FILE.to_owned())
        );
        assert!(artifacts.intervals.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_run_artifacts_fails_on_invalid_present_optional_json() {
        let dir = temp_dir("invalid-optional");
        write_minimal_session(&dir);
        fs::write(dir.join(INTERVALS_FILE), "invalid json").unwrap();
        let result = load_run_artifacts(&dir, ArtifactLoadOptions::REPORT);
        assert!(result.is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn load_session_accepts_run_dir() {
        let dir = temp_dir("session-run-dir");
        write_minimal_session(&dir);
        let result = load_session(&dir);
        assert!(result.is_ok());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn validate_run_dir_warns_on_missing_optional_artifacts() {
        let dir = temp_dir("missing-optional-artifacts");
        write_minimal_session(&dir);
        let report = validate_run_dir(&dir).unwrap();
        assert!(report.is_ok());
        assert!(
            report
                .missing_optional_files
                .contains(&INTERVALS_FILE.to_owned())
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_validate_run_dir_ndjson() {
        let dir = temp_dir("validate-ndjson");
        write_minimal_session(&dir);

        let interval_path = dir.join(INTERVALS_FILE);
        let record = IntervalRecord {
            elapsed_ms: 100,
            samples: 1,
            ..Default::default()
        };
        let line1 = serde_json::to_string(&record).unwrap();
        let line2 = serde_json::to_string(&record).unwrap();
        fs::write(&interval_path, format!("{}\n{}\n", line1, line2)).unwrap();

        let report = validate_run_dir(&dir).unwrap();
        assert!(report.is_ok());
        assert!(report.present_files.contains(&INTERVALS_FILE.to_owned()));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_artifact_load_options_validate_only() {
        let dir = temp_dir("validate-only");
        write_minimal_session(&dir);
        let artifacts = load_run_artifacts(&dir, ArtifactLoadOptions::VALIDATE_ONLY).unwrap();
        assert!(artifacts.intervals.is_empty());
        assert!(artifacts.spikes.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_load_run_artifacts_accepts_direct_session_json_path() {
        let dir = temp_dir("direct-session-path");
        write_minimal_session(&dir);
        let session_path = dir.join("session.json");
        let result = load_run_artifacts(&session_path, ArtifactLoadOptions::REPORT);
        assert!(result.is_ok());
        let artifacts = result.unwrap();
        // Since we passed session.json directly, metadata should be found in parent dir
        // Wait, load_metadata(&run_dir) will find it if it's there.
        // But write_minimal_session only writes session.json.
        assert!(artifacts.metadata.is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
