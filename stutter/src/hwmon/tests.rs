use std::{fs, path::PathBuf};

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
