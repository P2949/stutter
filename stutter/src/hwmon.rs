use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use crate::recorder::GpuSample;

#[derive(Debug)]
pub struct HwmonReader {
    gpu_busy: Option<fs::File>,
    vram_used: Option<fs::File>,
    vram_total: Option<fs::File>,
    freq1_input: Option<fs::File>,
    freq2_input: Option<fs::File>,
    temp1_input: Option<fs::File>,
    power1_average: Option<fs::File>,
    buf: String,
}

impl HwmonReader {
    pub fn discover_at(root_override: Option<&Path>) -> Option<Self> {
        let overridden = root_override.and_then(|p| {
            if p.exists() {
                Some(p.to_path_buf())
            } else {
                log::warn!("hwmon_root_override_not_found path={}", p.display());
                None
            }
        });

        let root = overridden
            .or_else(|| discover_hwmon_root(Path::new("/sys/class/drm")))
            .or_else(|| discover_hwmon_root(Path::new("/sys/class/hwmon")))?;

        Some(Self {
            gpu_busy: fs::File::open(root.join("device/gpu_busy_percent"))
                .or_else(|_| fs::File::open(root.join("gpu_busy_percent"))).ok(),
            vram_used: fs::File::open(root.join("device/mem_info_vram_used"))
                .or_else(|_| fs::File::open(root.join("mem_info_vram_used"))).ok(),
            vram_total: fs::File::open(root.join("device/mem_info_vram_total"))
                .or_else(|_| fs::File::open(root.join("mem_info_vram_total"))).ok(),
            freq1_input: fs::File::open(root.join("freq1_input")).ok(),
            freq2_input: fs::File::open(root.join("freq2_input")).ok(),
            temp1_input: fs::File::open(root.join("temp1_input")).ok(),
            power1_average: fs::File::open(root.join("power1_average")).ok(),
            buf: String::with_capacity(32),
        })
    }

    pub fn sample(&mut self, elapsed_ms: u128) -> GpuSample {
        GpuSample {
            elapsed_ms,
            gpu_busy_percent: read_u32_cached(&mut self.gpu_busy, &mut self.buf),
            vram_used_bytes: read_u64_cached(&mut self.vram_used, &mut self.buf),
            vram_total_bytes: read_u64_cached(&mut self.vram_total, &mut self.buf),
            gpu_clock_mhz: read_u32_cached(&mut self.freq1_input, &mut self.buf).map(|khz| khz / 1_000),
            mem_clock_mhz: read_u32_cached(&mut self.freq2_input, &mut self.buf).map(|khz| khz / 1_000),
            temp_millidegrees: read_u32_cached(&mut self.temp1_input, &mut self.buf),
            power_microwatts: read_u64_cached(&mut self.power1_average, &mut self.buf),
        }
    }

    #[cfg(test)]
    fn from_root(root: PathBuf) -> Self {
        Self {
            gpu_busy: fs::File::open(root.join("gpu_busy_percent")).ok(),
            vram_used: None,
            vram_total: None,
            freq1_input: fs::File::open(root.join("freq1_input")).ok(),
            freq2_input: fs::File::open(root.join("freq2_input")).ok(),
            temp1_input: fs::File::open(root.join("temp1_input")).ok(),
            power1_average: fs::File::open(root.join("power1_average")).ok(),
            buf: String::with_capacity(32),
        }
    }
}

fn discover_hwmon_root(root: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some("hwmon")
            && let Some(hwmon) = first_hwmon_child(&path)
        {
            return Some(hwmon);
        }
        if let Some(hwmon) = first_hwmon_child(&path.join("device/hwmon")) {
            return Some(hwmon);
        }
        if path.join("gpu_busy_percent").exists() || path.join("temp1_input").exists() {
            return Some(path);
        }
    }
    None
}

fn first_hwmon_child(path: &Path) -> Option<PathBuf> {
    fs::read_dir(path)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.join("temp1_input").exists() || path.join("power1_average").exists())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_basic_hwmon_fields() {
        let root = temp_dir("hwmon");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("gpu_busy_percent"), "97\n").unwrap();
        fs::write(root.join("freq1_input"), "2200000\n").unwrap();
        fs::write(root.join("freq2_input"), "1000000\n").unwrap();
        fs::write(root.join("temp1_input"), "61000\n").unwrap();
        fs::write(root.join("power1_average"), "120000000\n").unwrap();

        let mut reader = HwmonReader::from_root(root.clone());
        let sample = reader.sample(123);

        assert_eq!(sample.elapsed_ms, 123);
        assert_eq!(sample.gpu_busy_percent, Some(97));
        assert_eq!(sample.gpu_clock_mhz, Some(2200));
        assert_eq!(sample.mem_clock_mhz, Some(1000));
        assert_eq!(sample.temp_millidegrees, Some(61000));
        assert_eq!(sample.power_microwatts, Some(120000000));

        fs::remove_dir_all(root).ok();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }
}
