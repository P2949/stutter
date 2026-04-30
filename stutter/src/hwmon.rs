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
    pub fn discover_with_options(
        root_override: Option<&Path>,
        drm_card: Option<&str>,
        render_node: Option<&Path>,
    ) -> Option<Self> {
        let root = if let Some(root) = root_override {
            if root.exists() {
                Some(root.to_path_buf())
            } else {
                log::warn!("hwmon_root_override_not_found path={}", root.display());
                None
            }
        } else if let Some(card) = drm_card {
            let root = discover_drm_hwmon_root(Path::new("/sys/class/drm"), card);
            if root.is_none() {
                log::warn!("hwmon_drm_card_not_found card={card}");
            }
            root
        } else if let Some(node) = render_node {
            let root = node
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| discover_drm_hwmon_root(Path::new("/sys/class/drm"), name));
            if root.is_none() {
                log::warn!("hwmon_render_node_not_found node={}", node.display());
            }
            root
        } else {
            discover_hwmon_root(Path::new("/sys/class/drm"))
                .or_else(|| discover_hwmon_root(Path::new("/sys/class/hwmon")))
        }?;

        Some(Self::from_hwmon_root(root))
    }

    fn from_hwmon_root(root: PathBuf) -> Self {
        Self {
            gpu_busy: fs::File::open(root.join("device/gpu_busy_percent"))
                .or_else(|_| fs::File::open(root.join("gpu_busy_percent")))
                .ok(),
            vram_used: fs::File::open(root.join("device/mem_info_vram_used"))
                .or_else(|_| fs::File::open(root.join("mem_info_vram_used")))
                .ok(),
            vram_total: fs::File::open(root.join("device/mem_info_vram_total"))
                .or_else(|_| fs::File::open(root.join("mem_info_vram_total")))
                .ok(),
            freq1_input: fs::File::open(root.join("freq1_input")).ok(),
            freq2_input: fs::File::open(root.join("freq2_input")).ok(),
            temp1_input: fs::File::open(root.join("temp1_input")).ok(),
            power1_average: fs::File::open(root.join("power1_average")).ok(),
            buf: String::with_capacity(32),
        }
    }

    pub fn sample(&mut self, elapsed_ms: u128) -> GpuSample {
        GpuSample {
            elapsed_ms,
            gpu_busy_percent: read_u32_cached(&mut self.gpu_busy, &mut self.buf),
            vram_used_bytes: read_u64_cached(&mut self.vram_used, &mut self.buf),
            vram_total_bytes: read_u64_cached(&mut self.vram_total, &mut self.buf),
            gpu_clock_mhz: read_u32_cached(&mut self.freq1_input, &mut self.buf)
                .map(|khz| khz / 1_000),
            mem_clock_mhz: read_u32_cached(&mut self.freq2_input, &mut self.buf)
                .map(|khz| khz / 1_000),
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

fn discover_drm_hwmon_root(drm_root: &Path, drm_name: &str) -> Option<PathBuf> {
    if drm_name.is_empty() || drm_name.contains('/') {
        return None;
    }

    let drm_path = drm_root.join(drm_name);
    first_hwmon_child(&drm_path.join("device/hwmon"))
        .or_else(|| first_hwmon_child(&drm_path.join("hwmon")))
        .or_else(|| sensor_root(&drm_path.join("device")))
        .or_else(|| sensor_root(&drm_path))
}

fn sensor_root(path: &Path) -> Option<PathBuf> {
    (path.join("gpu_busy_percent").exists()
        || path.join("temp1_input").exists()
        || path.join("power1_average").exists())
    .then(|| path.to_path_buf())
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

    #[test]
    fn discover_at_uses_fake_hwmon_root_override() {
        let root = temp_dir("hwmon-discover");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("gpu_busy_percent"), "55\n").unwrap();
        fs::write(root.join("temp1_input"), "47000\n").unwrap();

        let mut reader = HwmonReader::discover_with_options(Some(&root), None, None).unwrap();
        let sample = reader.sample(7);

        assert_eq!(sample.elapsed_ms, 7);
        assert_eq!(sample.gpu_busy_percent, Some(55));
        assert_eq!(sample.temp_millidegrees, Some(47000));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn discover_drm_hwmon_root_selects_requested_card() {
        let root = temp_dir("drm-hwmon");
        let card0 = root.join("card0/device/hwmon/hwmon0");
        let card1 = root.join("card1/device/hwmon/hwmon1");
        fs::create_dir_all(&card0).unwrap();
        fs::create_dir_all(&card1).unwrap();
        fs::write(card0.join("temp1_input"), "39000\n").unwrap();
        fs::write(card1.join("temp1_input"), "61000\n").unwrap();

        assert_eq!(discover_drm_hwmon_root(&root, "card1"), Some(card1));
        assert_eq!(discover_drm_hwmon_root(&root, "card0"), Some(card0));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn discover_drm_hwmon_root_selects_render_node_name() {
        let root = temp_dir("render-hwmon");
        let render = root.join("renderD129/device/hwmon/hwmon3");
        fs::create_dir_all(&render).unwrap();
        fs::write(render.join("power1_average"), "100\n").unwrap();

        assert_eq!(discover_drm_hwmon_root(&root, "renderD129"), Some(render));

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
