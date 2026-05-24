//! Tests extracted from the parent module to keep production files below the architecture size gate.

use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

fn temp_root(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-vm-knobs-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_knob(root: &Path, relative: &str, value: &str) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, format!("{value}\n")).unwrap();
    path
}

fn manual_policy(paths: &[&str]) -> VmKnobPolicy {
    VmKnobPolicy {
        allow_vm_knob_changes: true,
        mode: VmKnobMode::ManualApply,
        allowed_paths: paths.iter().map(PathBuf::from).collect::<BTreeSet<_>>(),
        require_latency_cliff_evidence: true,
        latency_cliff_evidence: true,
    }
}

fn suggest_policy(paths: &[&str]) -> VmKnobPolicy {
    VmKnobPolicy {
        mode: VmKnobMode::SuggestOnly,
        ..manual_policy(paths)
    }
}

fn thp_action(root: &Path) -> VmKnobAction {
    VmKnobAction {
        root: root.to_path_buf(),
        changes: vec![
            VmKnobChange {
                path: PathBuf::from("sys/kernel/mm/transparent_hugepage/enabled"),
                value: "madvise".to_owned(),
            },
            VmKnobChange {
                path: PathBuf::from("proc/sys/vm/compaction_proactiveness"),
                value: "20".to_owned(),
            },
        ],
    }
}

#[test]
fn safety_class_is_high_risk() {
    let root = temp_root("safety");
    let action = thp_action(&root);

    assert_eq!(action.safety_class(), SafetyClass::HighRisk);

    fs::remove_dir_all(root).ok();
}

#[test]
fn swappiness_safe_value_is_reversible_medium_risk() {
    let root = temp_root("swappiness-medium-risk");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("proc/sys/vm/swappiness"),
            value: "10".to_owned(),
        }],
    };

    assert_eq!(action.safety_class(), SafetyClass::ReversibleMediumRisk);

    fs::remove_dir_all(root).ok();
}

