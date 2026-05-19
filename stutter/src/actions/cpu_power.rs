use std::{
    collections::BTreeSet,
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::actions::{
    ActionId, ActionState, ActionWarning, CpuPowerRestoreRecord, RollbackToken, SafetyClass,
    TuningAction,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryPolicy {
    Never,
    AllowWithExplicitConfig,
}

#[derive(Debug, Clone)]
pub struct CpuPowerPolicy {
    pub allow_cpu_power_changes: bool,
    pub allowed_cpus: BTreeSet<u32>,
    pub allow_governor_changes: bool,
    pub allow_epp_changes: bool,
    pub battery_policy: BatteryPolicy,
    pub explicit_battery_override: bool,
}

impl Default for CpuPowerPolicy {
    fn default() -> Self {
        Self {
            allow_cpu_power_changes: false,
            allowed_cpus: BTreeSet::new(),
            allow_governor_changes: false,
            allow_epp_changes: false,
            battery_policy: BatteryPolicy::Never,
            explicit_battery_override: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CpuPowerAction {
    pub sysfs_root: PathBuf,
    pub cpus: Vec<u32>,
    pub scaling_governor: Option<String>,
    pub energy_performance_preference: Option<String>,
}

pub(crate) struct CpuPowerRollbackHandler;

impl RollbackHandler for CpuPowerRollbackHandler {
    fn id(&self) -> &'static str {
        "cpu-power-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("CPU power rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("CPU power rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::CpuPowerRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("CPU power rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "cpu-power-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::CpuPowerRestore { records } = token else {
            anyhow::bail!("CPU power rollback handler does not support {token:?}");
        };

        rollback_cpu_power_records(records)?;

        Ok(token_restore_result(
            self.id(),
            token,
            records.len(),
            0,
            Vec::new(),
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CpuPowerTargetFile {
    cpu: u32,
    path: PathBuf,
    requested_value: String,
    original_value: String,
}

impl CpuPowerAction {
    pub fn preflight_with_policy(
        &self,
        policy: &CpuPowerPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(policy)
    }

    fn preflight_at(&self, policy: &CpuPowerPolicy) -> anyhow::Result<Vec<ActionWarning>> {
        let target_files = self.collect_target_files(policy)?;
        let mut warnings = Vec::new();

        if target_files
            .iter()
            .all(|target| target.original_value == target.requested_value)
        {
            warnings.push(ActionWarning {
                message: "all CPU power sysfs files already have requested values".to_owned(),
            });
        }

        Ok(warnings)
    }

    fn dry_run_at(&self, policy: &CpuPowerPolicy) -> anyhow::Result<ActionState> {
        let target_files = self.collect_target_files(policy)?;
        let pending_changes = target_files
            .iter()
            .filter(|target| target.original_value != target.requested_value)
            .count();

        Ok(ActionState {
            applied: false,
            affected_tasks: pending_changes,
            checked_tasks: target_files.len(),
            pending_changes,
            warnings: Vec::new(),
        })
    }

    fn verify_at(&self, policy: &CpuPowerPolicy) -> anyhow::Result<ActionState> {
        let target_files = self.collect_target_files(policy)?;
        let mut pending_changes = 0usize;

        for target in &target_files {
            let current = read_trimmed(&target.path)
                .with_context(|| format!("failed to verify {}", target.path.display()))?;
            if current != target.requested_value {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: !target_files.is_empty() && pending_changes == 0,
            affected_tasks: target_files.len(),
            checked_tasks: target_files.len(),
            pending_changes,
            warnings: Vec::new(),
        })
    }

    fn collect_target_files(
        &self,
        policy: &CpuPowerPolicy,
    ) -> anyhow::Result<Vec<CpuPowerTargetFile>> {
        validate_policy_and_request(self, policy)?;
        let mut files = Vec::new();

        for cpu in sorted_unique_cpus(&self.cpus) {
            if !policy.allowed_cpus.contains(&cpu) {
                anyhow::bail!("CPU {cpu} is not in explicit CPU power allowlist");
            }

            let cpufreq = self.cpu_cpufreq_dir(cpu);
            if !cpufreq.is_dir() {
                anyhow::bail!(
                    "cpufreq directory does not exist for CPU {cpu}: {}",
                    cpufreq.display()
                );
            }

            if let Some(governor) = &self.scaling_governor {
                let path = cpufreq.join("scaling_governor");
                files.push(read_target_file(cpu, &path, governor)?);
            }

            if let Some(epp) = &self.energy_performance_preference {
                let path = cpufreq.join("energy_performance_preference");
                files.push(read_target_file(cpu, &path, epp)?);
            }
        }

        Ok(files)
    }

    fn cpu_cpufreq_dir(&self, cpu: u32) -> PathBuf {
        self.sysfs_root
            .join("devices/system/cpu")
            .join(format!("cpu{cpu}"))
            .join("cpufreq")
    }

    fn power_supply_root(&self) -> PathBuf {
        self.sysfs_root.join("class/power_supply")
    }
}

impl TuningAction for CpuPowerAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "cpu-power:set:cpus={}:governor={}:epp={}",
            cpu_list_label(&self.cpus),
            optional_value_label(self.scaling_governor.as_deref()),
            optional_value_label(self.energy_performance_preference.as_deref())
        ))
    }

    fn describe(&self) -> String {
        format!(
            "set CPU power values for CPU(s) [{}]: scaling_governor={}, energy_performance_preference={}",
            cpu_list_label(&self.cpus),
            optional_value_label(self.scaling_governor.as_deref()),
            optional_value_label(self.energy_performance_preference.as_deref())
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&CpuPowerPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(&CpuPowerPolicy::default())
    }

    fn apply(&self) -> anyhow::Result<RollbackToken> {
        let policy = CpuPowerPolicy::default();
        let target_files = self.collect_target_files(&policy)?;
        let mut records = Vec::new();

        for target in target_files {
            records.push(CpuPowerRestoreRecord {
                path: target.path.clone(),
                original_value: target.original_value.clone(),
            });

            if target.original_value == target.requested_value {
                continue;
            }

            write_trimmed(&target.path, &target.requested_value)
                .with_context(|| format!("failed to write {}", target.path.display()))?;

            let actual = read_trimmed(&target.path)
                .with_context(|| format!("failed to verify write to {}", target.path.display()))?;
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

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(&CpuPowerPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::CpuPowerRestore { records } = token else {
            anyhow::bail!("rollback token is not a CPU power restore token");
        };

        rollback_cpu_power_records(records)
    }
}

fn validate_policy_and_request(
    action: &CpuPowerAction,
    policy: &CpuPowerPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_cpu_power_changes {
        anyhow::bail!("policy does not allow CPU governor/EPP changes");
    }

    if action.cpus.is_empty() {
        anyhow::bail!("CPU power action requires at least one explicit CPU");
    }

    if action.scaling_governor.is_none() && action.energy_performance_preference.is_none() {
        anyhow::bail!(
            "CPU power action requires scaling_governor, energy_performance_preference, or both"
        );
    }

    if policy.allowed_cpus.is_empty() {
        anyhow::bail!("CPU power action requires a non-empty explicit CPU allowlist");
    }

    if action.scaling_governor.is_some() && !policy.allow_governor_changes {
        anyhow::bail!("policy does not allow scaling_governor changes");
    }

    if action.energy_performance_preference.is_some() && !policy.allow_epp_changes {
        anyhow::bail!("policy does not allow energy_performance_preference changes");
    }

    if let Some(governor) = &action.scaling_governor {
        validate_sysfs_value("scaling_governor", governor)?;
    }

    if let Some(epp) = &action.energy_performance_preference {
        validate_sysfs_value("energy_performance_preference", epp)?;
    }

    if is_on_battery(&action.power_supply_root())? {
        match policy.battery_policy {
            BatteryPolicy::Never => {
                anyhow::bail!("refusing CPU governor/EPP changes while on battery")
            }
            BatteryPolicy::AllowWithExplicitConfig if !policy.explicit_battery_override => {
                anyhow::bail!(
                    "refusing CPU governor/EPP changes while on battery without explicit battery override"
                )
            }
            BatteryPolicy::AllowWithExplicitConfig => {}
        }
    }

    Ok(())
}

fn read_target_file(
    cpu: u32,
    path: &Path,
    requested_value: &str,
) -> anyhow::Result<CpuPowerTargetFile> {
    ensure_writable_file(path)?;
    let original_value = read_trimmed(path)?;
    Ok(CpuPowerTargetFile {
        cpu,
        path: path.to_path_buf(),
        requested_value: requested_value.to_owned(),
        original_value,
    })
}

fn validate_sysfs_value(name: &str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{name} value must not be empty");
    }

    if value.trim() != value {
        anyhow::bail!("{name} value must not contain leading or trailing whitespace");
    }

    if value.len() > 64 {
        anyhow::bail!("{name} value is too long");
    }

    for ch in value.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
            anyhow::bail!("{name} value contains invalid character {ch:?}");
        }
    }

    Ok(())
}

fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        anyhow::bail!(
            "required CPU power sysfs file does not exist: {}",
            path.display()
        );
    }

    OpenOptions::new().write(true).open(path).with_context(|| {
        format!(
            "required CPU power sysfs file is not writable: {}",
            path.display()
        )
    })?;

    Ok(())
}

fn is_on_battery(power_supply_root: &Path) -> anyhow::Result<bool> {
    if !power_supply_root.exists() {
        return Ok(false);
    }

    for entry in fs::read_dir(power_supply_root)
        .with_context(|| format!("failed to read {}", power_supply_root.display()))?
    {
        let entry = entry.with_context(|| {
            format!("failed to read entry under {}", power_supply_root.display())
        })?;
        let path = entry.path();

        let supply_type = read_trimmed(&path.join("type")).unwrap_or_default();
        if supply_type != "Mains" && supply_type != "USB" && supply_type != "USB_C" {
            continue;
        }

        let online = read_trimmed(&path.join("online")).unwrap_or_default();
        if online == "1" {
            return Ok(false);
        }
    }

    for entry in fs::read_dir(power_supply_root)
        .with_context(|| format!("failed to read {}", power_supply_root.display()))?
    {
        let entry = entry.with_context(|| {
            format!("failed to read entry under {}", power_supply_root.display())
        })?;
        let path = entry.path();

        let supply_type = read_trimmed(&path.join("type")).unwrap_or_default();
        if supply_type == "Battery" {
            let status = read_trimmed(&path.join("status")).unwrap_or_default();
            if status == "Discharging" || status == "Not charging" {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn rollback_cpu_power_records(records: &[CpuPowerRestoreRecord]) -> anyhow::Result<()> {
    let mut failures = Vec::new();

    for record in records.iter().rev() {
        if let Err(err) = write_trimmed(&record.path, &record.original_value) {
            failures.push(format!(
                "{} restore write failed: {err:#}",
                record.path.display()
            ));
            continue;
        }

        match read_trimmed(&record.path) {
            Ok(actual) if actual == record.original_value => {}
            Ok(actual) => failures.push(format!(
                "{} restore verification failed: requested={:?} actual={:?}",
                record.path.display(),
                record.original_value,
                actual
            )),
            Err(err) => failures.push(format!(
                "{} restore verification read failed: {err:#}",
                record.path.display()
            )),
        }
    }

    if !failures.is_empty() {
        anyhow::bail!(
            "failed to rollback CPU power values: {}",
            failures.join("; ")
        );
    }

    Ok(())
}

fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}

fn write_trimmed(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value.trim())
        .with_context(|| format!("failed to write {:?} to {}", value.trim(), path.display()))
}

fn sorted_unique_cpus(cpus: &[u32]) -> Vec<u32> {
    let mut cpus = cpus.to_vec();
    cpus.sort_unstable();
    cpus.dedup();
    cpus
}

fn cpu_list_label(cpus: &[u32]) -> String {
    sorted_unique_cpus(cpus)
        .into_iter()
        .map(|cpu| cpu.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn optional_value_label(value: Option<&str>) -> String {
    value.unwrap_or("keep").to_owned()
}

#[cfg(test)]
mod tests {
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
    fn safety_class_is_high_risk_by_default() {
        let root = temp_sysfs_root("safety");
        let action = action_for(&root);

        assert_eq!(action.safety_class(), SafetyClass::HighRisk);

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

        assert!(err.contains("policy does not allow CPU governor/EPP changes"));
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

        assert!(err.contains("requires at least one explicit CPU"));
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

        assert!(err.contains("requires scaling_governor"));
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

        assert!(err.contains("requires a non-empty explicit CPU allowlist"));
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

        assert!(err.contains("CPU 0 is not in explicit CPU power allowlist"));
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

        assert!(err.contains("policy does not allow scaling_governor changes"));
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

        assert!(err.contains("policy does not allow energy_performance_preference changes"));
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

        assert!(err.contains("scaling_governor value contains invalid character"));
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

        assert!(err.contains("required CPU power sysfs file does not exist"));
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

        assert!(err.contains("refusing CPU governor/EPP changes while on battery"));
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
                    record.path.ends_with("scaling_governor")
                        && record.original_value == "schedutil"
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

        assert!(err.contains("rollback token is not a CPU power restore token"));
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
}
