use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::irq_inspect::{IrqLine, parse_proc_interrupts};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemInventory {
    pub cpu_policies: Vec<CpuPolicyInventory>,
    pub drm_devices: Vec<DrmDeviceInventory>,
    pub irq_default_smp_affinity: Option<String>,
    pub irq_lines: Vec<IrqLine>,
    pub power_source: PowerSourceSnapshot,
    pub sched_ext_available: bool,
    pub vm_knobs: BTreeMap<String, String>,
    pub inventory_hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpuPolicyInventory {
    pub policy: String,
    pub path: PathBuf,
    pub scaling_governor: Option<String>,
    pub available_governors: Option<String>,
    pub energy_performance_preference: Option<String>,
    pub energy_performance_available_preferences: Option<String>,
    pub related_cpus: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DrmDeviceInventory {
    pub name: String,
    pub path: PathBuf,
    pub render_node: Option<String>,
    pub pci_id: Option<String>,
    pub vendor: Option<String>,
    pub hwmon_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PowerSourceSnapshot {
    pub ac_online: Option<bool>,
    pub battery_present: bool,
    pub battery_discharging: Option<bool>,
}

impl PowerSourceSnapshot {
    pub fn on_battery_or_discharging(&self) -> bool {
        self.battery_discharging == Some(true)
            || (self.battery_present && self.ac_online == Some(false))
    }

    pub fn ac_power_for_evidence(&self) -> Option<bool> {
        self.ac_online
            .or_else(|| (!self.battery_present).then_some(true))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInventoryRoot {
    pub proc_root: PathBuf,
    pub sys_root: PathBuf,
}

impl Default for SystemInventoryRoot {
    fn default() -> Self {
        Self {
            proc_root: PathBuf::from("/proc"),
            sys_root: PathBuf::from("/sys"),
        }
    }
}

impl SystemInventory {
    pub fn probe() -> Self {
        Self::probe_root(&SystemInventoryRoot::default())
    }

    pub fn probe_root(root: &SystemInventoryRoot) -> Self {
        let cpu_policies = probe_cpu_policies(&root.sys_root);
        let drm_devices = probe_drm_devices(&root.sys_root);
        let irq_default_smp_affinity =
            read_trimmed(root.proc_root.join("irq/default_smp_affinity"));
        let irq_lines = probe_irq_lines(&root.proc_root);
        let power_source = probe_power_source(&root.sys_root);
        let sched_ext_available = root.sys_root.join("kernel/sched_ext").exists();
        let vm_knobs = probe_vm_knobs(&root.proc_root);
        let inventory_hash = inventory_hash(
            &cpu_policies,
            &drm_devices,
            irq_default_smp_affinity.as_deref(),
            &irq_lines,
            &power_source,
            sched_ext_available,
            &vm_knobs,
        );

        Self {
            cpu_policies,
            drm_devices,
            irq_default_smp_affinity,
            irq_lines,
            power_source,
            sched_ext_available,
            vm_knobs,
            inventory_hash,
        }
    }
}

fn probe_cpu_policies(sys_root: &Path) -> Vec<CpuPolicyInventory> {
    let cpufreq_root = sys_root.join("devices/system/cpu/cpufreq");
    let Ok(entries) = std::fs::read_dir(cpufreq_root) else {
        return Vec::new();
    };

    let mut policies = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let policy = entry.file_name().to_string_lossy().into_owned();
            if !policy.starts_with("policy") || !path.is_dir() {
                return None;
            }
            Some(CpuPolicyInventory {
                policy,
                scaling_governor: read_trimmed(path.join("scaling_governor")),
                available_governors: read_trimmed(path.join("scaling_available_governors")),
                energy_performance_preference: read_trimmed(
                    path.join("energy_performance_preference"),
                ),
                energy_performance_available_preferences: read_trimmed(
                    path.join("energy_performance_available_preferences"),
                ),
                related_cpus: read_trimmed(path.join("related_cpus")),
                path,
            })
        })
        .collect::<Vec<_>>();

    policies.sort_by(|left, right| left.policy.cmp(&right.policy));
    policies
}

fn probe_drm_devices(sys_root: &Path) -> Vec<DrmDeviceInventory> {
    let drm_root = sys_root.join("class/drm");
    let Ok(entries) = std::fs::read_dir(drm_root) else {
        return Vec::new();
    };

    let mut devices = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("card") || name.contains('-') || !path.exists() {
                return None;
            }

            let device_path = path.join("device");
            let vendor_id = read_trimmed(device_path.join("vendor"));
            let device_id = read_trimmed(device_path.join("device"));
            let pci_id = match (vendor_id.as_deref(), device_id.as_deref()) {
                (Some(vendor), Some(device)) => Some(format!(
                    "{}:{}",
                    trim_hex_prefix(vendor),
                    trim_hex_prefix(device)
                )),
                _ => None,
            };
            let vendor = vendor_id.as_deref().map(gpu_vendor_name);

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

            Some(DrmDeviceInventory {
                name,
                path,
                render_node,
                pci_id,
                vendor,
                hwmon_paths,
            })
        })
        .collect::<Vec<_>>();

    devices.sort_by(|left, right| left.name.cmp(&right.name));
    devices
}

fn probe_irq_lines(proc_root: &Path) -> Vec<IrqLine> {
    let Some(contents) = std::fs::read_to_string(proc_root.join("interrupts")).ok() else {
        return Vec::new();
    };

    parse_proc_interrupts(&contents).unwrap_or_default()
}

fn probe_power_source(sys_root: &Path) -> PowerSourceSnapshot {
    let power_root = sys_root.join("class/power_supply");
    let Ok(entries) = std::fs::read_dir(power_root) else {
        return PowerSourceSnapshot::default();
    };

    let mut ac_online = None;
    let mut saw_ac_offline = false;
    let mut battery_present = false;
    let mut saw_non_discharging_battery = false;
    let mut battery_discharging = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let supply_type = read_trimmed(path.join("type"))
            .unwrap_or_default()
            .to_ascii_lowercase();

        if is_ac_power_supply(&supply_type) {
            match read_boolish(path.join("online")) {
                Some(true) => ac_online = Some(true),
                Some(false) if ac_online != Some(true) => {
                    saw_ac_offline = true;
                    ac_online = Some(false);
                }
                _ => {}
            }
        } else if supply_type == "battery" {
            battery_present = true;
            match read_trimmed(path.join("status"))
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "discharging" => battery_discharging = Some(true),
                "charging" | "full" | "not charging" | "unknown"
                    if battery_discharging != Some(true) =>
                {
                    saw_non_discharging_battery = true;
                    battery_discharging = Some(false);
                }
                _ => {}
            }
        }
    }

    if ac_online.is_none() && saw_ac_offline {
        ac_online = Some(false);
    }
    if battery_discharging.is_none() && saw_non_discharging_battery {
        battery_discharging = Some(false);
    }

    PowerSourceSnapshot {
        ac_online,
        battery_present,
        battery_discharging,
    }
}

