//! Frame metrics of one MangoHud log, with the warmup trimmed by elapsed time.
use super::outcome::{Summary, summarize};

#[derive(Debug, PartialEq)]
pub struct FrameMetrics {
    pub frames: usize,
    pub summary: Summary,
}

fn split(line: &str) -> Vec<&str> {
    line.split(',').map(str::trim).collect()
}

/// Frametimes (ms) from `csv` after `warmup_ms` of elapsed time. Returns `None` when there is no
/// recognized header, no usable frame, or any frame is malformed: damaged data is not repaired.
pub fn read_frametimes(csv: &str, warmup_ms: f64) -> Option<Vec<f64>> {
    let mut lines = csv.lines();
    let mut layout = None;
    for line in lines.by_ref() {
        let columns = split(line);
        let frametime = columns
            .iter()
            .position(|name| name.eq_ignore_ascii_case("frametime"));
        if let Some(frametime) = frametime {
            let elapsed = columns.iter().position(|name| {
                name.eq_ignore_ascii_case("elapsed") || name.eq_ignore_ascii_case("elapsed_ns")
            });
            layout = Some((frametime, elapsed));
            break;
        }
    }
    let (frametime_index, elapsed_index) = layout?;
    let mut frametimes = Vec::new();
    let mut cumulative_ms = 0.0;
    for line in lines.filter(|line| !line.trim().is_empty()) {
        let columns = split(line);
        let frametime: f64 = columns.get(frametime_index)?.parse().ok()?;
        if !frametime.is_finite() || frametime <= 0.0 {
            return None;
        }
        let elapsed_ms = match elapsed_index {
            Some(index) => columns.get(index)?.parse::<f64>().ok()? / 1_000_000.0,
            None => {
                cumulative_ms += frametime;
                cumulative_ms
            }
        };
        if elapsed_ms >= warmup_ms {
            frametimes.push(frametime);
        }
    }
    (!frametimes.is_empty()).then_some(frametimes)
}

pub fn metrics(csv: &str, warmup_ms: f64) -> Option<FrameMetrics> {
    let frametimes = read_frametimes(csv, warmup_ms)?;
    Some(FrameMetrics {
        frames: frametimes.len(),
        summary: summarize(&frametimes)?,
    })
}

/// A manifest metric name resolved against a log's metrics; `None` for a name this runner does
/// not measure, so an unknown metric is a blocked cohort and never a silent zero.
pub fn metric_value(name: &str, metrics: &FrameMetrics) -> Option<f64> {
    match name {
        "frame_p50_ms" => Some(metrics.summary.p50),
        "frame_p95_ms" => Some(metrics.summary.p95),
        "frame_p99_ms" => Some(metrics.summary.p99),
        "frame_mean_ms" => Some(metrics.summary.mean),
        "frame_count" => Some(metrics.frames as f64),
        _ => None,
    }
}
