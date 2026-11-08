use std::{fs, os::unix::fs::symlink};

use super::*;

fn fixture(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("stutter-gpu-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn card(root: &std::path::Path, name: &str, slot: &str, files: &[(&str, &str)]) {
    let device = root.join("pci").join(slot);
    fs::create_dir_all(&device).unwrap();
    for (file, text) in files {
        fs::write(device.join(file), text).unwrap();
    }
    let drm = root.join("drm");
    fs::create_dir_all(drm.join(name)).unwrap();
    symlink(&device, drm.join(name).join("device")).unwrap();
}

#[test]
fn a_card_is_read_exactly_and_only_card_nodes_count() {
    let root = fixture("exact");
    card(
        &root,
        "card0",
        "0000:03:00.0",
        &[
            ("vendor", "0x1002\n"),
            ("device", "0x73bf\n"),
            ("subsystem_vendor", "0x1002\n"),
            ("numa_node", "-1\n"),
            ("power_dpm_force_performance_level", "auto\n"),
            (
                "pp_power_profile_mode",
                "NUM        MODE_NAME\n  0 3D_FULL_SCREEN *\n  1   POWER_SAVING\n",
            ),
        ],
    );
    fs::create_dir_all(root.join("drm/card0-DP-1")).unwrap();
    fs::create_dir_all(root.join("drm/renderD128")).unwrap();
    let gpus = observe_gpus(&root.join("drm"));
    assert_eq!(gpus.len(), 1);
    let gpu = &gpus[0];
    assert_eq!(gpu.pci_slot, "0000:03:00.0");
    assert_eq!(gpu.vendor_id.as_deref(), Some("0x1002"));
    assert_eq!(gpu.device_id.as_deref(), Some("0x73bf"));
    assert_eq!(gpu.subsystem_device_id, None);
    assert_eq!(gpu.numa_node, Some(-1));
    assert_eq!(gpu.dpm_force_level.as_deref(), Some("auto"));
    assert_eq!(gpu.active_profile.as_deref(), Some("0 3D_FULL_SCREEN"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn two_gpus_stay_distinct_and_a_missing_root_is_empty_not_an_error() {
    let root = fixture("multi");
    card(&root, "card1", "0000:0a:00.0", &[("vendor", "0x10de\n")]);
    card(&root, "card0", "0000:03:00.0", &[("vendor", "0x1002\n")]);
    let gpus = observe_gpus(&root.join("drm"));
    assert_eq!(
        gpus.iter().map(|g| g.pci_slot.as_str()).collect::<Vec<_>>(),
        ["0000:03:00.0", "0000:0a:00.0"]
    );
    assert_ne!(gpus[0].vendor_id, gpus[1].vendor_id);
    assert!(observe_gpus(&root.join("absent")).is_empty());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_profile_without_an_active_marker_is_unknown_never_guessed() {
    assert_eq!(active_profile("NUM MODE\n 0 A\n 1 B\n"), None);
    assert_eq!(active_profile(" 1 B *\n").as_deref(), Some("1 B"));
}

#[test]
fn the_real_host_read_never_panics_and_reports_only_what_sysfs_says() {
    for gpu in observe_gpus(std::path::Path::new("/sys/class/drm")) {
        assert!(gpu.card.starts_with("card"));
        assert!(gpu.pci_slot.contains(':'), "{gpu:?}");
        if let Some(vendor) = &gpu.vendor_id {
            assert!(vendor.starts_with("0x"));
        }
    }
}
