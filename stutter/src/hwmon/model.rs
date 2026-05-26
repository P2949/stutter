use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;

#[derive(Debug)]
pub struct HwmonReader {
    pub(super) drm_card: Option<String>,
    pub(super) render_node: Option<String>,
    pub(super) gpu_busy: Option<fs::File>,
    pub(super) vram_used: Option<fs::File>,
    pub(super) vram_total: Option<fs::File>,
    pub(super) freq1_input: Option<fs::File>,
    pub(super) freq1_is_mhz: bool,
    pub(super) freq2_input: Option<fs::File>,
    pub(super) temp1_input: Option<fs::File>,
    pub(super) power1_average: Option<fs::File>,
    pub(super) buf: String,
    pub(super) nvidia_state: Option<NvidiaWorker>,
}

#[derive(Debug)]
pub(super) struct NvidiaState {
    pub(super) latest: Mutex<Option<NvidiaSample>>,
    pub(super) shutdown: AtomicBool,
}

#[derive(Debug)]
pub(super) struct NvidiaWorker {
    pub(super) state: Arc<NvidiaState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NvidiaSample {
    pub(super) gpu_busy_percent: u32,
    pub(super) vram_used_bytes: u64,
    pub(super) vram_total_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HwmonProbeReport {
    pub selected_root: Option<PathBuf>,
    pub nvidia_fallback_available: bool,
    pub gpu_busy_available: bool,
    pub vram_used_available: bool,
    pub vram_total_available: bool,
    pub temp_available: bool,
    pub power_available: bool,
    pub warnings: Vec<String>,
}

impl NvidiaState {
    pub(super) fn new() -> Self {
        Self {
            latest: Mutex::new(None),
            shutdown: AtomicBool::new(false),
        }
    }
}

impl NvidiaWorker {
    pub(super) fn latest(&self) -> Option<NvidiaSample> {
        self.state.latest.lock().ok().and_then(|sample| *sample)
    }
}

impl Drop for NvidiaWorker {
    fn drop(&mut self) {
        self.state.shutdown.store(true, Ordering::Relaxed);
    }
}