fn is_ac_power_supply(supply_type: &str) -> bool {
    matches!(
        supply_type,
        "mains" | "ac" | "usb" | "usb_c" | "usb-c" | "usb_pd" | "wireless"
    )
}

fn read_boolish(path: impl AsRef<Path>) -> Option<bool> {
    match read_trimmed(path)?.to_ascii_lowercase().as_str() {
        "1" | "true" | "online" | "yes" => Some(true),
        "0" | "false" | "offline" | "no" => Some(false),
        _ => None,
    }
}

fn trim_hex_prefix(value: &str) -> String {
    value.trim().trim_start_matches("0x").to_ascii_lowercase()
}

fn gpu_vendor_name(vendor_id: &str) -> String {
    match trim_hex_prefix(vendor_id).as_str() {
        "1002" => "amd".to_owned(),
        "10de" => "nvidia".to_owned(),
        "8086" => "intel".to_owned(),
        other => other.to_owned(),
    }
}

fn probe_vm_knobs(proc_root: &Path) -> BTreeMap<String, String> {
    let mut knobs = BTreeMap::new();
    for relative in [
        "sys/vm/swappiness",
        "sys/vm/dirty_ratio",
        "sys/vm/dirty_background_ratio",
        "sys/vm/dirty_bytes",
        "sys/vm/dirty_background_bytes",
    ] {
        if let Some(value) = read_trimmed(proc_root.join(relative)) {
            knobs.insert(relative.to_owned(), value);
        }
    }
    knobs
}

