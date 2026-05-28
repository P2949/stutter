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
        "stutter-cpu-power-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_cpu_cpufreq(root: &Path, cpu: u32, governor: &str, epp: &str) -> PathBuf {
    let cpufreq = root
        .join("devices/system/cpu")
        .join(format!("cpu{cpu}"))
        .join("cpufreq");
    fs::create_dir_all(&cpufreq).unwrap();
    fs::write(cpufreq.join("scaling_governor"), format!("{governor}\n")).unwrap();
    fs::write(
        cpufreq.join("energy_performance_preference"),
        format!("{epp}\n"),
    )
    .unwrap();
    cpufreq
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

fn policy_for(cpus: &[u32]) -> CpuPowerPolicy {
    CpuPowerPolicy {
        allow_cpu_power_changes: true,
        allowed_cpus: cpus.iter().copied().collect::<BTreeSet<_>>(),
        allow_governor_changes: true,
        allow_epp_changes: true,
        battery_policy: BatteryPolicy::Never,
        explicit_battery_override: false,
    }
}

fn action_for(root: &Path) -> CpuPowerAction {
    CpuPowerAction {
        sysfs_root: root.to_path_buf(),
        cpus: vec![0, 1],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: Some("performance".to_owned()),
    }
}

#[test]
fn safety_class_is_high_risk_when_governor_changes() {
    let root = temp_sysfs_root("safety");
    let action = action_for(&root);

    assert_eq!(action.safety_class(), SafetyClass::HighRisk);

    fs::remove_dir_all(root).ok();
}

#[test]
fn epp_only_is_reversible_medium_risk() {
    let root = temp_sysfs_root("epp-medium-risk");
    let mut action = action_for(&root);
    action.scaling_governor = None;
    action.energy_performance_preference = Some("performance".to_owned());

    assert_eq!(action.safety_class(), SafetyClass::ReversibleMediumRisk);

    fs::remove_dir_all(root).ok();
}

