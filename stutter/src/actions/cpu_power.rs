use std::{
    collections::BTreeSet,
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, CpuPowerRestoreRecord,
    RollbackToken, SafetyClass, TuningAction,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "cpu-power-restore")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "cpu-power-restore")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_cpu_power_restore().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "cpu-power-restore",
                token.kind(),
            )
            .into());
        }
        Ok(token_dry_run_preview(self.id(), token, "cpu-power-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let Some(records) = token.as_cpu_power_restore() else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "cpu-power-restore",
                token.kind(),
            )
            .into());
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

    pub fn dry_run_with_policy(&self, policy: &CpuPowerPolicy) -> anyhow::Result<ActionState> {
        self.dry_run_at(policy)
    }

    pub fn verify_with_policy(&self, policy: &CpuPowerPolicy) -> anyhow::Result<ActionState> {
        self.verify_at(policy)
    }

    pub fn apply_with_policy(&self, policy: &CpuPowerPolicy) -> anyhow::Result<RollbackToken> {
        let target_files = self.collect_target_files(policy)?;
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
                return Err(ActionBoundaryError::restore_failed(
                    "cpu_power",
                    format!(
                        "CPU power write verification failed for {}: requested={:?} actual={:?}",
                        target.path.display(),
                        target.requested_value,
                        actual
                    ),
                )
                .into());
            }
        }

        Ok(RollbackToken::CpuPowerRestore { records })
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
                return Err(ActionBoundaryError::PolicyDenied {
                    action_kind: "cpu_power",
                    requirement: "allowed_cpus_contains_target",
                }
                .into());
            }

            let cpufreq = self.cpu_cpufreq_dir(cpu);
            if !cpufreq.is_dir() {
                return Err(ActionBoundaryError::MissingPath {
                    action_kind: "cpu_power",
                    path: cpufreq,
                }
                .into());
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
        if self.energy_performance_preference.is_some() && self.scaling_governor.is_none() {
            SafetyClass::ReversibleMediumRisk
        } else {
            SafetyClass::HighRisk
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&CpuPowerPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(&CpuPowerPolicy::default())
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&CpuPowerPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(&CpuPowerPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some(records) = token.as_cpu_power_restore() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("cpu-power-restore"),
            )
            .into());
        };

        rollback_cpu_power_records(records)
    }
}

fn validate_policy_and_request(
    action: &CpuPowerAction,
    policy: &CpuPowerPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_cpu_power_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "cpu_power",
            requirement: "allow_cpu_power_changes",
        }
        .into());
    }

    if action.cpus.is_empty() {
        return Err(ActionBoundaryError::MissingExplicitTargets {
            action_kind: "cpu_power",
        }
        .into());
    }

    if action.scaling_governor.is_none() && action.energy_performance_preference.is_none() {
        return Err(ActionBoundaryError::InvalidRequest {
            action_kind: "cpu_power",
            reason:
                "CPU power action requires scaling_governor, energy_performance_preference, or both"
                    .to_owned(),
        }
        .into());
    }

    if policy.allowed_cpus.is_empty() {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "cpu_power",
            requirement: "allowed_cpus_non_empty",
        }
        .into());
    }

    if action.scaling_governor.is_some() && !policy.allow_governor_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "cpu_power",
            requirement: "allow_governor_changes",
        }
        .into());
    }

    if action.energy_performance_preference.is_some() && !policy.allow_epp_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "cpu_power",
            requirement: "allow_epp_changes",
        }
        .into());
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
                return Err(ActionBoundaryError::PolicyDenied {
                    action_kind: "cpu_power",
                    requirement: "battery_policy_allow",
                }
                .into());
            }
            BatteryPolicy::AllowWithExplicitConfig if !policy.explicit_battery_override => {
                return Err(ActionBoundaryError::PolicyDenied {
                    action_kind: "cpu_power",
                    requirement: "explicit_battery_override",
                }
                .into());
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
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "cpu_power",
            field: name.to_owned(),
            reason: format!("{name} value must not be empty"),
        }
        .into());
    }

    if value.trim() != value {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "cpu_power",
            field: name.to_owned(),
            reason: format!("{name} value must not contain leading or trailing whitespace"),
        }
        .into());
    }

    if value.len() > 64 {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "cpu_power",
            field: name.to_owned(),
            reason: format!("{name} value is too long"),
        }
        .into());
    }

    for ch in value.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
            return Err(ActionBoundaryError::InvalidValue {
                action_kind: "cpu_power",
                field: name.to_owned(),
                reason: format!("{name} value contains invalid character {ch:?}"),
            }
            .into());
        }
    }

    Ok(())
}

fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        return Err(ActionBoundaryError::MissingPath {
            action_kind: "cpu_power",
            path: path.to_path_buf(),
        }
        .into());
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
        return Err(ActionBoundaryError::restore_failed(
            "cpu_power",
            format!(
                "failed to rollback CPU power values: {}",
                failures.join("; ")
            ),
        )
        .into());
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
#[path = "cpu_power/tests.rs"]
mod tests;
