use std::path::{Path, PathBuf};

use anyhow::Context;

use super::{
    model::{IrqAffinityAction, IrqAffinityPolicy, IrqAffinityRisk, IrqAffinitySnapshot},
    sysfs::{
        ensure_writable_file, normalize_affinity, read_irq_device_hint, read_trimmed, write_trimmed,
    },
    validate::{validate_affinity_value, validate_irq_identity, validate_policy_and_request},
};
use crate::actions::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, IrqAffinityRestoreRecord,
    RollbackToken, SafetyClass, TuningAction,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
};

pub(crate) struct IrqAffinityRollbackHandler;

impl RollbackHandler for IrqAffinityRollbackHandler {
    fn id(&self) -> &'static str {
        "irq-affinity-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "irq-affinity-restore")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "irq-affinity-restore")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_irq_affinity_restore().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "irq-affinity-restore",
                token.kind(),
            )
            .into());
        }
        Ok(token_dry_run_preview(
            self.id(),
            token,
            "irq-affinity-restore",
        ))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let Some(records) = token.as_irq_affinity_restore() else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "irq-affinity-restore",
                token.kind(),
            )
            .into());
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

impl IrqAffinityAction {
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
                return Err(ActionBoundaryError::restore_failed(
                    "irq_affinity",
                    format!(
                        "IRQ {} smp_affinity verification failed: requested={} actual={}",
                        self.irq, self.smp_affinity, after
                    ),
                )
                .into());
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

    pub(super) fn preflight_at(
        &self,
        policy: &IrqAffinityPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
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

    pub(super) fn dry_run_at(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<ActionState> {
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

    pub(super) fn verify_at(&self, policy: &IrqAffinityPolicy) -> anyhow::Result<ActionState> {
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

    pub(super) fn collect_snapshot(
        &self,
        policy: &IrqAffinityPolicy,
    ) -> anyhow::Result<IrqAffinitySnapshot> {
        validate_policy_and_request(self, policy)?;
        validate_irq_identity(self)?;
        validate_affinity_value(&self.smp_affinity)?;

        let irq_dir = self.irq_dir();
        if !irq_dir.is_dir() {
            return Err(ActionBoundaryError::MissingPath {
                action_kind: "irq_affinity",
                path: irq_dir,
            }
            .into());
        }

        let smp_affinity_path = irq_dir.join("smp_affinity");
        ensure_writable_file(&smp_affinity_path)?;
        let current_affinity = read_trimmed(&smp_affinity_path)?;

        let current_device_hint = read_irq_device_hint(&irq_dir)
            .with_context(|| format!("failed to read stable device hint for IRQ {}", self.irq))?;

        if current_device_hint != self.device_hint {
            return Err(ActionBoundaryError::InvalidRequest {
                action_kind: "irq_affinity",
                reason: format!(
                    "IRQ {} device mapping changed: expected {:?}, actual {:?}",
                    self.irq, self.device_hint, current_device_hint
                ),
            }
            .into());
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

    pub(super) fn smp_affinity_path(&self) -> PathBuf {
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

    fn apply(&self) -> crate::actions::ApplyResult {
        self.apply_with_policy(&IrqAffinityPolicy::default())
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(&IrqAffinityPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some(records) = token.as_irq_affinity_restore() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("irq-affinity-restore"),
            )
            .into());
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
            return Err(ActionBoundaryError::restore_failed(
                "irq_affinity",
                format!("failed to rollback IRQ affinity: {}", failures.join("; ")),
            )
            .into());
        }

        Ok(())
    }
}
