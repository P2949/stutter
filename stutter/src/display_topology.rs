//! Automatic GPU/display topology detection.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DisplayTopologySnapshot {
    pub collected_at_elapsed_ms: Option<u64>,
    pub session_type: Option<String>,
    pub compositor: Option<CompositorInfo>,
    pub drm_devices: Vec<DrmDeviceInfo>,
    pub connectors: Vec<ConnectorInfo>,
    pub guessed_path: Option<DisplayPathGuess>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompositorInfo {
    pub name: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DrmDeviceInfo {
    pub card: String,                // card0
    pub render_node: Option<String>, // renderD128
    pub driver: Option<String>,      // i915, amdgpu
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub pci_slot: Option<String>,
    pub boot_vga: Option<bool>,
    pub hwmon_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectorInfo {
    pub card: String,
    pub name: String,           // HDMI-A-1, DP-1
    pub status: Option<String>, // connected/disconnected
    pub enabled: Option<String>,
    pub modes: Vec<String>,
    pub edid_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisplayPathGuess {
    pub render_card: Option<String>,
    pub render_driver: Option<String>,
    pub scanout_card: Option<String>,
    pub scanout_driver: Option<String>,
    pub connector: Option<String>,
    pub is_cross_gpu: Option<bool>,
    pub confidence: String,
    pub reasons: Vec<String>,
}

const KNOWN_COMPOSITORS: &[&str] = &[
    "gnome-shell",
    "kwin_wayland",
    "kwin_x11",
    "sway",
    "hyprland",
    "gamescope",
    "weston",
    "wayfire",
    "labwc",
    "xorg",
    "Xorg",
    "X",
];

pub fn probe_display_topology() -> DisplayTopologySnapshot {
    probe_display_topology_root(Path::new("/proc"), Path::new("/sys"))
}

pub fn probe_display_topology_root(proc_root: &Path, sys_root: &Path) -> DisplayTopologySnapshot {
    let mut warnings = Vec::new();

    let session_type = std::env::var("XDG_SESSION_TYPE")
        .ok()
        .filter(|s| !s.trim().is_empty());

    let compositor = detect_compositor(proc_root);
    let drm_devices = probe_drm_devices(sys_root);
    let connectors = probe_connectors(sys_root, &mut warnings);
    let guessed_path = guess_display_path(&drm_devices, &connectors, &mut warnings);

    DisplayTopologySnapshot {
        collected_at_elapsed_ms: None,
        session_type,
        compositor,
        drm_devices,
        connectors,
        guessed_path,
        warnings,
    }
}

fn detect_compositor(proc_root: &Path) -> Option<CompositorInfo> {
    let entries = std::fs::read_dir(proc_root).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        let Ok(pid) = name_str.parse::<u32>() else {
            continue;
        };
        let comm_path = entry.path().join("comm");
        if let Ok(comm) = std::fs::read_to_string(comm_path) {
            let comm_trimmed = comm.trim();
            if KNOWN_COMPOSITORS.contains(&comm_trimmed) {
                return Some(CompositorInfo {
                    name: comm_trimmed.to_owned(),
                    pid: Some(pid),
                });
            }
        }
    }
    None
}

fn probe_drm_devices(sys_root: &Path) -> Vec<DrmDeviceInfo> {
    let drm_root = sys_root.join("class/drm");
    let Ok(entries) = std::fs::read_dir(drm_root) else {
        return Vec::new();
    };

    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') || !path.exists() {
            continue;
        }

        let device_path = path.join("device");
        let vendor_id = read_trimmed(device_path.join("vendor"));
        let device_id = read_trimmed(device_path.join("device"));

        let driver = if let Ok(driver_link) = std::fs::read_link(device_path.join("driver")) {
            driver_link
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        } else {
            None
        };

        let pci_slot = if let Ok(canon_device) = std::fs::canonicalize(&device_path) {
            canon_device
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        } else {
            None
        };

        let boot_vga = read_trimmed(device_path.join("boot_vga")).map(|val| val.trim() == "1");

        let render_node = std::fs::read_dir(path.join("device/drm"))
            .ok()
            .and_then(|entries| {
                entries.flatten().find_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    name.starts_with("renderD").then_some(name)
                })
            });

        let hwmon_paths = std::fs::read_dir(path.join("device/hwmon"))
            .ok()
            .map(|entries| {
                let mut paths = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>();
                paths.sort();
                paths
            })
            .unwrap_or_default();

        devices.push(DrmDeviceInfo {
            card: name,
            render_node,
            driver,
            vendor_id,
            device_id,
            pci_slot,
            boot_vga,
            hwmon_paths,
        });
    }

    devices.sort_by(|left, right| left.card.cmp(&right.card));
    devices
}

