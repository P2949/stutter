use std::{collections::BTreeSet, fs, path::Path};

use crate::system_inventory::{DrmDeviceInventory, SystemInventory};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusGpuSource {
    ExplicitOverride,
    TargetProcessFd,
    GpuSample,
    HwmonSelection,
    SingleGpuFallback,
    Unresolved,
}

impl FocusGpuSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitOverride => "explicit_override",
            Self::TargetProcessFd => "target_process_fd",
            Self::GpuSample => "gpu_sample",
            Self::HwmonSelection => "hwmon_selection",
            Self::SingleGpuFallback => "single_gpu_fallback",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FocusGpuResolution {
    pub render_node: Option<String>,
    pub drm_card: Option<String>,
    pub pci_id: Option<String>,
    pub confidence: f32,
    pub source: FocusGpuSource,
}

impl FocusGpuResolution {
    fn unresolved() -> Self {
        Self {
            render_node: None,
            drm_card: None,
            pci_id: None,
            confidence: 0.0,
            source: FocusGpuSource::Unresolved,
        }
    }

    fn from_device(device: &DrmDeviceInventory, confidence: f32, source: FocusGpuSource) -> Self {
        Self {
            render_node: device.render_node.clone(),
            drm_card: Some(device.name.clone()),
            pci_id: device.pci_id.clone(),
            confidence,
            source,
        }
    }
}

pub struct FocusGpuResolver;

#[derive(Clone, Debug)]
pub struct FocusGpuResolverInput<'a> {
    pub proc_root: &'a Path,
    pub target_pids: &'a [u32],
    pub inventory: &'a SystemInventory,
    pub observed_render_node: Option<&'a str>,
    pub observed_drm_card: Option<&'a str>,
    pub explicit_render_node: Option<&'a str>,
    pub explicit_drm_card: Option<&'a str>,
}

impl FocusGpuResolver {
    pub fn resolve(input: FocusGpuResolverInput<'_>) -> FocusGpuResolution {
        if let Some(render_node) = input.explicit_render_node
            && let Some(device) = device_for_render_node(input.inventory, render_node)
        {
            return FocusGpuResolution::from_device(device, 1.0, FocusGpuSource::ExplicitOverride);
        }

        if let Some(drm_card) = input.explicit_drm_card
            && let Some(device) = device_for_drm_card(input.inventory, drm_card)
        {
            return FocusGpuResolution::from_device(device, 1.0, FocusGpuSource::ExplicitOverride);
        }

        if let Some(render_node) = focused_render_node_from_proc(input.proc_root, input.target_pids)
            && let Some(device) = device_for_render_node(input.inventory, &render_node)
        {
            return FocusGpuResolution::from_device(device, 0.95, FocusGpuSource::TargetProcessFd);
        }

        if let Some(render_node) = input.observed_render_node
            && let Some(device) = device_for_render_node(input.inventory, render_node)
        {
            return FocusGpuResolution::from_device(device, 0.85, FocusGpuSource::GpuSample);
        }

        if let Some(drm_card) = input.observed_drm_card
            && let Some(device) = device_for_drm_card(input.inventory, drm_card)
        {
            return FocusGpuResolution::from_device(device, 0.80, FocusGpuSource::HwmonSelection);
        }

        match input.inventory.drm_devices.as_slice() {
            [single] => {
                FocusGpuResolution::from_device(single, 0.60, FocusGpuSource::SingleGpuFallback)
            }
            _ => FocusGpuResolution::unresolved(),
        }
    }
}

fn focused_render_node_from_proc(proc_root: &Path, target_pids: &[u32]) -> Option<String> {
    let mut render_nodes = BTreeSet::new();

    for pid in target_pids {
        collect_render_nodes_from_fd_dir(
            &proc_root.join(pid.to_string()).join("fd"),
            &mut render_nodes,
        );
    }

    (render_nodes.len() == 1)
        .then(|| render_nodes.into_iter().next())
        .flatten()
}

fn collect_render_nodes_from_fd_dir(fd_dir: &Path, render_nodes: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(fd_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let Ok(target) = fs::read_link(entry.path()) else {
            continue;
        };
        if let Some(render_node) = render_node_name(&target) {
            render_nodes.insert(render_node);
        }
    }
}

fn render_node_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.starts_with("renderD"))
        .map(str::to_owned)
}

