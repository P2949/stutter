//! Cross-checks a native benchmark's `summary.json` against its own `frames.csv`, and records the
//! graphics context the game reported. The game derives its average from the per-frame times, so a
//! summary that disagrees with its frame file, or a frame file with a different frame count, is not
//! one coherent native result and is rejected.

use serde::{Deserialize, Serialize};

const MAX_FRAMES: usize = 2_000_000;
/// The summary average is 1000 / mean(frame ms); allow for its printed precision.
const AVERAGE_TOLERANCE: f64 = 0.005;

/// What the game reported about how it ran. Compared across arms; never trusted for identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeContext {
    pub game_version: Option<String>,
    pub preset_name: Option<String>,
    pub gpu_name: Option<String>,
    pub upscaling_type: Option<i64>,
    pub ray_tracing_enabled: Option<bool>,
    pub frame_generation_type: Option<i64>,
    pub frame_count: u64,
}

fn text(data: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    data.get(key)?
        .as_str()
        .map(|value| value.chars().take(128).collect())
}

/// Frame times (ms) from the `Frame time (ms)` column of the game's CSV.
fn frame_times(csv: &[u8]) -> anyhow::Result<Vec<f64>> {
    let text = std::str::from_utf8(csv)?;
    let mut lines = text.lines();
    let header = lines
        .next()
        .ok_or_else(|| anyhow::anyhow!("frames.csv is empty"))?;
    let column = header
        .split(',')
        .position(|name| name.trim().eq_ignore_ascii_case("Frame time (ms)"))
        .ok_or_else(|| anyhow::anyhow!("frames.csv has no frame-time column"))?;
    let mut times = Vec::new();
    for line in lines.filter(|line| !line.trim().is_empty()) {
        anyhow::ensure!(times.len() < MAX_FRAMES, "frames.csv has too many frames");
        let value: f64 = line
            .split(',')
            .nth(column)
            .ok_or_else(|| anyhow::anyhow!("frames.csv row is short"))?
            .trim()
            .parse()?;
        anyhow::ensure!(
            value.is_finite() && value > 0.0,
            "frames.csv has an invalid frame time"
        );
        times.push(value);
    }
    anyhow::ensure!(!times.is_empty(), "frames.csv has no frames");
    Ok(times)
}

/// Verify the summary and frame file describe the same run and return the reported context.
pub fn verify_frames(summary: &[u8], frames_csv: &[u8]) -> anyhow::Result<NativeContext> {
    let value: serde_json::Value = serde_json::from_slice(summary)?;
    let root = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("native summary is not a JSON object"))?;
    let data = root
        .get("Data")
        .and_then(serde_json::Value::as_object)
        .unwrap_or(root);
    let times = frame_times(frames_csv)?;
    let count = times.len() as u64;
    if let Some(reported) = data.get("frameNumber").and_then(serde_json::Value::as_u64) {
        anyhow::ensure!(
            reported == count,
            "summary frame count differs from frames.csv"
        );
    }
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let from_frames = 1000.0 / mean;
    let average = data
        .get("averageFps")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| anyhow::anyhow!("native summary lacks averageFps"))?;
    anyhow::ensure!(
        (from_frames - average).abs() <= average * AVERAGE_TOLERANCE,
        "summary average FPS disagrees with the frame times"
    );
    Ok(NativeContext {
        game_version: text(data, "gameVersion"),
        preset_name: text(data, "presetName"),
        gpu_name: text(data, "gpuName"),
        upscaling_type: data
            .get("upscalingType")
            .and_then(serde_json::Value::as_i64),
        ray_tracing_enabled: data
            .get("rayTracingEnabled")
            .and_then(serde_json::Value::as_bool),
        frame_generation_type: data
            .get("frameGenerationType")
            .and_then(serde_json::Value::as_i64),
        frame_count: count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv(rows: usize, ms: f64) -> Vec<u8> {
        let mut text = String::from("Frame index, Frame time (ms), DLSS Frame Generation (ms)\n");
        for index in 0..rows {
            text.push_str(&format!("{index}, {ms:.2}, {ms:.2}\n"));
        }
        text.into_bytes()
    }

    #[test]
    fn a_coherent_real_shaped_result_yields_its_context() {
        let summary = br#"{"RootType":"worldBenchmarkSummary","Data":{"averageFps":71.0,"frameNumber":100,"gameVersion":"2.5","presetName":"RayTracingUltra","gpuName":"GPU","upscalingType":2,"rayTracingEnabled":true,"frameGenerationType":0}}"#;
        let context = verify_frames(summary, &csv(100, 1000.0 / 71.0)).unwrap();
        assert_eq!(context.frame_count, 100);
        assert_eq!(context.preset_name.as_deref(), Some("RayTracingUltra"));
        assert_eq!(context.ray_tracing_enabled, Some(true));
    }

    #[test]
    fn a_summary_that_disagrees_with_its_frame_file_is_rejected() {
        let summary = br#"{"Data":{"averageFps":71.0,"frameNumber":100}}"#;
        // Wrong average for the frame times.
        assert!(verify_frames(summary, &csv(100, 20.0)).is_err());
        // Wrong frame count.
        assert!(verify_frames(summary, &csv(99, 1000.0 / 71.0)).is_err());
        // Junk, empty and column-less files.
        assert!(verify_frames(summary, b"frame,time\n1,0\n").is_err());
        assert!(verify_frames(summary, b"").is_err());
        assert!(verify_frames(b"{}", &csv(100, 14.0)).is_err());
    }
}