fn probe_connectors(sys_root: &Path, _warnings: &mut Vec<String>) -> Vec<ConnectorInfo> {
    let drm_root = sys_root.join("class/drm");
    let Ok(entries) = std::fs::read_dir(&drm_root) else {
        return Vec::new();
    };

    let mut connectors = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || !name.contains('-') {
            continue;
        }

        let Some((card_part, conn_part)) = name.split_once('-') else {
            continue;
        };

        let status = read_trimmed(path.join("status"));
        let enabled = read_trimmed(path.join("enabled"));
        let modes = read_trimmed_lines(path.join("modes"));

        let edid_hash = if let Ok(edid_bytes) = std::fs::read(path.join("edid")) {
            if !edid_bytes.is_empty() && edid_bytes.iter().any(|&b| b != 0) {
                Some(stable_hash_bytes(&edid_bytes))
            } else {
                None
            }
        } else {
            None
        };

        connectors.push(ConnectorInfo {
            card: card_part.to_owned(),
            name: conn_part.to_owned(),
            status,
            enabled,
            modes,
            edid_hash,
        });
    }

    connectors.sort_by(|a, b| (&a.card, &a.name).cmp(&(&b.card, &b.name)));
    connectors
}

fn guess_display_path(
    drm_devices: &[DrmDeviceInfo],
    connectors: &[ConnectorInfo],
    warnings: &mut Vec<String>,
) -> Option<DisplayPathGuess> {
    if drm_devices.is_empty() {
        return None;
    }

    let mut reasons = Vec::new();

    // 1. Find the scanout card.
    let mut scanout_card = None;
    let mut connected_connector = None;

    for connector in connectors {
        if connector.status.as_deref() == Some("connected") {
            scanout_card = Some(connector.card.clone());
            connected_connector = Some(connector.name.clone());
            reasons.push(format!(
                "Detected connected connector {} on {}",
                connector.name, connector.card
            ));
            break;
        }
    }

    if scanout_card.is_none() {
        warnings.push("no connected connector found; display path is unknown".to_owned());
        let render_device = preferred_render_device(drm_devices);
        if let Some(device) = render_device {
            reasons.push(format!(
                "No connected connector found; render device candidate is {}",
                device.card
            ));
        }
        return Some(DisplayPathGuess {
            render_card: render_device.map(|device| device.card.clone()),
            render_driver: render_device.and_then(|device| device.driver.clone()),
            scanout_card: None,
            scanout_driver: None,
            connector: None,
            is_cross_gpu: None,
            confidence: "unknown".to_owned(),
            reasons,
        });
    }

    let scanout_card_str = scanout_card.as_ref()?;
    let scanout_device = drm_devices.iter().find(|d| &d.card == scanout_card_str);
    let scanout_driver = scanout_device.and_then(|d| d.driver.clone());

    // 2. Find the render card.
    let render_device = preferred_render_device(drm_devices);
    let mut render_card = render_device.map(|device| {
        if is_discrete_gpu(device) {
            reasons.push(format!(
                "Detected discrete GPU {} with driver {:?}",
                device.card, device.driver
            ));
        } else {
            reasons.push(format!(
                "Defaulting to first device with render node: {}",
                device.card
            ));
        }
        device.card.clone()
    });

    if render_card.is_none() {
        render_card = Some(scanout_card_str.clone());
        reasons.push("Defaulting render card to scanout card".to_owned());
    }

    let render_card_str = render_card.as_ref()?;
    let render_device = drm_devices.iter().find(|d| &d.card == render_card_str);
    let render_driver = render_device.and_then(|d| d.driver.clone());

    let is_cross_gpu = Some(render_card_str != scanout_card_str);

    let confidence =
        if connected_connector.is_some() && render_driver.is_some() && scanout_driver.is_some() {
            "high".to_owned()
        } else if render_card_str == scanout_card_str {
            "medium".to_owned()
        } else {
            "low".to_owned()
        };

    Some(DisplayPathGuess {
        render_card: Some(render_card_str.clone()),
        render_driver,
        scanout_card: Some(scanout_card_str.clone()),
        scanout_driver,
        connector: connected_connector,
        is_cross_gpu,
        confidence,
        reasons,
    })
}

fn preferred_render_device(drm_devices: &[DrmDeviceInfo]) -> Option<&DrmDeviceInfo> {
    drm_devices
        .iter()
        .find(|device| is_discrete_gpu(device))
        .or_else(|| {
            drm_devices
                .iter()
                .find(|device| device.render_node.is_some())
        })
}

