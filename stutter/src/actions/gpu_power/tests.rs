//! Tests extracted from the parent module to keep production files below the architecture size gate.

use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

fn temp_sysfs_root(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-gpu-power-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_gpu_device(root: &Path, card: &str, dpm: &str, profile: &str) -> PathBuf {
    let device = root.join("class/drm").join(card).join("device");
    fs::create_dir_all(&device).unwrap();
    fs::write(
        device.join("power_dpm_force_performance_level"),
        format!("{dpm}\n"),
    )
    .unwrap();
    fs::write(device.join("pp_power_profile_mode"), format!("{profile}\n")).unwrap();
    device
}

fn write_ac_power(root: &Path, online: bool) {
    let ac = root.join("class/power_supply/AC");
    fs::create_dir_all(&ac).unwrap();
    fs::write(ac.join("type"), "Mains\n").unwrap();
    fs::write(ac.join("online"), if online { "1\n" } else { "0\n" }).unwrap();
}

fn write_battery(root: &Path, status: &str) {
    let battery = root.join("class/power_supply/BAT0");
    fs::create_dir_all(&battery).unwrap();
    fs::write(battery.join("type"), "Battery\n").unwrap();
    fs::write(battery.join("status"), format!("{status}\n")).unwrap();
}

fn policy_for(cards: &[&str]) -> GpuPowerPolicy {
    GpuPowerPolicy {
        allow_gpu_power_changes: true,
        mode: GpuPowerMode::SuggestOnly,
        allowed_drm_cards: cards
            .iter()
            .map(|card| (*card).to_owned())
            .collect::<BTreeSet<_>>(),
        allow_force_performance_level: true,
        allow_power_profile_mode: true,
        battery_policy: GpuBatteryPolicy::Never,
        explicit_battery_override: false,
    }
}

fn manual_policy_for(cards: &[&str]) -> GpuPowerPolicy {
    GpuPowerPolicy {
        mode: GpuPowerMode::ManualApply,
        ..policy_for(cards)
    }
}

fn action_for(root: &Path) -> GpuPowerAction {
    GpuPowerAction {
        sysfs_root: root.to_path_buf(),
        drm_card: "card0".to_owned(),
        power_dpm_force_performance_level: Some("high".to_owned()),
        pp_power_profile_mode: Some("3D_FULL_SCREEN".to_owned()),
    }
}

#[test]
fn safety_class_is_high_risk_when_dpm_level_changes() {
    let root = temp_sysfs_root("safety");
    let action = action_for(&root);

    assert_eq!(action.safety_class(), SafetyClass::HighRisk);

    fs::remove_dir_all(root).ok();
}

#[test]
fn profile_mode_only_is_reversible_medium_risk() {
    let root = temp_sysfs_root("profile-medium-risk");
    let mut action = action_for(&root);
    action.power_dpm_force_performance_level = None;
    action.pp_power_profile_mode = Some("3D_FULL_SCREEN".to_owned());

    assert_eq!(action.safety_class(), SafetyClass::ReversibleMediumRisk);

    fs::remove_dir_all(root).ok();
}

#[test]
fn action_id_and_description_include_card_and_requested_values() {
    let root = temp_sysfs_root("id");
    let action = action_for(&root);

    assert_eq!(
        action.id(),
        ActionId::new("gpu-power:set:card=card0:dpm=high:profile=3D_FULL_SCREEN".to_owned())
    );
    assert_eq!(
        action.describe(),
        "set GPU power values for card0: power_dpm_force_performance_level=high, pp_power_profile_mode=3D_FULL_SCREEN"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_default_disabled_policy() {
    let root = temp_sysfs_root("default-disabled");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");

    let err = action_for(&root).preflight().unwrap_err().to_string();

    assert!(err.contains("policy does not allow GPU power/performance profile changes"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_warns_that_action_is_high_risk_and_suggest_only() {
    let root = temp_sysfs_root("suggest-only");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");

    let warnings = action_for(&root)
        .preflight_at(&policy_for(&["card0"]))
        .unwrap();

    assert!(warnings.iter().any(|warning| {
        warning
            .message
            .contains("high risk and should remain manual/suggest-only")
    }));
    assert!(warnings.iter().any(|warning| {
        warning
            .message
            .contains("policy is suggest-only; no writes will be performed")
    }));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_missing_knobs() {
    let root = temp_sysfs_root("missing-knobs");
    let action = GpuPowerAction {
        sysfs_root: root.clone(),
        drm_card: "card0".to_owned(),
        power_dpm_force_performance_level: None,
        pp_power_profile_mode: None,
    };

    let err = action
        .preflight_at(&policy_for(&["card0"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires power_dpm_force_performance_level"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_empty_allowlist() {
    let root = temp_sysfs_root("empty-allowlist");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");
    let mut policy = policy_for(&[]);
    policy.allowed_drm_cards.clear();

    let err = action_for(&root)
        .preflight_at(&policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires a non-empty explicit DRM card allowlist"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_card_not_in_allowlist() {
    let root = temp_sysfs_root("card-not-allowed");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");

    let err = action_for(&root)
        .preflight_at(&policy_for(&["card1"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("DRM card card0 is not in explicit GPU power allowlist"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_invalid_drm_card_name() {
    let root = temp_sysfs_root("bad-card");
    let mut action = action_for(&root);
    action.drm_card = "../card0".to_owned();

    let err = action
        .preflight_at(&policy_for(&["../card0"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("DRM card name must start with card"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_dpm_when_not_allowed() {
    let root = temp_sysfs_root("dpm-not-allowed");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");
    let mut policy = policy_for(&["card0"]);
    policy.allow_force_performance_level = false;

    let err = action_for(&root)
        .preflight_at(&policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow power_dpm_force_performance_level changes"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_profile_when_not_allowed() {
    let root = temp_sysfs_root("profile-not-allowed");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");
    let mut policy = policy_for(&["card0"]);
    policy.allow_power_profile_mode = false;

    let err = action_for(&root)
        .preflight_at(&policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow pp_power_profile_mode changes"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_invalid_dpm_value() {
    let root = temp_sysfs_root("invalid-dpm");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");
    let mut action = action_for(&root);
    action.power_dpm_force_performance_level = Some("turbo;bad".to_owned());

    let err = action
        .preflight_at(&policy_for(&["card0"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("power_dpm_force_performance_level value contains invalid character"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_unsupported_dpm_value() {
    let root = temp_sysfs_root("unsupported-dpm");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");
    let mut action = action_for(&root);
    action.power_dpm_force_performance_level = Some("turbo".to_owned());

    let err = action
        .preflight_at(&policy_for(&["card0"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("unsupported power_dpm_force_performance_level value"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_missing_gpu_sysfs_file() {
    let root = temp_sysfs_root("missing-file");
    fs::create_dir_all(root.join("class/drm/card0/device")).unwrap();

    let err = action_for(&root)
        .preflight_at(&policy_for(&["card0"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("required GPU power sysfs file does not exist"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_battery_without_override() {
    let root = temp_sysfs_root("battery-no-override");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");
    write_ac_power(&root, false);
    write_battery(&root, "Discharging");

    let err = action_for(&root)
        .preflight_at(&policy_for(&["card0"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("refusing GPU power/performance profile changes while on battery"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_allows_battery_only_with_explicit_override() {
    let root = temp_sysfs_root("battery-override");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");
    write_ac_power(&root, false);
    write_battery(&root, "Discharging");

    let mut policy = policy_for(&["card0"]);
    policy.battery_policy = GpuBatteryPolicy::AllowWithExplicitConfig;
    policy.explicit_battery_override = true;

    action_for(&root).preflight_at(&policy).unwrap();

    fs::remove_dir_all(root).ok();
}

#[test]
fn dry_run_counts_pending_changes_without_mutating() {
    let root = temp_sysfs_root("dry-run");
    let device = write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");

    let state = action_for(&root)
        .dry_run_at(&policy_for(&["card0"]))
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 2);
    assert_eq!(state.affected_tasks, 2);
    assert_eq!(state.pending_changes, 2);
    assert_eq!(
        read_trimmed(&device.join("power_dpm_force_performance_level")).unwrap(),
        "auto"
    );
    assert_eq!(
        read_trimmed(&device.join("pp_power_profile_mode")).unwrap(),
        "BOOTUP_DEFAULT"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn apply_with_manual_policy_writes_verifies_and_saves_every_original_value() {
    let root = temp_sysfs_root("apply");
    let device = write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");

    let token = action_for(&root)
        .apply_with_policy(&manual_policy_for(&["card0"]))
        .unwrap();

    assert_eq!(
        read_trimmed(&device.join("power_dpm_force_performance_level")).unwrap(),
        "high"
    );
    assert_eq!(
        read_trimmed(&device.join("pp_power_profile_mode")).unwrap(),
        "3D_FULL_SCREEN"
    );

    match token {
        RollbackToken::GpuPowerRestore { records } => {
            assert_eq!(records.len(), 2);
            assert!(records.iter().any(|record| {
                record.path.ends_with("power_dpm_force_performance_level")
                    && record.original_value == "auto"
            }));
            assert!(records.iter().any(|record| {
                record.path.ends_with("pp_power_profile_mode")
                    && record.original_value == "BOOTUP_DEFAULT"
            }));
        }
        other => panic!("unexpected rollback token: {other:?}"),
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn apply_rejects_suggest_only_policy() {
    let root = temp_sysfs_root("apply-suggest-only");
    write_gpu_device(&root, "card0", "auto", "BOOTUP_DEFAULT");

    let err = action_for(&root)
        .apply_with_policy(&policy_for(&["card0"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("GPU power action is suggest-only"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn rollback_restores_all_values_and_verifies() {
    let root = temp_sysfs_root("rollback");
    let device = write_gpu_device(&root, "card0", "high", "3D_FULL_SCREEN");
    let action = action_for(&root);

    let token = RollbackToken::GpuPowerRestore {
        records: vec![
            GpuPowerRestoreRecord {
                path: device.join("power_dpm_force_performance_level"),
                original_value: "auto".to_owned(),
            },
            GpuPowerRestoreRecord {
                path: device.join("pp_power_profile_mode"),
                original_value: "BOOTUP_DEFAULT".to_owned(),
            },
        ],
    };

    action.rollback(&token).unwrap();

    assert_eq!(
        read_trimmed(&device.join("power_dpm_force_performance_level")).unwrap(),
        "auto"
    );
    assert_eq!(
        read_trimmed(&device.join("pp_power_profile_mode")).unwrap(),
        "BOOTUP_DEFAULT"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn rollback_rejects_wrong_token_kind() {
    let root = temp_sysfs_root("wrong-token");
    let action = action_for(&root);

    let err = action
        .rollback(&RollbackToken::NiceRestore {
            records: Vec::new(),
        })
        .unwrap_err()
        .to_string();

    assert!(err.contains("invalid rollback token"));
    assert!(err.contains("expected gpu-power-restore"));
    assert!(err.contains("actual nice-restore"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn gpu_power_restore_token_reports_affected_records_and_no_restore_path() {
    let token = RollbackToken::GpuPowerRestore {
        records: vec![
            GpuPowerRestoreRecord {
                path: PathBuf::from(
                    "/sys/class/drm/card0/device/power_dpm_force_performance_level",
                ),
                original_value: "auto".to_owned(),
            },
            GpuPowerRestoreRecord {
                path: PathBuf::from("/sys/class/drm/card0/device/pp_power_profile_mode"),
                original_value: "BOOTUP_DEFAULT".to_owned(),
            },
        ],
    };

    assert_eq!(token.affected_tasks(), 2);
    assert!(token.restore_path().is_none());
}
