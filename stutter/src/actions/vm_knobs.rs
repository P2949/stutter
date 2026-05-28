use std::{
    collections::BTreeSet,
    fs,
    fs::OpenOptions,
    path::{Component, Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction, VmKnobRestoreRecord,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmKnobMode {
    SuggestOnly,
    ManualApply,
}

#[derive(Debug, Clone)]
pub struct VmKnobPolicy {
    pub allow_vm_knob_changes: bool,
    pub mode: VmKnobMode,
    pub allowed_paths: BTreeSet<PathBuf>,
    pub require_latency_cliff_evidence: bool,
    pub latency_cliff_evidence: bool,
}

impl Default for VmKnobPolicy {
    fn default() -> Self {
        Self {
            allow_vm_knob_changes: false,
            mode: VmKnobMode::SuggestOnly,
            allowed_paths: BTreeSet::new(),
            require_latency_cliff_evidence: true,
            latency_cliff_evidence: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmKnobChange {
    pub path: PathBuf,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmKnobAction {
    pub root: PathBuf,
    pub changes: Vec<VmKnobChange>,
}

pub(crate) struct VmKnobRollbackHandler;

impl RollbackHandler for VmKnobRollbackHandler {
    fn id(&self) -> &'static str {
        "vm-knob-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "vm-knob-restore")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "vm-knob-restore")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_vm_knob_restore().is_some() || token.as_sysfs_restore().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        let rollback_kind = if token.as_vm_knob_restore().is_some() {
            "vm-knob-restore"
        } else if token.as_sysfs_restore().is_some() {
            "sysfs-restore"
        } else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "vm-knob-restore",
                token.kind(),
            )
            .into());
        };
        Ok(token_dry_run_preview(self.id(), token, rollback_kind))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        if let Some(records) = token.as_vm_knob_restore() {
            rollback_vm_knob_records(records)?;
            return Ok(token_restore_result(
                self.id(),
                token,
                records.len(),
                0,
                Vec::new(),
            ));
        }

        if let Some((path, original_value)) = token.as_sysfs_restore() {
            write_trimmed(path, original_value)?;
            return Ok(token_restore_result(
                self.id(),
                token,
                1,
                0,
                vec![format!("restored sysfs value {}", path.display())],
            ));
        }

        Err(ActionBoundaryError::unsupported_rollback_token(
            self.id(),
            "vm-knob-restore",
            token.kind(),
        )
        .into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VmKnobTargetFile {
    path: PathBuf,
    relative_path: PathBuf,
    requested_value: String,
    original_value: String,
}

impl VmKnobAction {
    pub fn preflight_with_policy(
        &self,
        policy: &VmKnobPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(policy)
    }

    pub fn dry_run_with_policy(&self, policy: &VmKnobPolicy) -> anyhow::Result<ActionState> {
        self.dry_run_at(policy)
    }

    pub fn verify_with_policy(&self, policy: &VmKnobPolicy) -> anyhow::Result<ActionState> {
        self.verify_at(policy)
    }

    pub fn apply_with_policy(&self, policy: &VmKnobPolicy) -> anyhow::Result<RollbackToken> {
        validate_manual_apply_policy(policy)?;
        let target_files = self.collect_target_files(policy)?;
        let mut records = Vec::new();

        for target in target_files {
            records.push(VmKnobRestoreRecord {
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
                rollback_vm_knob_records(&records)?;
                return Err(ActionBoundaryError::restore_failed(
                    "vm_knobs",
                    format!(
                        "VM knob write verification failed for {}: requested={:?} actual={:?}",
                        target.path.display(),
                        target.requested_value,
                        actual
                    ),
                )
                .into());
            }
        }

        Ok(RollbackToken::VmKnobRestore { records })
    }

    fn preflight_at(&self, policy: &VmKnobPolicy) -> anyhow::Result<Vec<ActionWarning>> {
        let target_files = self.collect_target_files(policy)?;
        let mut warnings = Vec::new();

        warnings.push(ActionWarning {
            message: "THP/compaction/VM knob changes are high risk, system-wide, and should remain manual/suggest-only until safer actions prove reliable".to_owned(),
        });

        if matches!(policy.mode, VmKnobMode::SuggestOnly) {
            warnings.push(ActionWarning {
                message:
                    "policy is suggest-only; no writes will be performed by preflight or dry-run"
                        .to_owned(),
            });
        }

        if target_files
            .iter()
            .all(|target| target.original_value == target.requested_value)
        {
            warnings.push(ActionWarning {
                message: "all VM knob files already have requested values".to_owned(),
            });
        }

        Ok(warnings)
    }

    fn dry_run_at(&self, policy: &VmKnobPolicy) -> anyhow::Result<ActionState> {
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
                message: "VM knob action dry-run only; no sysfs/procfs writes performed".to_owned(),
            }],
        })
    }

    fn verify_at(&self, policy: &VmKnobPolicy) -> anyhow::Result<ActionState> {
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

    fn collect_target_files(&self, policy: &VmKnobPolicy) -> anyhow::Result<Vec<VmKnobTargetFile>> {
        validate_policy_and_request(self, policy)?;
        let mut files = Vec::new();

        for change in &self.changes {
            let relative_path = normalize_relative_knob_path(&change.path)?;
            if !policy.allowed_paths.contains(&relative_path) {
                return Err(ActionBoundaryError::PolicyDenied {
                    action_kind: "vm_knobs",
                    requirement: "allowed_paths_contains_target",
                }
                .into());
            }

            validate_supported_reversible_knob(&relative_path)?;
            validate_knob_value(&relative_path, &change.value)?;
            validate_declared_safe_value(&relative_path, &change.value)?;

            let absolute_path = self.root.join(&relative_path);
            ensure_writable_file(&absolute_path)?;

            files.push(VmKnobTargetFile {
                path: absolute_path.clone(),
                relative_path,
                requested_value: change.value.clone(),
                original_value: read_trimmed(&absolute_path)?,
            });
        }

        Ok(files)
    }
}

impl TuningAction for VmKnobAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "vm-knobs:set:changes={}",
            self.changes
                .iter()
                .map(|change| format!(
                    "{}={}",
                    normalize_relative_knob_path(&change.path)
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|_| change.path.display().to_string()),
                    change.value
                ))
                .collect::<Vec<_>>()
                .join(",")
        ))
    }

    fn describe(&self) -> String {
        format!(
            "set high-risk VM knob(s): {}",
            self.changes
                .iter()
                .map(|change| format!("{}={}", change.path.display(), change.value))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    fn safety_class(&self) -> SafetyClass {
        if self.changes.len() == 1 && is_medium_risk_vm_knob_change(&self.changes[0]) {
            SafetyClass::ReversibleMediumRisk
        } else {
            SafetyClass::HighRisk
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&VmKnobPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(&VmKnobPolicy::default())
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&VmKnobPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(&VmKnobPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some(records) = token.as_vm_knob_restore() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("vm-knob-restore"),
            )
            .into());
        };

        rollback_vm_knob_records(records)
    }
}