fn is_discrete_gpu(device: &DrmDeviceInfo) -> bool {
    device
        .driver
        .as_deref()
        .is_some_and(|driver| matches!(driver, "amdgpu" | "nouveau" | "nvidia" | "radeon"))
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn read_trimmed_lines(path: impl AsRef<Path>) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| {
            value
                .lines()
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn stable_hash_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_sys_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-display-topology-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn link_driver(sys: &Path, card: &Path, driver: &str) {
        let driver_path = sys.join(format!("bus/pci/drivers/{driver}"));
        std::fs::create_dir_all(&driver_path).unwrap();
        std::os::unix::fs::symlink(&driver_path, card.join("device/driver")).ok();
    }

    #[test]
    fn test_topology_probing_and_guessing() {
        let root = temp_sys_root("intel-amd");
        let sys = root.join("sys");
        let proc = root.join("proc");

        // Intel iGPU: card0, scanout connected
        let card0 = sys.join("class/drm/card0");
        write(&card0.join("device/vendor"), "0x8086");
        write(&card0.join("device/device"), "0x9bc5");
        write(&card0.join("device/boot_vga"), "1");
        std::fs::create_dir_all(card0.join("device/drm/renderD128")).unwrap();
        write(&sys.join("class/drm/card0-HDMI-A-1/status"), "connected");
        write(&sys.join("class/drm/card0-HDMI-A-1/enabled"), "enabled");
        write(&sys.join("class/drm/card0-HDMI-A-1/modes"), "1920x1080\n");
        link_driver(&sys, &card0, "i915");

        // AMD dGPU: card1, render node only
        let card1 = sys.join("class/drm/card1");
        write(&card1.join("device/vendor"), "0x1002");
        write(&card1.join("device/device"), "0x73ff");
        std::fs::create_dir_all(card1.join("device/drm/renderD129")).unwrap();
        link_driver(&sys, &card1, "amdgpu");

        let topology = probe_display_topology_root(&proc, &sys);

        assert_eq!(topology.drm_devices.len(), 2);
        assert_eq!(topology.drm_devices[0].card, "card0");
        assert_eq!(topology.drm_devices[1].card, "card1");

        let guess = topology.guessed_path.unwrap();
        assert_eq!(guess.scanout_card.as_deref(), Some("card0"));
        assert_eq!(guess.render_card.as_deref(), Some("card1"));
        assert_eq!(guess.connector.as_deref(), Some("HDMI-A-1"));
        assert_eq!(guess.is_cross_gpu, Some(true));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn direct_amd_scanout_and_render_path_is_not_cross_gpu() {
        let root = temp_sys_root("direct-amd");
        let sys = root.join("sys");
        let proc = root.join("proc");
        let card0 = sys.join("class/drm/card0");
        write(&card0.join("device/vendor"), "0x1002");
        write(&card0.join("device/device"), "0x744c");
        std::fs::create_dir_all(card0.join("device/drm/renderD128")).unwrap();
        link_driver(&sys, &card0, "amdgpu");
        write(&sys.join("class/drm/card0-DP-1/status"), "connected");
        write(&sys.join("class/drm/card0-DP-1/enabled"), "enabled");
        write(&sys.join("class/drm/card0-DP-1/modes"), "2560x1440\n");

        let topology = probe_display_topology_root(&proc, &sys);
        let guess = topology.guessed_path.unwrap();

        assert_eq!(guess.scanout_card.as_deref(), Some("card0"));
        assert_eq!(guess.render_card.as_deref(), Some("card0"));
        assert_eq!(guess.connector.as_deref(), Some("DP-1"));
        assert_eq!(guess.is_cross_gpu, Some(false));
        assert_eq!(guess.scanout_driver.as_deref(), Some("amdgpu"));
        assert_eq!(guess.render_driver.as_deref(), Some("amdgpu"));

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn no_connected_connector_reports_unknown_path_with_warning() {
        let root = temp_sys_root("no-connector");
        let sys = root.join("sys");
        let proc = root.join("proc");
        let card0 = sys.join("class/drm/card0");
        write(&card0.join("device/vendor"), "0x8086");
        write(&card0.join("device/device"), "0x9bc5");
        std::fs::create_dir_all(card0.join("device/drm/renderD128")).unwrap();
        link_driver(&sys, &card0, "i915");
        write(&sys.join("class/drm/card0-HDMI-A-1/status"), "disconnected");
        write(&sys.join("class/drm/card0-HDMI-A-1/enabled"), "disabled");

        let topology = probe_display_topology_root(&proc, &sys);
        let guess = topology.guessed_path.unwrap();

        assert_eq!(guess.confidence, "unknown");
        assert_eq!(guess.scanout_card, None);
        assert_eq!(guess.connector, None);
        assert_eq!(guess.is_cross_gpu, None);
        assert!(
            topology
                .warnings
                .iter()
                .any(|warning| warning.contains("no connected connector found"))
        );

        std::fs::remove_dir_all(root).ok();
    }
}
