use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

use super::{
    RecordingRun,
    metadata::{monotonic_now_ns, timestamp_for_path},
};
use crate::{
    config::model::{MonitorConfig, RecordingConfig},
    recorder::{
        RecordingRetentionPolicy, apply_recording_retention, ensure_min_free_space_for_path,
    },
};

pub fn prepare_recording(config: &MonitorConfig) -> anyhow::Result<Option<RecordingRun>> {
    let recording = &config.recording;
    if recording.run_name.is_none() && recording.output_dir.is_none() {
        return Ok(None);
    }

    let started_at = SystemTime::now();
    let run_dir = resolve_run_dir(recording, started_at, env::var_os("HOME"));
    let retention_policy = RecordingRetentionPolicy::from_recording_config(recording);
    if recording.output_dir.is_none()
        && let Some(run_root) = run_dir.parent()
    {
        apply_recording_retention(run_root, &retention_policy, None, started_at)?;
    }
    if let Some(min_free_bytes) = retention_policy.min_free_bytes {
        ensure_min_free_space_for_path(&run_dir, min_free_bytes)?;
    }
    if let Err(err) = ensure_empty_dir(&run_dir) {
        return Err(err.context("record write failed"));
    }

    Ok(Some(RecordingRun {
        run_name: recording.run_name.clone(),
        run_dir,
        started_at,
        started_instant: Instant::now(),
        monotonic_start_ns: monotonic_now_ns(),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    }))
}

fn resolve_run_dir(
    recording: &RecordingConfig,
    started_at: SystemTime,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(out_dir) = &recording.output_dir {
        return out_dir.clone();
    }

    let mut base = home
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push(".local");
    base.push("state");
    base.push("stutter");
    base.push("runs");

    let run_name = recording.run_name.as_deref().unwrap_or("run");
    base.push(format!(
        "{}_{}",
        timestamp_for_path(started_at),
        sanitize_run_name(run_name)
    ));
    base
}

fn ensure_empty_dir(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::bail!("output directory already exists: {}", path.display());
    }

    fs::create_dir_all(path)?;
    Ok(())
}

fn sanitize_run_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