#[test]
fn action_id_and_description_include_changes() {
    let root = temp_root("id");
    let action = thp_action(&root);

    assert_eq!(
            action.id(),
            ActionId::new(
                "vm-knobs:set:changes=sys/kernel/mm/transparent_hugepage/enabled=madvise,proc/sys/vm/compaction_proactiveness=20".to_owned()
            )
        );
    assert_eq!(
        action.describe(),
        "set high-risk VM knob(s): sys/kernel/mm/transparent_hugepage/enabled=madvise, proc/sys/vm/compaction_proactiveness=20"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_default_disabled_policy() {
    let root = temp_root("default-disabled");
    write_knob(
        &root,
        "sys/kernel/mm/transparent_hugepage/enabled",
        "always",
    );
    write_knob(&root, "proc/sys/vm/compaction_proactiveness", "10");

    let err = thp_action(&root).preflight().unwrap_err().to_string();

    assert!(err.contains("policy does not allow THP/compaction/VM knob changes"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_warns_high_risk_and_suggest_only() {
    let root = temp_root("suggest-only");
    write_knob(
        &root,
        "sys/kernel/mm/transparent_hugepage/enabled",
        "always",
    );
    write_knob(&root, "proc/sys/vm/compaction_proactiveness", "10");

    let warnings = thp_action(&root)
        .preflight_at(&suggest_policy(&[
            "sys/kernel/mm/transparent_hugepage/enabled",
            "proc/sys/vm/compaction_proactiveness",
        ]))
        .unwrap();

    assert!(
        warnings
            .iter()
            .any(|warning| { warning.message.contains("high risk, system-wide") })
    );
    assert!(
        warnings
            .iter()
            .any(|warning| { warning.message.contains("policy is suggest-only") })
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_empty_allowlist() {
    let root = temp_root("empty-allowlist");
    let err = thp_action(&root)
        .preflight_at(&manual_policy(&[]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires a non-empty explicit path allowlist"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_empty_changes() {
    let root = temp_root("empty-changes");
    let action = VmKnobAction {
        root: root.clone(),
        changes: Vec::new(),
    };

    let err = action
        .preflight_at(&manual_policy(&["proc/sys/vm/swappiness"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires at least one explicit knob change"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_without_latency_cliff_evidence() {
    let root = temp_root("no-evidence");
    write_knob(&root, "proc/sys/vm/swappiness", "60");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("proc/sys/vm/swappiness"),
            value: "10".to_owned(),
        }],
    };
    let mut policy = manual_policy(&["proc/sys/vm/swappiness"]);
    policy.latency_cliff_evidence = false;

    let err = action.preflight_at(&policy).unwrap_err().to_string();

    assert!(err.contains("latency cliff evidence is required"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_path_not_in_allowlist() {
    let root = temp_root("not-allowed");
    write_knob(&root, "proc/sys/vm/swappiness", "60");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("proc/sys/vm/swappiness"),
            value: "10".to_owned(),
        }],
    };

    let err = action
        .preflight_at(&manual_policy(&["proc/sys/vm/dirty_ratio"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("is not in explicit allowlist"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_parent_traversal() {
    let root = temp_root("traversal");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("../proc/sys/vm/swappiness"),
            value: "10".to_owned(),
        }],
    };

    let err = action
        .preflight_at(&manual_policy(&["proc/sys/vm/swappiness"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("must not contain parent traversal"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_unsupported_knob() {
    let root = temp_root("unsupported");
    write_knob(&root, "proc/sys/vm/overcommit_memory", "0");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("proc/sys/vm/overcommit_memory"),
            value: "1".to_owned(),
        }],
    };

    let err = action
        .preflight_at(&manual_policy(&["proc/sys/vm/overcommit_memory"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("unsupported VM knob"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_compact_memory_as_not_reversible() {
    let root = temp_root("compact-memory");
    write_knob(&root, "proc/sys/vm/compact_memory", "0");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("proc/sys/vm/compact_memory"),
            value: "1".to_owned(),
        }],
    };

    let err = action
        .preflight_at(&manual_policy(&["proc/sys/vm/compact_memory"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("write-only trigger and is not reversible"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_invalid_thp_value() {
    let root = temp_root("bad-thp");
    write_knob(
        &root,
        "sys/kernel/mm/transparent_hugepage/enabled",
        "always",
    );
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("sys/kernel/mm/transparent_hugepage/enabled"),
            value: "sometimes".to_owned(),
        }],
    };

    let err = action
        .preflight_at(&manual_policy(&[
            "sys/kernel/mm/transparent_hugepage/enabled",
        ]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("unsupported value"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_numeric_value_out_of_range() {
    let root = temp_root("bad-range");
    write_knob(&root, "proc/sys/vm/dirty_ratio", "20");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("proc/sys/vm/dirty_ratio"),
            value: "0".to_owned(),
        }],
    };

    let err = action
        .preflight_at(&manual_policy(&["proc/sys/vm/dirty_ratio"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("outside allowed range 1..=100"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_rejects_missing_knob_file() {
    let root = temp_root("missing-file");
    let action = VmKnobAction {
        root: root.clone(),
        changes: vec![VmKnobChange {
            path: PathBuf::from("proc/sys/vm/swappiness"),
            value: "10".to_owned(),
        }],
    };

    let err = action
        .preflight_at(&manual_policy(&["proc/sys/vm/swappiness"]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("required VM knob file does not exist"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn dry_run_counts_pending_changes_without_mutating() {
    let root = temp_root("dry-run");
    let thp = write_knob(
        &root,
        "sys/kernel/mm/transparent_hugepage/enabled",
        "always",
    );
    let compaction = write_knob(&root, "proc/sys/vm/compaction_proactiveness", "10");

    let state = thp_action(&root)
        .dry_run_at(&manual_policy(&[
            "sys/kernel/mm/transparent_hugepage/enabled",
            "proc/sys/vm/compaction_proactiveness",
        ]))
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 2);
    assert_eq!(state.affected_tasks, 2);
    assert_eq!(state.pending_changes, 2);
    assert_eq!(read_trimmed(&thp).unwrap(), "always");
    assert_eq!(read_trimmed(&compaction).unwrap(), "10");
    fs::remove_dir_all(root).ok();
}

#[test]
fn apply_rejects_suggest_only_policy() {
    let root = temp_root("apply-suggest-only");
    write_knob(
        &root,
        "sys/kernel/mm/transparent_hugepage/enabled",
        "always",
    );
    write_knob(&root, "proc/sys/vm/compaction_proactiveness", "10");

    let err = thp_action(&root)
        .apply_with_policy(&suggest_policy(&[
            "sys/kernel/mm/transparent_hugepage/enabled",
            "proc/sys/vm/compaction_proactiveness",
        ]))
        .unwrap_err()
        .to_string();

    assert!(err.contains("VM knob action is suggest-only"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn apply_with_manual_policy_writes_verifies_and_saves_every_original_value() {
    let root = temp_root("apply");
    let thp = write_knob(
        &root,
        "sys/kernel/mm/transparent_hugepage/enabled",
        "always",
    );
    let compaction = write_knob(&root, "proc/sys/vm/compaction_proactiveness", "10");

    let token = thp_action(&root)
        .apply_with_policy(&manual_policy(&[
            "sys/kernel/mm/transparent_hugepage/enabled",
            "proc/sys/vm/compaction_proactiveness",
        ]))
        .unwrap();

    assert_eq!(read_trimmed(&thp).unwrap(), "madvise");
    assert_eq!(read_trimmed(&compaction).unwrap(), "20");

    match token {
        RollbackToken::VmKnobRestore { records } => {
            assert_eq!(records.len(), 2);
            assert!(records.iter().any(|record| {
                record.path.ends_with("enabled") && record.original_value == "always"
            }));
            assert!(records.iter().any(|record| {
                record.path.ends_with("compaction_proactiveness") && record.original_value == "10"
            }));
        }
        other => panic!("unexpected rollback token: {other:?}"),
    }

    fs::remove_dir_all(root).ok();
}

#[test]
fn rollback_restores_all_values_and_verifies() {
    let root = temp_root("rollback");
    let thp = write_knob(
        &root,
        "sys/kernel/mm/transparent_hugepage/enabled",
        "madvise",
    );
    let compaction = write_knob(&root, "proc/sys/vm/compaction_proactiveness", "20");
    let action = thp_action(&root);

    let token = RollbackToken::VmKnobRestore {
        records: vec![
            VmKnobRestoreRecord {
                path: thp.clone(),
                original_value: "always".to_owned(),
            },
            VmKnobRestoreRecord {
                path: compaction.clone(),
                original_value: "10".to_owned(),
            },
        ],
    };

    action.rollback(&token).unwrap();

    assert_eq!(read_trimmed(&thp).unwrap(), "always");
    assert_eq!(read_trimmed(&compaction).unwrap(), "10");
    fs::remove_dir_all(root).ok();
}

#[test]
fn rollback_rejects_wrong_token_kind() {
    let root = temp_root("wrong-token");
    let action = thp_action(&root);

    let err = action
        .rollback(&RollbackToken::NiceRestore {
            records: Vec::new(),
        })
        .unwrap_err()
        .to_string();

    assert!(err.contains("rollback token is not a VM knob restore token"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn vm_knob_restore_token_reports_affected_records_and_no_restore_path() {
    let token = RollbackToken::VmKnobRestore {
        records: vec![
            VmKnobRestoreRecord {
                path: PathBuf::from("/sys/kernel/mm/transparent_hugepage/enabled"),
                original_value: "always".to_owned(),
            },
            VmKnobRestoreRecord {
                path: PathBuf::from("/proc/sys/vm/compaction_proactiveness"),
                original_value: "10".to_owned(),
            },
        ],
    };

    assert_eq!(token.affected_tasks(), 2);
    assert!(token.restore_path().is_none());
}
