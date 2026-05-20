//! Recording runtime setup for monitor sessions.

use std::fs;

use log::info;

use crate::{
    config::{CsvStreamTarget, model::MonitorConfig},
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

            let registry = &mut recorder.streams;
            let dir = &run.run_dir;

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
