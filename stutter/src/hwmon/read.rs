#[cfg(test)]
use std::path::PathBuf;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
};

use super::model::{HwmonReader, NvidiaWorker};
use crate::recorder::GpuSample;

impl HwmonReader {
    pub fn sample(&mut self, elapsed_ms: u64) -> GpuSample {
        let mut gpu_busy_percent = read_u32_cached(&mut self.gpu_busy, &mut self.buf);
        let mut vram_used_bytes = read_u64_cached(&mut self.vram_used, &mut self.buf);
        let mut vram_total_bytes = read_u64_cached(&mut self.vram_total, &mut self.buf);

        if let Some(sample) = self.nvidia_state.as_ref().and_then(NvidiaWorker::latest) {
            if gpu_busy_percent.is_none() {
                gpu_busy_percent = Some(sample.gpu_busy_percent);
            }
            if vram_used_bytes.is_none() {
                vram_used_bytes = Some(sample.vram_used_bytes);
            }
            if vram_total_bytes.is_none() {
                vram_total_bytes = Some(sample.vram_total_bytes);
            }
        }

        let vram_used_percent = match (vram_used_bytes, vram_total_bytes) {
            (Some(used), Some(total)) if total > 0 => {
                Some(((used as f64 / total as f64) * 100.0) as u32)
            }
            _ => None,
        };

        let gpu_clock_mhz = read_u32_cached(&mut self.freq1_input, &mut self.buf)
            .map(|val| if self.freq1_is_mhz { val } else { val / 1_000 });

        GpuSample {
            elapsed_ms,
            drm_card: self.drm_card.clone(),
            render_node: self.render_node.clone(),
            gpu_busy_percent,
            vram_used_bytes,
            vram_total_bytes,
            vram_used_percent,
            gpu_clock_mhz,
            mem_clock_mhz: read_u32_cached(&mut self.freq2_input, &mut self.buf)
                .map(|khz| khz / 1_000),
            temp_millidegrees: read_u32_cached(&mut self.temp1_input, &mut self.buf),
            power_microwatts: read_u64_cached(&mut self.power1_average, &mut self.buf),
            power_limit_reason: None,
        }
    }

    #[cfg(test)]
    pub(super) fn from_root(root: PathBuf) -> Self {
        Self {
            drm_card: None,
            render_node: None,
            gpu_busy: fs::File::open(root.join("gpu_busy_percent")).ok(),
            vram_used: None,
            vram_total: None,
            freq1_input: fs::File::open(root.join("freq1_input")).ok(),
            freq1_is_mhz: false,
            freq2_input: fs::File::open(root.join("freq2_input")).ok(),
            temp1_input: fs::File::open(root.join("temp1_input")).ok(),
            power1_average: fs::File::open(root.join("power1_average")).ok(),
            buf: String::with_capacity(32),
            nvidia_state: None,
        }
    }
}

fn read_u32_cached(file_opt: &mut Option<fs::File>, buf: &mut String) -> Option<u32> {
    let file = file_opt.as_mut()?;
    buf.clear();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(buf).ok()?;
    buf.trim().parse().ok()
}

fn read_u64_cached(file_opt: &mut Option<fs::File>, buf: &mut String) -> Option<u64> {
    let file = file_opt.as_mut()?;
    buf.clear();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(buf).ok()?;
    buf.trim().parse().ok()
}