fn inventory_hash(
    cpu_policies: &[CpuPolicyInventory],
    drm_devices: &[DrmDeviceInventory],
    irq_default_smp_affinity: Option<&str>,
    irq_lines: &[IrqLine],
    power_source: &PowerSourceSnapshot,
    sched_ext_available: bool,
    vm_knobs: &BTreeMap<String, String>,
) -> String {
    let mut parts = Vec::new();
    for policy in cpu_policies {
        parts.push(format!(
            "cpu:{}:{}:{}:{}",
            policy.policy,
            policy.related_cpus.as_deref().unwrap_or(""),
            policy.available_governors.as_deref().unwrap_or(""),
            policy
                .energy_performance_available_preferences
                .as_deref()
                .unwrap_or("")
        ));
    }
    for drm in drm_devices {
        parts.push(format!(
            "drm:{}:{}:{}:{}:{}",
            drm.name,
            drm.render_node.as_deref().unwrap_or(""),
            drm.pci_id.as_deref().unwrap_or(""),
            drm.vendor.as_deref().unwrap_or(""),
            drm.hwmon_paths.len()
        ));
    }
    parts.push(format!(
        "irq_default:{}",
        irq_default_smp_affinity.unwrap_or("unknown")
    ));
    for irq in irq_lines {
        parts.push(format!("irq:{}:{}:{}", irq.irq, irq.total, irq.name));
    }
    parts.push(format!(
        "power:ac={:?}:battery_present={}:battery_discharging={:?}",
        power_source.ac_online, power_source.battery_present, power_source.battery_discharging
    ));
    parts.push(format!("sched_ext:{sched_ext_available}"));
    for (key, value) in vm_knobs {
        parts.push(format!("vm:{key}={value}"));
    }

    crate::daemon::state::daemon_profile_stable_hash(parts.iter().map(String::as_str))
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-system-inventory-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn fake_proc_inventory_parses_irq_lines() {
        let root = temp_root("irq-lines");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        write(
            &proc_root.join("interrupts"),
            r#"
           CPU0       CPU1
146:        10         20   PCI-MSI 524288-edge amdgpu
"#,
        );

        let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root,
            sys_root,
        });

        assert_eq!(inventory.irq_lines.len(), 1);
        assert_eq!(inventory.irq_lines[0].irq, "146");
        assert_eq!(inventory.irq_lines[0].name, "524288-edge amdgpu");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn fake_sysfs_inventory_finds_cpu_policies_and_hash_changes() {
        let root = temp_root("cpu");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        let policy0 = sys_root.join("devices/system/cpu/cpufreq/policy0");
        write(&policy0.join("scaling_governor"), "schedutil\n");
        write(
            &policy0.join("scaling_available_governors"),
            "schedutil performance powersave\n",
        );
        write(&policy0.join("related_cpus"), "0 1\n");
        write(&proc_root.join("irq/default_smp_affinity"), "ff\n");

        let first = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: proc_root.clone(),
            sys_root: sys_root.clone(),
        });
        write(&policy0.join("related_cpus"), "0 1 2\n");
        let second = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root,
            sys_root,
        });

        assert_eq!(first.cpu_policies.len(), 1);
        assert_eq!(first.cpu_policies[0].policy, "policy0");
        assert_ne!(first.inventory_hash, second.inventory_hash);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn fake_drm_inventory_maps_card_render_and_hwmon() {
        let root = temp_root("drm");
        let sys_root = root.join("sys");
        let card = sys_root.join("class/drm/card0");
        fs::create_dir_all(card.join("device/drm/renderD128")).unwrap();
        fs::create_dir_all(card.join("device/hwmon/hwmon0")).unwrap();
        write(&card.join("device/vendor"), "0x1002\n");
        write(&card.join("device/device"), "0x744c\n");

        let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: root.join("proc"),
            sys_root,
        });

        assert_eq!(inventory.drm_devices.len(), 1);
        assert_eq!(inventory.drm_devices[0].name, "card0");
        assert_eq!(
            inventory.drm_devices[0].render_node.as_deref(),
            Some("renderD128")
        );
        assert_eq!(
            inventory.drm_devices[0].pci_id.as_deref(),
            Some("1002:744c")
        );
        assert_eq!(inventory.drm_devices[0].vendor.as_deref(), Some("amd"));
        assert_eq!(inventory.drm_devices[0].hwmon_paths.len(), 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn fake_power_supply_inventory_reads_ac_and_battery_status() {
        let root = temp_root("power");
        let sys_root = root.join("sys");
        let ac = sys_root.join("class/power_supply/AC0");
        let battery = sys_root.join("class/power_supply/BAT0");
        write(&ac.join("type"), "Mains\n");
        write(&ac.join("online"), "1\n");
        write(&battery.join("type"), "Battery\n");
        write(&battery.join("status"), "Discharging\n");

        let first = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: root.join("proc"),
            sys_root: sys_root.clone(),
        });
        write(&battery.join("status"), "Charging\n");
        let second = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: root.join("proc"),
            sys_root,
        });

        assert_eq!(first.power_source.ac_online, Some(true));
        assert!(first.power_source.battery_present);
        assert_eq!(first.power_source.battery_discharging, Some(true));
        assert_eq!(second.power_source.battery_discharging, Some(false));
        assert_ne!(first.inventory_hash, second.inventory_hash);
        fs::remove_dir_all(root).ok();
    }
}