fn validate_policy_and_request(action: &VmKnobAction, policy: &VmKnobPolicy) -> anyhow::Result<()> {
    if !policy.allow_vm_knob_changes {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "vm_knobs",
            requirement: "allow_vm_knob_changes",
        }
        .into());
    }

    if policy.allowed_paths.is_empty() {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "vm_knobs",
            requirement: "allowed_paths_non_empty",
        }
        .into());
    }

    if action.changes.is_empty() {
        return Err(ActionBoundaryError::MissingExplicitTargets {
            action_kind: "vm_knobs",
        }
        .into());
    }

    if policy.require_latency_cliff_evidence && !policy.latency_cliff_evidence {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "vm_knobs",
            requirement: "latency_cliff_evidence",
        }
        .into());
    }

    Ok(())
}

fn validate_manual_apply_policy(policy: &VmKnobPolicy) -> anyhow::Result<()> {
    if !matches!(policy.mode, VmKnobMode::ManualApply) {
        return Err(ActionBoundaryError::InvalidRequest {
            action_kind: "vm_knobs",
            reason: "VM knob action is suggest-only; set policy mode to ManualApply for explicit manual writes".to_owned(),
        }
        .into());
    }

    Ok(())
}

fn normalize_relative_knob_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                return Err(ActionBoundaryError::InvalidValue {
                    action_kind: "vm_knobs",
                    field: "path".to_owned(),
                    reason: format!(
                        "VM knob path must not contain parent traversal: {}",
                        path.display()
                    ),
                }
                .into());
            }
            Component::Prefix(_) => {
                return Err(ActionBoundaryError::InvalidValue {
                    action_kind: "vm_knobs",
                    field: "path".to_owned(),
                    reason: format!(
                        "VM knob path must not contain platform prefix: {}",
                        path.display()
                    ),
                }
                .into());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "vm_knobs",
            field: "path".to_owned(),
            reason: "VM knob path must not be empty".to_owned(),
        }
        .into());
    }

    Ok(normalized)
}

