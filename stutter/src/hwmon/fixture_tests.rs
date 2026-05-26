use std::path::PathBuf;

fn hwmon_fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("hwmon")
        .join(name)
}

use super::*;

#[test]
fn hwmon_amd_gpu_fixture_is_read_correctly() {
    let root = hwmon_fixture_path("amd_gpu");
    let mut reader =
        HwmonReader::from_hwmon_root_with_identity(root.join("device/hwmon/hwmon0"), None, None);
    let sample = reader.sample(10);
    assert_eq!(sample.gpu_busy_percent, Some(97));
    assert_eq!(sample.gpu_clock_mhz, Some(2200));
    assert_eq!(sample.mem_clock_mhz, Some(1000));
    assert_eq!(sample.temp_millidegrees, Some(61000));
    assert_eq!(sample.power_microwatts, Some(120000000));
}

#[test]
fn hwmon_intel_cpu_fixture_is_read_correctly() {
    let root = hwmon_fixture_path("intel_cpu");
    // Intel usually has freq in gt0 and other things in hwmon
    let mut reader =
        HwmonReader::from_hwmon_root_with_identity(root.join("device/hwmon/hwmon1"), None, None);
    let sample = reader.sample(10);
    assert_eq!(sample.gpu_clock_mhz, Some(1200));
    assert_eq!(sample.temp_millidegrees, Some(45000));
    assert_eq!(sample.power_microwatts, Some(35000000));
}

#[test]
fn hwmon_missing_labels_fixture_returns_no_data_without_panic() {
    let root = hwmon_fixture_path("missing_labels");
    let mut reader =
        HwmonReader::from_hwmon_root_with_identity(root.join("device/hwmon/hwmon2"), None, None);
    let sample = reader.sample(10);
    assert_eq!(sample.gpu_busy_percent, None);
    assert_eq!(sample.temp_millidegrees, None);
}

#[test]
fn hwmon_malformed_numbers_fixture_ignores_bad_data() {
    let root = hwmon_fixture_path("malformed_numbers");
    let mut reader =
        HwmonReader::from_hwmon_root_with_identity(root.join("device/hwmon/hwmon3"), None, None);
    let sample = reader.sample(10);
    assert_eq!(sample.gpu_busy_percent, None);
    assert_eq!(sample.temp_millidegrees, None);
}

#[test]
fn hwmon_permission_denied_fixture_ignores_unreadable_files() {
    let root = hwmon_fixture_path("permission_denied");
    let mut reader =
        HwmonReader::from_hwmon_root_with_identity(root.join("device/hwmon/hwmon4"), None, None);
    let sample = reader.sample(10);
    assert_eq!(sample.gpu_busy_percent, None);
}