#[test]
fn action_id_and_description_include_cpu_governor_and_epp() {
    let root = temp_sysfs_root("id");
    let action = action_for(&root);

    assert_eq!(
        action.id(),
        ActionId::new("cpu-power:set:cpus=0,1:governor=performance:epp=performance".to_owned())
    );
    assert_eq!(
        action.describe(),
        "set CPU power values for CPU(s) [0,1]: scaling_governor=performance, energy_performance_preference=performance"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_default_disabled_policy() {
    let root = temp_sysfs_root("default-disabled");
    write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");

    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };

    let err = action.preflight().unwrap_err().to_string();

    assert!(err.contains("action_policy_denied"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_empty_cpu_list() {
    let root = temp_sysfs_root("empty-cpus");
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: Vec::new(),
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };

    let err = action
        .preflight_at(&policy_for(&[0]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("action_missing_explicit_targets"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_missing_knobs() {
    let root = temp_sysfs_root("missing-knobs");
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: None,
        energy_performance_preference: None,
    };

    let err = action
        .preflight_at(&policy_for(&[0]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("action_invalid_request"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_empty_allowlist() {
    let root = temp_sysfs_root("empty-allowlist");
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };
    let mut policy = policy_for(&[]);
    policy.allowed_cpus.clear();

    let err = action.preflight_at(&policy).unwrap_err().to_string();

    assert!(err.contains("action_policy_denied"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_cpu_not_in_allowlist() {
    let root = temp_sysfs_root("cpu-not-allowed");
    write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };

    let err = action
        .preflight_at(&policy_for(&[1]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("action_policy_denied"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_governor_when_not_allowed() {
    let root = temp_sysfs_root("governor-not-allowed");
    write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };
    let mut policy = policy_for(&[0]);
    policy.allow_governor_changes = false;

    let err = action.preflight_at(&policy).unwrap_err().to_string();

    assert!(err.contains("action_policy_denied"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_epp_when_not_allowed() {
    let root = temp_sysfs_root("epp-not-allowed");
    write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: None,
        energy_performance_preference: Some("performance".to_owned()),
    };
    let mut policy = policy_for(&[0]);
    policy.allow_epp_changes = false;

    let err = action.preflight_at(&policy).unwrap_err().to_string();

    assert!(err.contains("action_policy_denied"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_invalid_sysfs_value() {
    let root = temp_sysfs_root("invalid-value");
    write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance;bad".to_owned()),
        energy_performance_preference: None,
    };

    let err = action
        .preflight_at(&policy_for(&[0]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("action_invalid_value"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_missing_cpufreq_file() {
    let root = temp_sysfs_root("missing-file");
    fs::create_dir_all(root.join("devices/system/cpu/cpu0/cpufreq")).unwrap();
    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };

    let err = action
        .preflight_at(&policy_for(&[0]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("action_missing_path"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_battery_without_override() {
    let root = temp_sysfs_root("battery-no-override");
    write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    write_ac_power(&root, false);
    write_battery(&root, "Discharging");

    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };

    let err = action
        .preflight_at(&policy_for(&[0]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("action_policy_denied"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_allows_battery_only_with_explicit_override() {
    let root = temp_sysfs_root("battery-override");
    write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    write_ac_power(&root, false);
    write_battery(&root, "Discharging");

    let action = CpuPowerAction {
        sysfs_root: root.clone(),
        cpus: vec![0],
        scaling_governor: Some("performance".to_owned()),
        energy_performance_preference: None,
    };
    let mut policy = policy_for(&[0]);
    policy.battery_policy = BatteryPolicy::AllowWithExplicitConfig;
    policy.explicit_battery_override = true;

    action.preflight_at(&policy).unwrap();

    fs::remove_dir_all(root).ok();
}

#[test]
fn dry_run_counts_pending_changes_without_mutating() {
    let root = temp_sysfs_root("dry-run");
    let cpufreq0 = write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    let cpufreq1 = write_cpu_cpufreq(&root, 1, "schedutil", "balance_performance");

    let state = action_for(&root).dry_run_at(&policy_for(&[0, 1])).unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 4);
    assert_eq!(state.affected_tasks, 4);
    assert_eq!(state.pending_changes, 4);
    assert_eq!(
        read_trimmed(&cpufreq0.join("scaling_governor")).unwrap(),
        "schedutil"
    );
    assert_eq!(
        read_trimmed(&cpufreq1.join("energy_performance_preference")).unwrap(),
        "balance_performance"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn apply_with_test_policy_writes_verifies_and_saves_every_original_value() {
    let root = temp_sysfs_root("apply");
    let cpufreq0 = write_cpu_cpufreq(&root, 0, "schedutil", "balance_performance");
    let cpufreq1 = write_cpu_cpufreq(&root, 1, "schedutil", "balance_performance");

    let token = apply_with_policy_for_test(&action_for(&root), &policy_for(&[0, 1])).unwrap();

    assert_eq!(
        read_trimmed(&cpufreq0.join("scaling_governor")).unwrap(),
        "performance"
    );
    assert_eq!(
        read_trimmed(&cpufreq1.join("energy_performance_preference")).unwrap(),
        "performance"
    );

    match token {
        RollbackToken::CpuPowerRestore { records } => {
            assert_eq!(records.len(), 4);
            assert!(records.iter().any(|record| {
                record.path.ends_with("scaling_governor") && record.original_value == "schedutil"
            }));
            assert!(records.iter().any(|record| {
                record.path.ends_with("energy_performance_preference")
                    && record.original_value == "balance_performance"
            }));
        }
        other => panic!("unexpected rollback token: {other:?}"),
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn rollback_restores_all_values_and_verifies() {
    let root = temp_sysfs_root("rollback");
    let cpufreq0 = write_cpu_cpufreq(&root, 0, "performance", "performance");
    let cpufreq1 = write_cpu_cpufreq(&root, 1, "performance", "performance");
    let action = action_for(&root);

    let token = RollbackToken::CpuPowerRestore {
        records: vec![
            CpuPowerRestoreRecord {
                path: cpufreq0.join("scaling_governor"),
                original_value: "schedutil".to_owned(),
            },
            CpuPowerRestoreRecord {
                path: cpufreq0.join("energy_performance_preference"),
                original_value: "balance_performance".to_owned(),
            },
            CpuPowerRestoreRecord {
                path: cpufreq1.join("scaling_governor"),
                original_value: "schedutil".to_owned(),
            },
            CpuPowerRestoreRecord {
                path: cpufreq1.join("energy_performance_preference"),
                original_value: "balance_performance".to_owned(),
            },
        ],
    };

    action.rollback(&token).unwrap();

    assert_eq!(
        read_trimmed(&cpufreq0.join("scaling_governor")).unwrap(),
        "schedutil"
    );
    assert_eq!(
        read_trimmed(&cpufreq1.join("energy_performance_preference")).unwrap(),
        "balance_performance"
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
    assert!(err.contains("expected cpu-power-restore"));
    assert!(err.contains("actual nice-restore"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn cpu_power_restore_token_reports_affected_records_and_no_restore_path() {
    let token = RollbackToken::CpuPowerRestore {
        records: vec![
            CpuPowerRestoreRecord {
                path: PathBuf::from("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
                original_value: "schedutil".to_owned(),
            },
            CpuPowerRestoreRecord {
                path: PathBuf::from(
                    "/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference",
                ),
                original_value: "balance_performance".to_owned(),
            },
        ],
    };

    assert_eq!(token.affected_tasks(), 2);
    assert!(token.restore_path().is_none());
}

fn apply_with_policy_for_test(
    action: &CpuPowerAction,
    policy: &CpuPowerPolicy,
) -> anyhow::Result<RollbackToken> {
    let target_files = action.collect_target_files(policy)?;
    let mut records = Vec::new();

    for target in target_files {
        records.push(CpuPowerRestoreRecord {
            path: target.path.clone(),
            original_value: target.original_value.clone(),
        });

        if target.original_value == target.requested_value {
            continue;
        }

        write_trimmed(&target.path, &target.requested_value)?;
        let actual = read_trimmed(&target.path)?;
        if actual != target.requested_value {
            rollback_cpu_power_records(&records)?;
            anyhow::bail!(
                "CPU power write verification failed for {}: requested={:?} actual={:?}",
                target.path.display(),
                target.requested_value,
                actual
            );
        }
    }

    Ok(RollbackToken::CpuPowerRestore { records })
}
