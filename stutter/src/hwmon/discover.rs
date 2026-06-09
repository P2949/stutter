use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    classify::{has_nvidia_pci_device, start_nvidia_smi_thread},
    model::{HwmonProbeReport, HwmonReader},
};

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

    pub(super) fn empty() -> Self {
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

    pub(super) fn from_hwmon_root_with_identity(
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

pub(super) fn discover_drm_hwmon_root(drm_root: &Path, drm_name: &str) -> Option<PathBuf> {
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