fn device_for_render_node<'a>(
    inventory: &'a SystemInventory,
    render_node: &str,
) -> Option<&'a DrmDeviceInventory> {
    let render_node =
        render_node_name(Path::new(render_node)).unwrap_or_else(|| render_node.to_owned());
    inventory
        .drm_devices
        .iter()
        .find(|device| device.render_node.as_deref() == Some(render_node.as_str()))
}

fn device_for_drm_card<'a>(
    inventory: &'a SystemInventory,
    drm_card: &str,
) -> Option<&'a DrmDeviceInventory> {
    inventory
        .drm_devices
        .iter()
        .find(|device| device.name == drm_card)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink, path::PathBuf};

    use super::*;
    use crate::system_inventory::SystemInventory;

    fn inventory(devices: &[(&str, &str)]) -> SystemInventory {
        SystemInventory {
            cpu_policies: Vec::new(),
            drm_devices: devices
                .iter()
                .map(|(card, render)| DrmDeviceInventory {
                    name: (*card).to_owned(),
                    path: PathBuf::from(format!("/fake/sys/class/drm/{card}")),
                    render_node: Some((*render).to_owned()),
                    pci_id: Some(format!("pci-{card}")),
                    vendor: Some("test".to_owned()),
                    hwmon_paths: Vec::new(),
                })
                .collect(),
            irq_default_smp_affinity: None,
            irq_lines: Vec::new(),
            power_source: Default::default(),
            sched_ext_available: false,
            vm_knobs: Default::default(),
            inventory_hash: "gpu-focus-test".to_owned(),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("stutter-gpu-focus-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn input<'a>(
        proc_root: &'a Path,
        pids: &'a [u32],
        inventory: &'a SystemInventory,
    ) -> FocusGpuResolverInput<'a> {
        FocusGpuResolverInput {
            proc_root,
            target_pids: pids,
            inventory,
            observed_render_node: None,
            observed_drm_card: None,
            explicit_render_node: None,
            explicit_drm_card: None,
        }
    }

    #[test]
    fn single_gpu_fallback_resolves_with_low_confidence() {
        let proc_root = temp_dir("single");
        let inventory = inventory(&[("card0", "renderD128")]);
        let result = FocusGpuResolver::resolve(input(&proc_root, &[], &inventory));

        assert_eq!(result.render_node.as_deref(), Some("renderD128"));
        assert_eq!(result.drm_card.as_deref(), Some("card0"));
        assert_eq!(result.source, FocusGpuSource::SingleGpuFallback);
        assert!(result.confidence < 0.70);
    }

    #[test]
    fn target_process_fd_resolves_focused_render_node() {
        let proc_root = temp_dir("fd");
        let fd_dir = proc_root.join("1234/fd");
        fs::create_dir_all(&fd_dir).unwrap();
        symlink("/dev/dri/renderD129", fd_dir.join("7")).unwrap();
        let inventory = inventory(&[("card0", "renderD128"), ("card1", "renderD129")]);
        let result = FocusGpuResolver::resolve(input(&proc_root, &[1234], &inventory));

        assert_eq!(result.render_node.as_deref(), Some("renderD129"));
        assert_eq!(result.drm_card.as_deref(), Some("card1"));
        assert_eq!(result.source, FocusGpuSource::TargetProcessFd);
        assert!(result.confidence >= 0.90);
    }

    #[test]
    fn two_gpus_without_focus_signal_are_unresolved() {
        let proc_root = temp_dir("ambiguous");
        let inventory = inventory(&[("card0", "renderD128"), ("card1", "renderD129")]);
        let result = FocusGpuResolver::resolve(input(&proc_root, &[], &inventory));

        assert_eq!(result.source, FocusGpuSource::Unresolved);
        assert_eq!(result.render_node, None);
    }

    #[test]
    fn explicit_override_wins_over_ambiguous_fd_state() {
        let proc_root = temp_dir("explicit");
        let inventory = inventory(&[("card0", "renderD128"), ("card1", "renderD129")]);
        let result = FocusGpuResolver::resolve(FocusGpuResolverInput {
            explicit_render_node: Some("/dev/dri/renderD129"),
            ..input(&proc_root, &[], &inventory)
        });

        assert_eq!(result.render_node.as_deref(), Some("renderD129"));
        assert_eq!(result.drm_card.as_deref(), Some("card1"));
        assert_eq!(result.source, FocusGpuSource::ExplicitOverride);
        assert_eq!(result.confidence, 1.0);
    }
}
