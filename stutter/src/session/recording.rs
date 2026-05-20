//! Recording runtime setup for monitor sessions.

use std::{fs, path::Path};

use anyhow::Context;
use log::info;

use crate::{
    artifacts::{ArtifactKind, artifact_path},
    config::{CsvStreamTarget, model::MonitorConfig},
    display_topology,
    recorder::{self, LiveRecorder, SpikeEventBuffer},
    session::SessionProbePlan,
};

pub(crate) struct RecordingRuntime;

impl RecordingRuntime {
    pub(crate) fn begin(
        config: &MonitorConfig,
        probe_plan: &SessionProbePlan,
    ) -> anyhow::Result<LiveRecorder> {
        let recording = recorder::prepare_recording(config)?;
        let mut recorder = LiveRecorder {
            run: recording,
            ..Default::default()
        };

        if config.streams.json_stream {
            recorder.enable_stdout_spike_stream();
        }

        recorder.buffers.spike_events = recorder.run.as_ref().map(|_| SpikeEventBuffer::default());

        if let Some(run) = recorder.run.as_mut() {
            if let Some(path) = &config.mangohud.log
                && let Ok(meta) = fs::metadata(path)
            {
                run.mangohud_start_offset = Some(meta.len());
                info!(
                    "mangohud_alignment_init path={} start_offset={}",
                    path.display(),
                    meta.len()
                );
            }

            let dir = &run.run_dir;
            write_display_topology_artifact(dir, run.started_instant.elapsed().as_millis() as u64)?;

            let registry = &mut recorder.streams;

            for kind in probe_plan
                .loaded
                .activation_plan
                .required_stream_artifacts()
            {
                registry.create_stream(dir, kind)?;
            }
        }

        if let Some(csv_stream) = &config.streams.csv {
            recorder.csv_writer = Some(match csv_stream {
                CsvStreamTarget::File(path) => {
                    recorder::IntervalCsvWriter::create_file(path.clone())?
                }
                CsvStreamTarget::Stdout => recorder::IntervalCsvWriter::stdout(),
            });
        }

        Ok(recorder)
    }
}

fn write_display_topology_artifact(run_dir: &Path, elapsed_ms: u64) -> anyhow::Result<()> {
    let mut snapshot = display_topology::probe_display_topology();
    snapshot.collected_at_elapsed_ms = Some(elapsed_ms);

    let path = artifact_path(run_dir, ArtifactKind::DisplayTopology);
    let mut bytes = serde_json::to_vec_pretty(&snapshot)
        .context("failed to serialize display topology artifact")?;
    bytes.push(b'\n');
    fs::write(&path, bytes).with_context(|| {
        format!(
            "failed to write display topology artifact {}",
            path.display()
        )
    })
}
