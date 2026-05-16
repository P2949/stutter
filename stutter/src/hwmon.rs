use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;

use crate::recorder::GpuSample;

#[derive(Debug)]
pub struct HwmonReader {
    drm_card: Option<String>,
    render_node: Option<String>,
    gpu_busy: Option<fs::File>,
    vram_used: Option<fs::File>,
    vram_total: Option<fs::File>,
    freq1_input: Option<fs::File>,
    freq1_is_mhz: bool,
    freq2_input: Option<fs::File>,
    temp1_input: Option<fs::File>,
    power1_average: Option<fs::File>,
    buf: String,
    nvidia_state: Option<NvidiaWorker>,
}

#[derive(Debug)]
struct NvidiaState {
    latest: Mutex<Option<NvidiaSample>>,
    shutdown: AtomicBool,
}

#[derive(Debug)]
struct NvidiaWorker {
    state: Arc<NvidiaState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NvidiaSample {
    gpu_busy_percent: u32,
    vram_used_bytes: u64,
    vram_total_bytes: u64,
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

pub fn probe_hwmon_with_options(
    root_override: Option<&Path>,
    drm_card: Option<&str>,
    render_node: Option<&Path>,
) -> HwmonProbeReport {
    let mut warnings = Vec::new();
    let selected_root = if let Some(root) = root_override {
        match validate_hwmon_root_override(root) {
            Ok(root) => Some(root),
            Err(err) => {
                warnings.push(err);
                None
            }
        }
    } else if let Some(card) = drm_card {
        let root = discover_drm_hwmon_root(Path::new("/sys/class/drm"), card);
        if root.is_none() {
            warnings.push(format!("DRM card hwmon root not found: {card}"));
        }
        root
    } else if let Some(node) = render_node {
        let root = node
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|file_name| discover_drm_hwmon_root(Path::new("/sys/class/drm"), file_name));
        if root.is_none() {
            warnings.push(format!(
                "render-node hwmon root not found: {}",
                node.display()
            ));
        }
        root
    } else {
        discover_hwmon_root(Path::new("/sys/class/drm"))
            .or_else(|| discover_hwmon_root(Path::new("/sys/class/hwmon")))
    };

    let nvidia_fallback_available = has_nvidia_pci_device();
    if selected_root.is_none() && !nvidia_fallback_available {
        warnings.push("no GPU hwmon root discovered".to_owned());
    }

    let (
        gpu_busy_available,
        vram_used_available,
        vram_total_available,
        temp_available,
        power_available,
    ) = selected_root
        .as_ref()
        .map(|root| {
            (
                root.join("device/gpu_busy_percent").exists()
                    || root.join("gpu_busy_percent").exists(),
                root.join("device/mem_info_vram_used").exists()
                    || root.join("mem_info_vram_used").exists(),
                root.join("device/mem_info_vram_total").exists()
                    || root.join("mem_info_vram_total").exists(),
                root.join("temp1_input").exists(),
                root.join("power1_average").exists(),
            )
        })
        .unwrap_or((false, false, false, false, false));

    HwmonProbeReport {
        selected_root,
        nvidia_fallback_available,
        gpu_busy_available,
        vram_used_available,
        vram_total_available,
        temp_available,
        power_available,
        warnings,
    }
}

impl NvidiaState {
    fn new() -> Self {
        Self {
            latest: Mutex::new(None),
            shutdown: AtomicBool::new(false),
        }
    }
}

impl NvidiaWorker {
    fn latest(&self) -> Option<NvidiaSample> {
        self.state.latest.lock().ok().and_then(|sample| *sample)
    }
}

impl Drop for NvidiaWorker {
    fn drop(&mut self) {
        self.state.shutdown.store(true, Ordering::Relaxed);
    }
}

impl HwmonReader {
    pub fn discover_with_options(
        root_override: Option<&Path>,
        drm_card: Option<&str>,
        render_node: Option<&Path>,
    ) -> Option<Self> {
        let root = if let Some(root) = root_override {
            match validate_hwmon_root_override(root) {
                Ok(root) => Some(root),
                Err(err) => {
                    log::warn!(
                        "hwmon_root_override_invalid path={} err={err}",
                        root.display()
                    );
                    None
                }
            }
        } else if let Some(card) = drm_card {
            let root = discover_drm_hwmon_root(Path::new("/sys/class/drm"), card);
            if root.is_none() {
                log::warn!("hwmon_drm_card_not_found card={card}");
            }
            root
        } else if let Some(node) = render_node {
            match node.file_name().and_then(|name| name.to_str()) {
                Some(file_name) => {
                    let root = discover_drm_hwmon_root(Path::new("/sys/class/drm"), file_name);
                    if root.is_none() {
                        log::warn!("hwmon_render_node_not_found node={}", node.display());
                    }
                    root
                }
                None => {
                    log::warn!("hwmon_render_node_invalid path={}", node.display());
                    None
                }
            }
        } else {
            discover_hwmon_root(Path::new("/sys/class/drm"))
                .or_else(|| discover_hwmon_root(Path::new("/sys/class/hwmon")))
        };

        if let Some(root) = root {
            Some(Self::from_hwmon_root_with_identity(
                root,
                drm_card.map(str::to_owned),
                render_node
                    .and_then(|node| node.file_name())
                    .and_then(|name| name.to_str())
                    .map(str::to_owned),
            ))
        } else if has_nvidia_pci_device() {
            let mut reader = Self::empty();
            reader.nvidia_state = Some(start_nvidia_smi_thread());
            Some(reader)
        } else {
            None
        }
    }

