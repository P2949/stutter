use std::{
    fs,
    fs::OpenOptions,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ActionId, ActionState, ActionWarning, IrqAffinityRestoreRecord, RollbackToken, SafetyClass,
    TuningAction,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrqAffinityRisk {
    ReversibleMediumRisk,
    HighRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrqAffinityEvidence {
    pub strong_irq_evidence: bool,
    pub stable_irq_identity: bool,
    pub known_device_mapping: bool,
    pub observed_irq: Option<u32>,
    pub observed_device_hint: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct IrqAffinityPolicy {
    pub allow_irq_affinity_changes: bool,
    pub allow_high_risk_devices: bool,
    pub require_strong_irq_evidence: bool,
    pub require_stable_irq_identity: bool,
    pub require_known_device_mapping: bool,
}

impl Default for IrqAffinityPolicy {
    fn default() -> Self {
        Self {
            allow_irq_affinity_changes: false,
            allow_high_risk_devices: false,
            require_strong_irq_evidence: true,
            require_stable_irq_identity: true,
            require_known_device_mapping: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrqAffinityAction {
    pub irq: u32,
    pub device_hint: String,
    pub smp_affinity: String,
    pub risk: IrqAffinityRisk,
    pub evidence: IrqAffinityEvidence,
    pub irq_root: PathBuf,
}

pub(crate) struct IrqAffinityRollbackHandler;

impl RollbackHandler for IrqAffinityRollbackHandler {
    fn id(&self) -> &'static str {
        "irq-affinity-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("IRQ affinity rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("IRQ affinity rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::IrqAffinityRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("IRQ affinity rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(
            self.id(),
            token,
            "irq-affinity-restore",
        ))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::IrqAffinityRestore { records } = token else {
            anyhow::bail!("IRQ affinity rollback handler does not support {token:?}");
        };

        let mut restored = 0usize;
        let mut skipped = 0usize;
        let mut messages = Vec::new();

        for record in records {
            let irq_dir = Path::new("/proc/irq").join(record.irq.to_string());
            if !irq_dir.is_dir() {
                skipped += 1;
                messages.push(format!("IRQ {} directory disappeared", record.irq));
                continue;
            }

            match read_irq_device_hint(&irq_dir) {
                Ok(device_hint) if device_hint == record.device_hint => {}
                Ok(device_hint) => {
                    skipped += 1;
                    messages.push(format!(
                        "IRQ {} device mapping changed: expected {:?}, actual {:?}",
                        record.irq, record.device_hint, device_hint
                    ));
                    continue;
                }
                Err(err) => {
                    skipped += 1;
                    messages.push(format!(
                        "IRQ {} device mapping unreadable during rollback: {err:#}",
                        record.irq
                    ));
                    continue;
                }
            }

            let path = irq_dir.join("smp_affinity");
            write_trimmed(&path, &record.original_smp_affinity).with_context(|| {
                format!(
                    "failed to restore IRQ {} smp_affinity via {}",
                    record.irq,
                    path.display()
                )
            })?;
            restored += 1;
        }

        Ok(token_restore_result(
            self.id(),
            token,
            restored,
            skipped,
            messages,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IrqAffinitySnapshot {
    irq: u32,
    device_hint: String,
    smp_affinity: String,
}

impl IrqAffinityAction {
    pub fn new(
        irq: u32,
        device_hint: String,
        smp_affinity: String,
        risk: IrqAffinityRisk,
        evidence: IrqAffinityEvidence,
    ) -> Self {
        Self {
            irq,
            device_hint,
            smp_affinity,
            risk,
            evidence,
            irq_root: PathBuf::from("/proc/irq"),
        }
    }

    pub fn preflight_with_policy(
        &self,
        policy: &IrqAffinityPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(policy)
    }

    pub fn dry_run_with_policy(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<ActionState> {
        self.dry_run_at(policy)
    }

    pub fn verify_with_policy(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<ActionState> {
        self.verify_at(policy)
    }

    pub fn apply_with_policy(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<RollbackToken> {
        let snapshot = self.collect_snapshot(policy)?;
        let path = self.smp_affinity_path();

        if snapshot.smp_affinity.trim() != self.smp_affinity.trim() {
            write_trimmed(&path, &self.smp_affinity)
                .with_context(|| format!("failed to write IRQ {} smp_affinity", self.irq))?;

            let after = read_trimmed(&path)
                .with_context(|| format!("failed to verify IRQ {} smp_affinity write", self.irq))?;

            if normalize_affinity(&after) != normalize_affinity(&self.smp_affinity) {
                anyhow::bail!(
                    "IRQ {} smp_affinity verification failed: requested={} actual={}",
                    self.irq,
                    self.smp_affinity,
                    after
                );
            }
        }

        Ok(RollbackToken::IrqAffinityRestore {
            records: vec![IrqAffinityRestoreRecord {
                irq: snapshot.irq,
                device_hint: snapshot.device_hint,
                original_smp_affinity: snapshot.smp_affinity,
            }],
        })
    }

    fn preflight_at(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<Vec<ActionWarning>> {
        let snapshot = self.collect_snapshot(policy)?;
        let mut warnings = Vec::new();

        if snapshot.smp_affinity.trim() == self.smp_affinity.trim() {
            warnings.push(ActionWarning {
                message: format!(
                    "IRQ {} already has requested smp_affinity {}",
                    self.irq, self.smp_affinity
                ),
            });
        }

        Ok(warnings)
    }

    fn dry_run_at(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<ActionState> {
        let snapshot = self.collect_snapshot(policy)?;
        let pending_changes = usize::from(snapshot.smp_affinity.trim() != self.smp_affinity.trim());

        Ok(ActionState {
            applied: false,
            affected_tasks: pending_changes,
            checked_tasks: 1,
            pending_changes,
            warnings: Vec::new(),
        })
    }

    fn verify_at(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<ActionState> {
        let snapshot = self.collect_snapshot(policy)?;
        let pending_changes = usize::from(snapshot.smp_affinity.trim() != self.smp_affinity.trim());

        Ok(ActionState {
            applied: pending_changes == 0,
            affected_tasks: 1,
            checked_tasks: 1,
            pending_changes,
            warnings: Vec::new(),
        })
    }

    fn collect_snapshot(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<IrqAffinitySnapshot> {
        validate_policy_and_request(self, policy)?;
        validate_irq_identity(self)?;
        validate_affinity_value(&self.smp_affinity)?;

        let irq_dir = self.irq_dir();
        if !irq_dir.is_dir() {
            anyhow::bail!("IRQ directory does not exist: {}", irq_dir.display());
        }

        let smp_affinity_path = irq_dir.join("smp_affinity");
        ensure_writable_file(&smp_affinity_path)?;
        let current_affinity = read_trimmed(&smp_affinity_path)?;

        let current_device_hint = read_irq_device_hint(&irq_dir)
            .with_context(|| format!("failed to read stable device hint for IRQ {}", self.irq))?;

        if current_device_hint != self.device_hint {
            anyhow::bail!(
                "IRQ {} device mapping changed: expected {:?}, actual {:?}",
                self.irq,
                self.device_hint,
                current_device_hint
            );
        }

        Ok(IrqAffinitySnapshot {
            irq: self.irq,
            device_hint: current_device_hint,
            smp_affinity: current_affinity,
        })
    }

    fn irq_dir(&self) -> PathBuf {
        self.irq_root.join(self.irq.to_string())
    }

    fn smp_affinity_path(&self) -> PathBuf {
        self.irq_dir().join("smp_affinity")
    }
}

impl TuningAction for IrqAffinityAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "irq-affinity:set:irq={}:affinity={}",
            self.irq, self.smp_affinity
        ))
    }

    fn describe(&self) -> String {
        format!(
            "set IRQ {} ({}) smp_affinity to {}",
            self.irq, self.device_hint, self.smp_affinity
        )
    }

    fn safety_class(&self) -> SafetyClass {
        match self.risk {
            IrqAffinityRisk::ReversibleMediumRisk => SafetyClass::ReversibleMediumRisk,
            IrqAffinityRisk::HighRisk => SafetyClass::HighRisk,
        }
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&IrqAffinityPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(&IrqAffinityPolicy::default())
    }

    fn apply(&self) -> anyhow::Result<RollbackToken> {
        self.apply_with_policy(&IrqAffinityPolicy::default())
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(&IrqAffinityPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let RollbackToken::IrqAffinityRestore { records } = token else {
            anyhow::bail!("rollback token is not an IRQ affinity restore token");
        };

        let mut failures = Vec::new();

        for record in records {
            let irq_dir = self.irq_root.join(record.irq.to_string());
            if !irq_dir.is_dir() {
                failures.push(format!("IRQ {} directory disappeared", record.irq));
                continue;
            }

            match read_irq_device_hint(&irq_dir) {
                Ok(device_hint) if device_hint == record.device_hint => {}
                Ok(device_hint) => {
                    failures.push(format!(
                        "IRQ {} device mapping changed during rollback: expected {:?}, actual {:?}",
                        record.irq, record.device_hint, device_hint
                    ));
                    continue;
                }
                Err(err) => {
                    failures.push(format!(
                        "IRQ {} device mapping unreadable during rollback: {err:#}",
                        record.irq
                    ));
                    continue;
                }
            }

            let path = irq_dir.join("smp_affinity");
            if let Err(err) = write_trimmed(&path, &record.original_smp_affinity) {
                failures.push(format!(
                    "IRQ {} rollback write failed for {}: {err:#}",
                    record.irq,
                    path.display()
                ));
                continue;
            }

            match read_trimmed(&path) {
                Ok(actual)
                    if normalize_affinity(&actual)
                        == normalize_affinity(&record.original_smp_affinity) => {}
                Ok(actual) => failures.push(format!(
                    "IRQ {} rollback verification failed: requested={} actual={}",
                    record.irq, record.original_smp_affinity, actual
                )),
                Err(err) => failures.push(format!(
                    "IRQ {} rollback verification read failed: {err:#}",
                    record.irq
                )),
            }
        }

        if !failures.is_empty() {
            anyhow::bail!("failed to rollback IRQ affinity: {}", failures.join("; "));
        }

        Ok(())
    }
}

fn validate_policy_and_request(
    action: &IrqAffinityAction,
    policy: &IrqAffinityPolicy,
) -> anyhow::Result<()> {
    if !policy.allow_irq_affinity_changes {
        anyhow::bail!(
            "policy does not allow IRQ affinity changes; advisor remains investigate-first"
        );
    }

    if action.irq == 0 {
        anyhow::bail!("IRQ number must be greater than zero");
    }

    if action.device_hint.trim().is_empty() {
        anyhow::bail!("IRQ device mapping must be known before affinity changes");
    }

    if matches!(action.risk, IrqAffinityRisk::HighRisk) && !policy.allow_high_risk_devices {
        anyhow::bail!("policy does not allow high-risk IRQ affinity changes");
    }

    if policy.require_strong_irq_evidence && !action.evidence.strong_irq_evidence {
        anyhow::bail!("strong IRQ evidence is required before changing IRQ affinity");
    }

    if policy.require_stable_irq_identity && !action.evidence.stable_irq_identity {
        anyhow::bail!("stable IRQ identity is required before changing IRQ affinity");
    }

    if policy.require_known_device_mapping && !action.evidence.known_device_mapping {
        anyhow::bail!("known device mapping is required before changing IRQ affinity");
    }

    if let Some(observed_irq) = action.evidence.observed_irq
        && observed_irq != action.irq
    {
        anyhow::bail!(
            "IRQ evidence mismatch: action irq={} observed irq={}",
            action.irq,
            observed_irq
        );
    }

    if let Some(observed_device_hint) = &action.evidence.observed_device_hint
        && observed_device_hint != &action.device_hint
    {
        anyhow::bail!(
            "IRQ device evidence mismatch: action device={:?} observed device={:?}",
            action.device_hint,
            observed_device_hint
        );
    }

    Ok(())
}

fn validate_irq_identity(action: &IrqAffinityAction) -> anyhow::Result<()> {
    if action.device_hint.to_ascii_lowercase().contains("timer")
        || action
            .device_hint
            .to_ascii_lowercase()
            .contains("rescheduling")
        || action
            .device_hint
            .to_ascii_lowercase()
            .contains("call function")
        || action.device_hint.to_ascii_lowercase().contains("ipi")
    {
        anyhow::bail!(
            "refusing to change system-critical IRQ {} ({})",
            action.irq,
            action.device_hint
        );
    }

    Ok(())
}

fn validate_affinity_value(value: &str) -> anyhow::Result<()> {
    let value = value.trim();

    if value.is_empty() {
        anyhow::bail!("smp_affinity must not be empty");
    }

    if value.len() > 256 {
        anyhow::bail!("smp_affinity is too long");
    }

    for ch in value.chars() {
        if !(ch.is_ascii_hexdigit() || ch == ',') {
            anyhow::bail!("smp_affinity contains invalid character {ch:?}");
        }
    }

    if value.split(',').any(|part| part.is_empty()) {
        anyhow::bail!("smp_affinity contains an empty comma-separated component");
    }

    Ok(())
}

fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        anyhow::bail!(
            "required IRQ affinity file does not exist: {}",
            path.display()
        );
    }

    OpenOptions::new().write(true).open(path).with_context(|| {
        format!(
            "required IRQ affinity file is not writable: {}",
            path.display()
        )
    })?;

    Ok(())
}

fn read_irq_device_hint(irq_dir: &Path) -> anyhow::Result<String> {
    let actions_path = irq_dir.join("actions");
    if let Ok(actions) = read_trimmed(&actions_path)
        && !actions.is_empty()
    {
        return Ok(actions);
    }

    let name_path = irq_dir.join("name");
    if let Ok(name) = read_trimmed(&name_path)
        && !name.is_empty()
    {
        return Ok(name);
    }

    anyhow::bail!(
        "neither {} nor {} contained a device hint",
        actions_path.display(),
        name_path.display()
    )
}

fn normalize_affinity(value: &str) -> String {
    value
        .trim()
        .split(',')
        .map(|part| {
            let trimmed = part.trim_start_matches('0');
            if trimmed.is_empty() {
                "0".to_owned()
            } else {
                trimmed.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}

fn write_trimmed(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value.trim())
        .with_context(|| format!("failed to write {} to {}", value.trim(), path.display()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temp_irq_root(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-irq-affinity-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn evidence(irq: u32, device_hint: &str) -> IrqAffinityEvidence {
        IrqAffinityEvidence {
            strong_irq_evidence: true,
            stable_irq_identity: true,
            known_device_mapping: true,
            observed_irq: Some(irq),
            observed_device_hint: Some(device_hint.to_owned()),
            reason: "fake test evidence shows IRQ storm tied to mapped device".to_owned(),
        }
    }

    fn permissive_policy() -> IrqAffinityPolicy {
        IrqAffinityPolicy {
            allow_irq_affinity_changes: true,
            allow_high_risk_devices: false,
            require_strong_irq_evidence: true,
            require_stable_irq_identity: true,
            require_known_device_mapping: true,
        }
    }

    fn action(root: &Path) -> IrqAffinityAction {
        IrqAffinityAction {
            irq: 44,
            device_hint: "amdgpu".to_owned(),
            smp_affinity: "00000002".to_owned(),
            risk: IrqAffinityRisk::ReversibleMediumRisk,
            evidence: evidence(44, "amdgpu"),
            irq_root: root.to_path_buf(),
        }
    }

    fn write_fake_irq(root: &Path, irq: u32, device_hint: &str, affinity: &str) -> PathBuf {
        let irq_dir = root.join(irq.to_string());
        fs::create_dir_all(&irq_dir).unwrap();
        fs::write(irq_dir.join("actions"), format!("{device_hint}\n")).unwrap();
        fs::write(irq_dir.join("smp_affinity"), format!("{affinity}\n")).unwrap();
        irq_dir
    }

    #[test]
    fn safety_class_can_be_reversible_medium_risk() {
        let root = temp_irq_root("medium-risk");
        let action = action(&root);

        assert_eq!(action.safety_class(), SafetyClass::ReversibleMediumRisk);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn safety_class_can_be_high_risk() {
        let root = temp_irq_root("high-risk");
        let mut action = action(&root);
        action.risk = IrqAffinityRisk::HighRisk;

        assert_eq!(action.safety_class(), SafetyClass::HighRisk);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn action_id_and_description_include_irq_device_and_affinity() {
        let root = temp_irq_root("id");
        let action = action(&root);

        assert_eq!(
            action.id(),
            ActionId::new("irq-affinity:set:irq=44:affinity=00000002".to_owned())
        );
        assert_eq!(
            action.describe(),
            "set IRQ 44 (amdgpu) smp_affinity to 00000002"
        );

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_accepts_stable_irq_with_strong_evidence_and_known_mapping() {
        let root = temp_irq_root("preflight-ok");
        write_fake_irq(&root, 44, "amdgpu", "00000001");

        let warnings = action(&root).preflight_at(&permissive_policy()).unwrap();

        assert!(warnings.is_empty());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_default_policy_because_advisor_is_investigate_first() {
        let root = temp_irq_root("default-policy");
        write_fake_irq(&root, 44, "amdgpu", "00000001");

        let err = action(&root).preflight().unwrap_err().to_string();

        assert!(err.contains("advisor remains investigate-first"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_without_strong_irq_evidence() {
        let root = temp_irq_root("weak-evidence");
        write_fake_irq(&root, 44, "amdgpu", "00000001");
        let mut action = action(&root);
        action.evidence.strong_irq_evidence = false;

        let err = action
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("strong IRQ evidence is required"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_unstable_irq_identity() {
        let root = temp_irq_root("unstable");
        write_fake_irq(&root, 44, "amdgpu", "00000001");
        let mut action = action(&root);
        action.evidence.stable_irq_identity = false;

        let err = action
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("stable IRQ identity is required"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_unknown_device_mapping() {
        let root = temp_irq_root("unknown-device");
        write_fake_irq(&root, 44, "amdgpu", "00000001");
        let mut action = action(&root);
        action.evidence.known_device_mapping = false;

        let err = action
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("known device mapping is required"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_evidence_irq_mismatch() {
        let root = temp_irq_root("irq-mismatch");
        write_fake_irq(&root, 44, "amdgpu", "00000001");
        let mut action = action(&root);
        action.evidence.observed_irq = Some(45);

        let err = action
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("IRQ evidence mismatch"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_device_mapping_mismatch() {
        let root = temp_irq_root("device-mismatch");
        write_fake_irq(&root, 44, "nvme", "00000001");

        let err = action(&root)
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("device mapping changed"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_high_risk_without_policy_allowance() {
        let root = temp_irq_root("high-risk-policy");
        write_fake_irq(&root, 44, "amdgpu", "00000001");
        let mut action = action(&root);
        action.risk = IrqAffinityRisk::HighRisk;

        let err = action
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("does not allow high-risk IRQ affinity changes"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_system_critical_irq_identity() {
        let root = temp_irq_root("critical");
        write_fake_irq(&root, 44, "timer", "00000001");
        let mut action = action(&root);
        action.device_hint = "timer".to_owned();
        action.evidence = evidence(44, "timer");

        let err = action
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("refusing to change system-critical IRQ"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn preflight_rejects_invalid_affinity_string() {
        let root = temp_irq_root("invalid-affinity");
        write_fake_irq(&root, 44, "amdgpu", "00000001");
        let mut action = action(&root);
        action.smp_affinity = "not-a-mask".to_owned();

        let err = action
            .preflight_at(&permissive_policy())
            .unwrap_err()
            .to_string();

        assert!(err.contains("smp_affinity contains invalid character"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn dry_run_counts_pending_change_without_mutating() {
        let root = temp_irq_root("dry-run");
        let irq_dir = write_fake_irq(&root, 44, "amdgpu", "00000001");

        let state = action(&root).dry_run_at(&permissive_policy()).unwrap();

        assert!(!state.applied);
        assert_eq!(state.checked_tasks, 1);
        assert_eq!(state.affected_tasks, 1);
        assert_eq!(state.pending_changes, 1);
        assert_eq!(
            read_trimmed(&irq_dir.join("smp_affinity")).unwrap(),
            "00000001"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn apply_writes_affinity_verifies_and_saves_original() {
        let root = temp_irq_root("apply");
        let irq_dir = write_fake_irq(&root, 44, "amdgpu", "00000001");
        let mut action = action(&root);
        action.smp_affinity = "00000002".to_owned();

        let token = action
            .apply_with_policy_for_test(&permissive_policy())
            .unwrap();

        assert_eq!(
            read_trimmed(&irq_dir.join("smp_affinity")).unwrap(),
            "00000002"
        );

        match token {
            RollbackToken::IrqAffinityRestore { records } => {
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].irq, 44);
                assert_eq!(records[0].device_hint, "amdgpu");
                assert_eq!(records[0].original_smp_affinity, "00000001");
            }
            other => panic!("unexpected rollback token: {other:?}"),
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rollback_restores_original_affinity_and_verifies_write() {
        let root = temp_irq_root("rollback");
        let irq_dir = write_fake_irq(&root, 44, "amdgpu", "00000002");
        let action = action(&root);
        let token = RollbackToken::IrqAffinityRestore {
            records: vec![IrqAffinityRestoreRecord {
                irq: 44,
                device_hint: "amdgpu".to_owned(),
                original_smp_affinity: "00000001".to_owned(),
            }],
        };

        action.rollback(&token).unwrap();

        assert_eq!(
            read_trimmed(&irq_dir.join("smp_affinity")).unwrap(),
            "00000001"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rollback_rejects_device_mapping_change() {
        let root = temp_irq_root("rollback-mapping-change");
        write_fake_irq(&root, 44, "nvme", "00000002");
        let action = action(&root);
        let token = RollbackToken::IrqAffinityRestore {
            records: vec![IrqAffinityRestoreRecord {
                irq: 44,
                device_hint: "amdgpu".to_owned(),
                original_smp_affinity: "00000001".to_owned(),
            }],
        };

        let err = action.rollback(&token).unwrap_err().to_string();

        assert!(err.contains("device mapping changed during rollback"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rollback_rejects_wrong_token_kind() {
        let root = temp_irq_root("wrong-token");
        let action = action(&root);

        let err = action
            .rollback(&RollbackToken::NiceRestore {
                records: Vec::new(),
            })
            .unwrap_err()
            .to_string();

        assert!(err.contains("rollback token is not an IRQ affinity restore token"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn irq_affinity_restore_token_reports_affected_records_and_no_restore_path() {
        let token = RollbackToken::IrqAffinityRestore {
            records: vec![
                IrqAffinityRestoreRecord {
                    irq: 44,
                    device_hint: "amdgpu".to_owned(),
                    original_smp_affinity: "00000001".to_owned(),
                },
                IrqAffinityRestoreRecord {
                    irq: 45,
                    device_hint: "nvme".to_owned(),
                    original_smp_affinity: "00000002".to_owned(),
                },
            ],
        };

        assert_eq!(token.affected_tasks(), 2);
        assert!(token.restore_path().is_none());
    }

    impl IrqAffinityAction {
        fn apply_with_policy_for_test(
            &self,
            policy: &IrqAffinityPolicy,
        ) -> anyhow::Result<RollbackToken> {
            let snapshot = self.collect_snapshot(policy)?;
            let path = self.smp_affinity_path();

            if snapshot.smp_affinity.trim() != self.smp_affinity.trim() {
                write_trimmed(&path, &self.smp_affinity)?;
                let after = read_trimmed(&path)?;

                if normalize_affinity(&after) != normalize_affinity(&self.smp_affinity) {
                    anyhow::bail!(
                        "IRQ {} smp_affinity verification failed: requested={} actual={}",
                        self.irq,
                        self.smp_affinity,
                        after
                    );
                }
            }

            Ok(RollbackToken::IrqAffinityRestore {
                records: vec![IrqAffinityRestoreRecord {
                    irq: snapshot.irq,
                    device_hint: snapshot.device_hint,
                    original_smp_affinity: snapshot.smp_affinity,
                }],
            })
        }
    }
}