fn validate_supported_reversible_knob(relative_path: &Path) -> anyhow::Result<()> {
    let text = relative_path.to_string_lossy();

    match text.as_ref() {
        "sys/kernel/mm/transparent_hugepage/enabled"
        | "sys/kernel/mm/transparent_hugepage/defrag"
        | "sys/kernel/mm/transparent_hugepage/khugepaged/defrag"
        | "proc/sys/vm/compaction_proactiveness"
        | "proc/sys/vm/swappiness"
        | "proc/sys/vm/dirty_background_ratio"
        | "proc/sys/vm/dirty_ratio"
        | "proc/sys/vm/zone_reclaim_mode" => Ok(()),
        "proc/sys/vm/compact_memory" => {
            return Err(ActionBoundaryError::InvalidRequest {
                action_kind: "vm_knobs",
                reason: "/proc/sys/vm/compact_memory is a write-only trigger and is not reversible; keep it suggest-only outside VmKnobAction".to_owned(),
            }
            .into());
        }
        other => {
            return Err(ActionBoundaryError::UnsupportedValue {
                action_kind: "vm_knobs",
                field: "path",
                value: other.to_owned(),
            }
            .into());
        }
    }
}

fn validate_knob_value(relative_path: &Path, value: &str) -> anyhow::Result<()> {
    let text = relative_path.to_string_lossy();
    if text.starts_with("sys/kernel/mm/transparent_hugepage/") {
        validate_thp_value(relative_path, value)
    } else {
        validate_numeric_vm_value(relative_path, value)
    }
}

fn is_medium_risk_vm_knob_change(change: &VmKnobChange) -> bool {
    normalize_relative_knob_path(&change.path)
        .ok()
        .is_some_and(|path| {
            path == Path::new("proc/sys/vm/swappiness") && change.value.trim() == "10"
        })
}

fn validate_declared_safe_value(relative_path: &Path, value: &str) -> anyhow::Result<()> {
    if let Some(values) = declared_safe_values(relative_path)
        && !values.contains(&value)
    {
        return Err(ActionBoundaryError::PolicyDenied {
            action_kind: "vm_knobs",
            requirement: "declared_safe_values",
        }
        .into());
    }

    if relative_path == Path::new("proc/sys/vm/swappiness") {
        let parsed = value.parse::<u32>().with_context(|| {
            format!(
                "VM knob {} requires an unsigned integer value",
                relative_path.display()
            )
        })?;
        validate_range(relative_path, parsed, 1, 60)?;
    }

    Ok(())
}

fn declared_safe_values(relative_path: &Path) -> Option<&'static [&'static str]> {
    match relative_path.to_string_lossy().as_ref() {
        "proc/sys/vm/swappiness" => Some(&["10"]),
        "proc/sys/vm/dirty_background_ratio" => Some(&["5"]),
        "proc/sys/vm/dirty_ratio" => Some(&["10"]),
        _ => None,
    }
}