    fn empty() -> Self {
        Self {
            drm_card: None,
            render_node: None,
            gpu_busy: None,
            vram_used: None,
            vram_total: None,
            freq1_input: None,
            freq1_is_mhz: false,
            freq2_input: None,
            temp1_input: None,
            power1_average: None,
            buf: String::with_capacity(32),
            nvidia_state: None,
        }
    }

    fn from_hwmon_root_with_identity(
        root: PathBuf,
        drm_card: Option<String>,
        render_node: Option<String>,
    ) -> Self {
        let (freq1_input, freq1_is_mhz) = if let Ok(f) = fs::File::open(root.join("freq1_input")) {
            (Some(f), false)
        } else if let Ok(f) = fs::File::open(root.join("device/tile0/gt0/freq0/cur_freq_mhz")) {
            (Some(f), true)
        } else {
            (None, false)
        };

        let mut reader = Self {
            drm_card,
            render_node,
            gpu_busy: fs::File::open(root.join("device/gpu_busy_percent"))
                .or_else(|_| fs::File::open(root.join("gpu_busy_percent")))
                .ok(),
            vram_used: fs::File::open(root.join("device/mem_info_vram_used"))
                .or_else(|_| fs::File::open(root.join("mem_info_vram_used")))
                .ok(),
            vram_total: fs::File::open(root.join("device/mem_info_vram_total"))
                .or_else(|_| fs::File::open(root.join("mem_info_vram_total")))
                .ok(),
            freq1_input,
            freq1_is_mhz,
            freq2_input: fs::File::open(root.join("freq2_input")).ok(),
            temp1_input: fs::File::open(root.join("temp1_input")).ok(),
            power1_average: fs::File::open(root.join("power1_average")).ok(),
            buf: String::with_capacity(32),
            nvidia_state: None,
        };

        if reader.gpu_busy.is_none() && has_nvidia_pci_device() {
            reader.nvidia_state = Some(start_nvidia_smi_thread());
        }

        reader
    }

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
    fn from_root(root: PathBuf) -> Self {
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
    has_supported_hwmon_sensor_file(path).then(|| path.to_path_buf())
}

fn validate_hwmon_root_override(root: &Path) -> Result<PathBuf, String> {
    let metadata = fs::metadata(root).map_err(|err| {
        format!(
            "hwmon root override not accessible: {}: {err}",
            root.display()
        )
    })?;

    if !metadata.is_dir() {
        return Err(format!(
            "hwmon root override is not a hwmon directory: {} is not a directory",
            root.display()
        ));
    }

    if !has_supported_hwmon_sensor_file(root) {
        return Err(format!(
            "hwmon root override is not a hwmon directory: {} has no supported sensor files",
            root.display()
        ));
    }

    Ok(root.to_path_buf())
}

fn has_supported_hwmon_sensor_file(root: &Path) -> bool {
    [
        "device/gpu_busy_percent",
        "gpu_busy_percent",
        "device/mem_info_vram_used",
        "mem_info_vram_used",
        "device/mem_info_vram_total",
        "mem_info_vram_total",
        "freq1_input",
        "device/tile0/gt0/freq0/cur_freq_mhz",
        "freq2_input",
        "temp1_input",
        "power1_average",
    ]
    .iter()
    .any(|relative| root.join(relative).exists())
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

fn has_nvidia_pci_device() -> bool {
    static NVIDIA_PCI_PRESENT: OnceLock<bool> = OnceLock::new();

    *NVIDIA_PCI_PRESENT.get_or_init(has_nvidia_pci_device_uncached)
}

fn has_nvidia_pci_device_uncached() -> bool {
    let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") else {
        return false;
    };
    for entry in entries.flatten() {
        if let Ok(vendor) = fs::read_to_string(entry.path().join("vendor"))
            && vendor.trim() == "0x10de"
        {
            return true;
        }
    }
    false
}

fn start_nvidia_smi_thread() -> NvidiaWorker {
    let state = Arc::new(NvidiaState::new());

    let state_clone = state.clone();
    std::thread::spawn(move || {
        while !state_clone.shutdown.load(Ordering::Relaxed) {
            let output = std::process::Command::new("nvidia-smi")
                .args([
                    "--query-gpu=utilization.gpu,memory.used,memory.total",
                    "--format=csv,noheader,nounits",
                ])
                .output();

            if let Ok(out) = output
                && let Ok(s) = String::from_utf8(out.stdout)
                && let Some(sample) = parse_nvidia_smi_sample(&s)
                && let Ok(mut latest) = state_clone.latest.lock()
            {
                *latest = Some(sample);
            }

            // Sleep in small increments but with a larger total interval to
            // avoid spawning `nvidia-smi` frequently. Default total wait is
            // 5s, checked every 100ms so the worker can shut down promptly.
            let total_ms = 5_000u64;
            let step_ms = 100u64;
            let iterations = (total_ms / step_ms) as usize;
            for _ in 0..iterations {
                if state_clone.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(step_ms));
            }
        }
    });

