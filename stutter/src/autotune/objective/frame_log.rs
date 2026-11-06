use std::{path::Path, time::Duration};

use crate::autotune::rolling_window::RollingWindow;

/// Frame statistics of a MangoHud log as the controller computes them: the same parser and
/// plausibility filter feed the same rolling-window p99 the live observation uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameLogStats {
    pub valid_frames: usize,
    pub p99_ms: f64,
}

/// Statistics over the first `first_n` frames of the log (all frames when `None`).
pub fn frame_log_stats(path: &Path, first_n: Option<usize>) -> anyhow::Result<FrameLogStats> {
    let events = crate::mangohud::read_frame_events(path, 0, None, None, None)?;
    let mut window = RollingWindow::new(Duration::from_secs(24 * 3600));
    for event in events.into_iter().take(first_n.unwrap_or(usize::MAX)) {
        window.push_frame(event);
    }
    Ok(FrameLogStats {
        valid_frames: window.frames().len(),
        p99_ms: window.frame_p99_ms(),
    })
}