fn validate_thp_value(relative_path: &Path, value: &str) -> anyhow::Result<()> {
    validate_plain_value(relative_path, value)?;

    let allowed = match relative_path.to_string_lossy().as_ref() {
        "sys/kernel/mm/transparent_hugepage/enabled" => ["always", "madvise", "never"].as_slice(),
        "sys/kernel/mm/transparent_hugepage/defrag" => {
            ["always", "defer", "defer+madvise", "madvise", "never"].as_slice()
        }
        "sys/kernel/mm/transparent_hugepage/khugepaged/defrag" => ["0", "1"].as_slice(),
        _ => &[],
    };

    if !allowed.contains(&value) {
        return Err(ActionBoundaryError::UnsupportedValue {
            action_kind: "vm_knobs",
            field: "value",
            value: value.to_owned(),
        }
        .into());
    }

    Ok(())
}

fn validate_numeric_vm_value(relative_path: &Path, value: &str) -> anyhow::Result<()> {
    validate_plain_value(relative_path, value)?;
    let parsed = value.parse::<u32>().with_context(|| {
        format!(
            "VM knob {} requires an unsigned integer value",
            relative_path.display()
        )
    })?;

    match relative_path.to_string_lossy().as_ref() {
        "proc/sys/vm/compaction_proactiveness" => validate_range(relative_path, parsed, 0, 100),
        "proc/sys/vm/swappiness" => validate_range(relative_path, parsed, 0, 200),
        "proc/sys/vm/dirty_background_ratio" => validate_range(relative_path, parsed, 0, 100),
        "proc/sys/vm/dirty_ratio" => validate_range(relative_path, parsed, 1, 100),
        "proc/sys/vm/zone_reclaim_mode" => validate_range(relative_path, parsed, 0, 7),
        _ => {
            return Err(ActionBoundaryError::UnsupportedValue {
                action_kind: "vm_knobs",
                field: "numeric_vm_knob",
                value: relative_path.display().to_string(),
            }
            .into());
        }
    }
}

fn validate_range(relative_path: &Path, value: u32, min: u32, max: u32) -> anyhow::Result<()> {
    if !(min..=max).contains(&value) {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "vm_knobs",
            field: relative_path.display().to_string(),
            reason: format!("value {} is outside allowed range {}..={}", value, min, max),
        }
        .into());
    }

    Ok(())
}

fn validate_plain_value(relative_path: &Path, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "vm_knobs",
            field: relative_path.display().to_string(),
            reason: "value must not be empty".to_owned(),
        }
        .into());
    }

    if value.trim() != value {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "vm_knobs",
            field: relative_path.display().to_string(),
            reason: "value must not contain leading or trailing whitespace".to_owned(),
        }
        .into());
    }

    if value.len() > 64 {
        return Err(ActionBoundaryError::InvalidValue {
            action_kind: "vm_knobs",
            field: relative_path.display().to_string(),
            reason: "value is too long".to_owned(),
        }
        .into());
    }

    for ch in value.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '+') {
            return Err(ActionBoundaryError::InvalidValue {
                action_kind: "vm_knobs",
                field: relative_path.display().to_string(),
                reason: format!("value contains invalid character {:?}", ch),
            }
            .into());
        }
    }

    Ok(())
}

fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        return Err(ActionBoundaryError::MissingPath {
            action_kind: "vm_knobs",
            path: path.to_path_buf(),
        }
        .into());
    }

    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("required VM knob file is not writable: {}", path.display()))?;

    Ok(())
}

fn rollback_vm_knob_records(records: &[VmKnobRestoreRecord]) -> anyhow::Result<()> {
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
            "vm_knobs",
            format!("failed to rollback VM knob values: {}", failures.join("; ")),
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
#[cfg(test)]
#[path = "vm_knobs/tests.rs"]
mod tests;