    NvidiaWorker { state }
}

fn parse_nvidia_smi_sample(output: &str) -> Option<NvidiaSample> {
    let line = output.lines().next()?;
    let mut parts = line.split(',').map(str::trim);
    let busy = parts.next()?.parse::<u32>().ok()?;
    let used_mb = parts.next()?.parse::<u64>().ok()?;
    let total_mb = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    Some(NvidiaSample {
        gpu_busy_percent: busy,
        vram_used_bytes: used_mb * 1024 * 1024,
        vram_total_bytes: total_mb * 1024 * 1024,
    })
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
    fn nvidia_sample_is_absent_until_worker_records_data() {
        let state = NvidiaState::new();

        assert_eq!(*state.latest.lock().unwrap(), None);
    }

    #[test]
    fn parses_nvidia_smi_csv_sample_without_sentinels() {
        let sample = parse_nvidia_smi_sample("42, 1024, 8192\n").unwrap();

        assert_eq!(sample.gpu_busy_percent, 42);
        assert_eq!(sample.vram_used_bytes, 1024 * 1024 * 1024);
        assert_eq!(sample.vram_total_bytes, 8192 * 1024 * 1024);
        assert_eq!(parse_nvidia_smi_sample("not-ready\n"), None);
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
    fn probe_hwmon_with_options_reports_available_fake_files() {
        let root = temp_dir("hwmon-probe");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("gpu_busy_percent"), "55\n").unwrap();
        fs::write(root.join("temp1_input"), "47000\n").unwrap();
        fs::write(root.join("power1_average"), "100\n").unwrap();

        let report = probe_hwmon_with_options(Some(&root), None, None);

        assert_eq!(report.selected_root, Some(root.clone()));
        assert!(report.gpu_busy_available);
        assert!(!report.vram_used_available);
        assert!(report.temp_available);
        assert!(report.power_available);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hwmon_root_override_rejects_missing_path() {
        let root = temp_dir("hwmon-missing");

        let report = probe_hwmon_with_options(Some(&root), None, None);

        assert_eq!(report.selected_root, None);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("hwmon root override not accessible"))
        );
    }

    #[test]
    fn hwmon_root_override_rejects_file_path() {
        let root = temp_dir("hwmon-file");
        if let Some(parent) = root.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&root, "not a directory\n").unwrap();

        let report = probe_hwmon_with_options(Some(&root), None, None);

        assert_eq!(report.selected_root, None);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("is not a directory"))
        );

        fs::remove_file(root).ok();
    }

    #[test]
    fn hwmon_root_override_rejects_directory_without_supported_sensor_files() {
        let root = temp_dir("hwmon-empty");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("name"), "fake\n").unwrap();

        let report = probe_hwmon_with_options(Some(&root), None, None);

        assert_eq!(report.selected_root, None);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("has no supported sensor files"))
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn hwmon_root_override_accepts_non_sysfs_fake_root_with_supported_sensor_file() {
        let root = temp_dir("hwmon-fake-valid");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("temp1_input"), "47000\n").unwrap();

        let report = probe_hwmon_with_options(Some(&root), None, None);

        assert_eq!(report.selected_root, Some(root.clone()));
        assert!(report.temp_available);

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
