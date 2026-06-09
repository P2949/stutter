//! MangoHud frame finalization helpers for monitor sessions.

use log::warn;

use crate::{
    artifacts::{ArtifactKind, push_artifact_event},
    config::model::MonitorConfig,
    mangohud, recorder,
};

pub(crate) fn read_and_stream_non_live_events(
    config: &MonitorConfig,
    recorder: &mut recorder::LiveRecorder,
) -> Vec<recorder::FrameEvent> {
    let Some(path) = config.mangohud.log.as_ref() else {
        return Vec::new();
    };
    if config.mangohud.log_live {
        return Vec::new();
    }

    let Some(run) = recorder.run.as_ref() else {
        return Vec::new();
    };
    let alignment_monotonic_ns = run.mangohud_first_frame_monotonic_ns;
    let alignment_raw_elapsed_ms = run.mangohud_first_frame_raw_elapsed_ms;
    let mangohud_ignore_offset = run.mangohud_start_offset.unwrap_or(0);
    let recorder_start_monotonic_ns = run.monotonic_start_ns;

    let frame_events = match mangohud::read_frame_events(
        path,
        mangohud_ignore_offset,
        alignment_monotonic_ns,
        alignment_raw_elapsed_ms,
        recorder_start_monotonic_ns,
    ) {
        Ok(events) => events,
        Err(err) => {
            warn!(
                "mangohud_log_read_failed path={} err={err:#}",
                path.display()
            );
            Vec::new()
        }
    };

    stream_frame_events(recorder, &frame_events);
    frame_events
}

fn stream_frame_events(
    recorder: &mut recorder::LiveRecorder,
    frame_events: &[recorder::FrameEvent],
) {
    if !recorder.streams.contains(ArtifactKind::FrameEvents) {
        return;
    }

    for frame in frame_events {
        push_artifact_event(
            recorder,
            ArtifactKind::FrameEvents,
            frame,
            "frame_events",
            |c| c.frame_event_count += 1,
        );
    }
}
