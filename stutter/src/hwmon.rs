use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::recorder::GpuSample;

#[derive(Clone, Debug)]
pub struct HwmonReader {
    root: PathBuf,
}

impl HwmonReader {
    pub fn discover() -> Option<Self> {
        discover_hwmon_root(Path::new("/sys/class/drm"))
            .or_else(|| discover_hwmon_root(Path::new("/sys/class/hwmon")))
            .map(|root| Self { root })
    }

    pub fn sample(&self, elapsed_ms: u128) -> GpuSample {
        GpuSample {
            elapsed_ms,
            gpu_busy_percent: read_u32(self.root.join("device/gpu_busy_percent"))
                .or_else(|| read_u32(self.root.join("gpu_busy_percent"))),
            vram_used_bytes: read_u64(self.root.join("device/mem_info_vram_used"))
                .or_else(|| read_u64(self.root.join("mem_info_vram_used"))),
            vram_total_bytes: read_u64(self.root.join("device/mem_info_vram_total"))
                .or_else(|| read_u64(self.root.join("mem_info_vram_total"))),
            gpu_clock_mhz: read_u32(self.root.join("freq1_input")).map(|khz| khz / 1_000),
            mem_clock_mhz: read_u32(self.root.join("freq2_input")).map(|khz| khz / 1_000),
            temp_millidegrees: read_u32(self.root.join("temp1_input")),
            power_microwatts: read_u64(self.root.join("power1_average")),
        }
    }

    #[cfg(test)]
    fn from_root(root: PathBuf) -> Self {
        Self { root }
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

fn read_u32(path: PathBuf) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_u64(path: PathBuf) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
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

        let sample = HwmonReader::from_root(root.clone()).sample(123);

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
