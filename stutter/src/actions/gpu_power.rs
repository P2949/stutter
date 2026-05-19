use std::{
    collections::BTreeSet,
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::actions::{
    ActionId, ActionState, ActionWarning, GpuPowerRestoreRecord, RollbackToken, SafetyClass,
    TuningAction,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuPowerMode {
    SuggestOnly,
    ManualApply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuBatteryPolicy {
    Never,
    AllowWithExplicitConfig,
}

#[derive(Debug, Clone)]
pub struct GpuPowerPolicy {
    pub allow_gpu_power_changes: bool,
    pub mode: GpuPowerMode,
    pub allowed_drm_cards: BTreeSet<String>,
    pub allow_force_performance_level: bool,
    pub allow_power_profile_mode: bool,
    pub battery_policy: GpuBatteryPolicy,
    pub explicit_battery_override: bool,
}

impl Default for GpuPowerPolicy {
    fn default() -> Self {
        Self {
            allow_gpu_power_changes: false,
            mode: GpuPowerMode::SuggestOnly,
            allowed_drm_cards: BTreeSet::new(),
            allow_force_performance_level: false,
            allow_power_profile_mode: false,
            battery_policy: GpuBatteryPolicy::Never,
            explicit_battery_override: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GpuPowerAction {
    pub sysfs_root: PathBuf,
    pub drm_card: String,
    pub power_dpm_force_performance_level: Option<String>,
    pub pp_power_profile_mode: Option<String>,
}

pub(crate) struct GpuPowerRollbackHandler;

impl RollbackHandler for GpuPowerRollbackHandler {
    fn id(&self) -> &'static str {
        "gpu-power-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("GPU power rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("GPU power rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::GpuPowerRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("GPU power rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "gpu-power-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::GpuPowerRestore { records } = token else {
            anyhow::bail!("GPU power rollback handler does not support {token:?}");
        };

        rollback_gpu_power_records(records)?;

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
struct GpuPowerTargetFile {
    path: PathBuf,
    requested_value: String,
    original_value: String,
}

impl GpuPowerAction {
    pub fn preflight_with_policy(
        &self,
        policy: &GpuPowerPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(policy)
    }

    pub fn dry_run_with_policy(&self, policy: &GpuPowerPolicy) -> anyhow::Result<ActionState> {
        self.dry_run_at(policy)
    }

    pub fn verify_with_policy(&self, policy: &GpuPowerPolicy) -> anyhow::Result<ActionState> {
        self.verify_at(policy)
    }

    pub fn apply_with_policy(&self, policy: &GpuPowerPolicy) -> anyhow::Result<RollbackToken> {
        validate_manual_apply_policy(policy)?;
        let target_files = self.collect_target_files(policy)?;
        let mut records = Vec::new();

        for target in target_files {
            records.push(GpuPowerRestoreRecord {
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
                rollback_gpu_power_records(&records)?;
                anyhow::bail!(
                    "GPU power write verification failed for {}: requested={:?} actual={:?}",
                    target.path.display(),
                    target.requested_value,
                    actual
                );
            }
        }

        Ok(RollbackToken::GpuPowerRestore { records })
    }

    fn preflight_at(&self, policy: &GpuPowerPolicy) -> anyhow::Result<Vec<ActionWarning>> {
        let target_files = self.collect_target_files(policy)?;
        let mut warnings = Vec::new();

        warnings.push(ActionWarning {
            message: "GPU power/performance profile changes are high risk and should remain manual/suggest-only until heavily validated".to_owned(),
        });

        if matches!(policy.mode, GpuPowerMode::SuggestOnly) {
            warnings.push(ActionWarning {
                message: "policy is suggest-only; no writes will be performed by dry-run/preflight"
                    .to_owned(),
            });
        }

        if target_files
            .iter()
            .all(|target| target.original_value == target.requested_value)
        {
            warnings.push(ActionWarning {
                message: "all GPU power sysfs files already have requested values".to_owned(),
            });
        }

        Ok(warnings)
    }

    fn dry_run_at(&self, policy: &GpuPowerPolicy) -> anyhow::Result<ActionState> {
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
            warnings: vec![ActionWarning {
                message: "GPU power action dry-run only; no sysfs writes performed".to_owned(),
            }],
        })
    }

    fn verify_at(&self, policy: &GpuPowerPolicy) -> anyhow::Result<ActionState> {
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
        policy: &GpuPowerPolicy,
    ) -> anyhow::Result<Vec<GpuPowerTargetFile>> {
        validate_policy_and_request(self, policy)?;
        let device_dir = self.device_dir();

        if !device_dir.is_dir() {
            anyhow::bail!(
                "GPU device sysfs directory does not exist for {}: {}",
                self.drm_card,
                device_dir.display()
            );
        }

        let mut files = Vec::new();

        if let Some(level) = &self.power_dpm_force_performance_level {
            files.push(read_target_file(
                &device_dir.join("power_dpm_force_performance_level"),
                level,
            )?);
        }

        if let Some(profile) = &self.pp_power_profile_mode {
            files.push(read_target_file(
                &device_dir.join("pp_power_profile_mode"),
                profile,
            )?);
        }

        Ok(files)
    }

    fn device_dir(&self) -> PathBuf {
        self.sysfs_root
            .join("class/drm")
            .join(&self.drm_card)
            .join("device")
    }

    fn power_supply_root(&self) -> PathBuf {
        self.sysfs_root.join("class/power_supply")
    }
}

impl TuningAction for GpuPowerAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "gpu-power:set:card={}:dpm={}:profile={}",
            self.drm_card,
            optional_value_label(self.power_dpm_force_performance_level.as_deref()),
            optional_value_label(self.pp_power_profile_mode.as_deref())
        ))
    }

    fn describe(&self) -> String {
        format!(
            "set GPU power values for {}: power_dpm_force_performance_level={}, pp_power_profile_mode={}",
            self.drm_card,
            optional_value_label(self.power_dpm_force_performance_level.as_deref()),
            optional_value_label(self.pp_power_profile_mode.as_deref())
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&GpuPowerPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(&GpuPowerPolicy::default())
    }

    fn apply(&self) -> anyhow::Result<RollbackToken> {
        self.apply_with_policy(&GpuPowerPolicy::default())
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(&GpuPowerPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::GpuPowerRestore { records } = token else {
            anyhow::bail!("rollback token is not a GPU power restore token");
        };

        rollback_gpu_power_records(records)
    }
}

fn validate_policy_and_request(
    action: &GpuPowerAction,
    policy: &GpuPowerPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_gpu_power_changes {
        anyhow::bail!("policy does not allow GPU power/performance profile changes");
    }

    validate_drm_card_name(&action.drm_card)?;

    if action.power_dpm_force_performance_level.is_none() && action.pp_power_profile_mode.is_none()
    {
        anyhow::bail!(
            "GPU power action requires power_dpm_force_performance_level, pp_power_profile_mode, or both"
        );
    }

    if policy.allowed_drm_cards.is_empty() {
        anyhow::bail!("GPU power action requires a non-empty explicit DRM card allowlist");
    }

    if !policy.allowed_drm_cards.contains(&action.drm_card) {
        anyhow::bail!(
            "DRM card {} is not in explicit GPU power allowlist",
            action.drm_card
        );
    }

    if action.power_dpm_force_performance_level.is_some() && !policy.allow_force_performance_level {
        anyhow::bail!("policy does not allow power_dpm_force_performance_level changes");
    }

    if action.pp_power_profile_mode.is_some() && !policy.allow_power_profile_mode {
        anyhow::bail!("policy does not allow pp_power_profile_mode changes");
    }

    if let Some(level) = &action.power_dpm_force_performance_level {
        validate_dpm_force_performance_level(level)?;
    }

    if let Some(profile) = &action.pp_power_profile_mode {
        validate_power_profile_mode(profile)?;
    }

    if is_on_battery(&action.power_supply_root())? {
        match policy.battery_policy {
            GpuBatteryPolicy::Never => {
                anyhow::bail!("refusing GPU power/performance profile changes while on battery")
            }
            GpuBatteryPolicy::AllowWithExplicitConfig if !policy.explicit_battery_override => {
                anyhow::bail!(
                    "refusing GPU power/performance profile changes while on battery without explicit battery override"
                )
            }
            GpuBatteryPolicy::AllowWithExplicitConfig => {}
        }
    }

    Ok(())
}

fn validate_manual_apply_policy(policy: &GpuPowerPolicy) -> anyhow::Result<()> {
    if !matches!(policy.mode, GpuPowerMode::ManualApply) {
        anyhow::bail!(
            "GPU power action is suggest-only; set policy mode to ManualApply for explicit manual writes"
        );
    }

    Ok(())
}

fn read_target_file(path: &Path, requested_value: &str) -> anyhow::Result<GpuPowerTargetFile> {
    ensure_writable_file(path)?;
    let original_value = read_trimmed(path)?;

    Ok(GpuPowerTargetFile {
        path: path.to_path_buf(),
        requested_value: requested_value.to_owned(),
        original_value,
    })
}

fn validate_drm_card_name(card: &str) -> anyhow::Result<()> {
    if card.trim().is_empty() {
        anyhow::bail!("DRM card name must not be empty");
    }

    if card.trim() != card {
        anyhow::bail!("DRM card name must not contain leading or trailing whitespace");
    }

    let Some(suffix) = card.strip_prefix("card") else {
        anyhow::bail!("DRM card name must start with card");
    };

    if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
        anyhow::bail!("DRM card name must be card followed by digits");
    }

    Ok(())
}

fn validate_dpm_force_performance_level(value: &str) -> anyhow::Result<()> {
    validate_sysfs_value("power_dpm_force_performance_level", value)?;

    match value {
        "auto" | "low" | "high" | "manual" | "profile_standard" | "profile_min_sclk"
        | "profile_min_mclk" | "profile_peak" => Ok(()),
        other => anyhow::bail!(
            "unsupported power_dpm_force_performance_level value {:?}",
            other
        ),
    }
}

fn validate_power_profile_mode(value: &str) -> anyhow::Result<()> {
    validate_sysfs_value("pp_power_profile_mode", value)
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
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == ' ') {
            anyhow::bail!("{name} value contains invalid character {ch:?}");
        }
    }

    Ok(())
}

fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        anyhow::bail!(
            "required GPU power sysfs file does not exist: {}",
            path.display()
        );
    }

    OpenOptions::new().write(true).open(path).with_context(|| {
        format!(
            "required GPU power sysfs file is not writable: {}",
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

fn rollback_gpu_power_records(records: &[GpuPowerRestoreRecord]) -> anyhow::Result<()> {
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
            "failed to rollback GPU power values: {}",
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
    fn safety_class_is_high_risk() {
        let root = temp_sysfs_root("safety");
        let action = action_for(&root);

        assert_eq!(action.safety_class(), SafetyClass::HighRisk);

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

        assert!(err.contains("rollback token is not a GPU power restore token"));
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
}
