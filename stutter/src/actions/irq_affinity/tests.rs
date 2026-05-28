use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{
    sysfs::{normalize_affinity, read_trimmed, write_trimmed},
    *,
};
use crate::actions::{
    ActionId, IrqAffinityRestoreRecord, RollbackToken, SafetyClass, TuningAction,
};

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

    assert!(err.contains("action_policy_denied"));
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

    assert!(err.contains("action_policy_denied"));
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

    assert!(err.contains("action_policy_denied"));
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

    assert!(err.contains("action_policy_denied"));
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

    assert!(err.contains("action_invalid_request"));
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

    assert!(err.contains("action_invalid_request"));
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

    assert!(err.contains("action_policy_denied"));
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

    assert!(err.contains("action_invalid_request"));
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

    assert!(err.contains("action_invalid_value"));
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

    assert!(err.contains("invalid rollback token"));
    assert!(err.contains("expected irq-affinity-restore"));
    assert!(err.contains("actual nice-restore"));
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
