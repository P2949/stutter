//! A read-only observer of each GPU's exact identity and power-profile state (package W6).
//!
//! It reads sysfs only: the PCI slot, vendor, device and subsystem ids, the bound driver, the
//! NUMA node, the DPM force level and the active line of `pp_power_profile_mode`. It never writes,
//! so it is safe on a host where GPU mutation is quarantined, and it gives the owner report
//! (LACT, CoreCtrl) an exact device to be about rather than a bare process name.
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Most DRM cards read; a host with more is not a realistic desktop.
const MAX_CARDS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuObservation {
    pub card: String,
    /// The PCI slot, e.g. `0000:03:00.0`, taken from the device symlink.
    pub pci_slot: String,
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub subsystem_vendor_id: Option<String>,
    pub subsystem_device_id: Option<String>,
    pub driver: Option<String>,
    pub numa_node: Option<i32>,
    /// `auto`, `manual`, `high`, `low`, ... as the driver reports; absent where unsupported.
    pub dpm_force_level: Option<String>,
    /// The profile line marked active (`*`) in `pp_power_profile_mode`.
    pub active_profile: Option<String>,
}

fn read(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty() && text.len() <= 4096).then(|| text.to_owned())
}

fn active_profile(text: &str) -> Option<String> {
    text.lines()
        .find(|line| line.contains('*'))
        .map(|line| {
            line.replace('*', " ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|line| !line.is_empty())
}

/// `card0`, `card1`, ... under `drm_root` (`/sys/class/drm`); connectors (`card0-DP-1`) are skipped.
pub fn observe_gpus(drm_root: &Path) -> Vec<GpuObservation> {
    let Ok(entries) = fs::read_dir(drm_root) else {
        return Vec::new();
    };
    let mut cards: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let digits = name.strip_prefix("card")?;
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| (name, entry.path().join("device")))
        })
        .collect();
    cards.sort();
    cards.truncate(MAX_CARDS);
    cards
        .into_iter()
        .filter_map(|(card, device)| {
            let slot = fs::canonicalize(&device)
                .ok()?
                .file_name()?
                .to_str()?
                .to_owned();
            Some(GpuObservation {
                card,
                pci_slot: slot,
                vendor_id: read(&device.join("vendor")),
                device_id: read(&device.join("device")),
                subsystem_vendor_id: read(&device.join("subsystem_vendor")),
                subsystem_device_id: read(&device.join("subsystem_device")),
                driver: fs::read_link(device.join("driver"))
                    .ok()
                    .and_then(|link| link.file_name()?.to_str().map(str::to_owned)),
                numa_node: read(&device.join("numa_node")).and_then(|n| n.parse().ok()),
                dpm_force_level: read(&device.join("power_dpm_force_performance_level")),
                active_profile: read(&device.join("pp_power_profile_mode"))
                    .and_then(|text| active_profile(&text)),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "gpu_identity_tests.rs"]
mod tests;
