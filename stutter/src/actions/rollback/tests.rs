use std::path::PathBuf;

use super::*;
use crate::actions::token::RollbackToken;

struct TestRollbackHandler {
    id: &'static str,
    candidates: Vec<PathBuf>,
    restore_result: RollbackResult,
    fail_discover: bool,
    fail_dry_run: bool,
    fail_restore: bool,
}

impl TestRollbackHandler {
    fn new(id: &'static str, candidates: Vec<&str>) -> Self {
        Self {
            id,
            candidates: candidates.into_iter().map(PathBuf::from).collect(),
            restore_result: RollbackResult {
                handler_id: id,
                restore_path: PathBuf::from("/tmp/restore"),
                restored: 1,
                skipped_dead: 0,
                skipped_identity_mismatch: 0,
                legacy_unverified: 0,
                errors: 0,
                messages: Vec::new(),
            },
            fail_discover: false,
            fail_dry_run: false,
            fail_restore: false,
        }
    }

    fn with_restore_result(mut self, restore_result: RollbackResult) -> Self {
        self.restore_result = restore_result;
        self
    }

    fn with_discover_failure(mut self) -> Self {
        self.fail_discover = true;
        self
    }

    fn with_dry_run_failure(mut self) -> Self {
        self.fail_dry_run = true;
        self
    }

    fn with_restore_failure(mut self) -> Self {
        self.fail_restore = true;
        self
    }
}

impl RollbackHandler for TestRollbackHandler {
    fn id(&self) -> &'static str {
        self.id
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        if self.fail_discover {
            anyhow::bail!("discover failed");
        }
        Ok(self
            .candidates
            .iter()
            .cloned()
            .map(|restore_path| RollbackCandidate {
                handler_id: self.id,
                restore_path,
            })
            .collect())
    }

    fn dry_run(&self, candidate: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        if self.fail_dry_run {
            anyhow::bail!("dry run failed");
        }
        Ok(RollbackPreview {
            handler_id: self.id,
            restore_path: candidate.restore_path.clone(),
            affected_tasks: 2,
            message: "would restore test candidate".to_owned(),
        })
    }

    fn restore(&self, candidate: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        if self.fail_restore {
            anyhow::bail!("restore failed");
        }
        let mut result = self.restore_result.clone();
        result.restore_path = candidate.restore_path;
        Ok(result)
    }
}

struct TokenRollbackHandler;

impl RollbackHandler for TokenRollbackHandler {
    fn id(&self) -> &'static str {
        "token"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("path dry-run is not supported by token test handler")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("path restore is not supported by token test handler")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::SysfsRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        Ok(RollbackPreview {
            handler_id: self.id(),
            restore_path: token.restore_path().cloned().unwrap_or_default(),
            affected_tasks: token.affected_tasks(),
            message: "would restore sysfs token".to_owned(),
        })
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        Ok(RollbackResult {
            handler_id: self.id(),
            restore_path: token.restore_path().cloned().unwrap_or_default(),
            restored: token.affected_tasks(),
            skipped_dead: 0,
            skipped_identity_mismatch: 0,
            legacy_unverified: 0,
            errors: 0,
            messages: vec!["restored sysfs token".to_owned()],
        })
    }
}

#[test]
fn rollback_registry_discovers_and_previews_all_handlers() {
    let mut registry = RollbackRegistry::new();
    registry.register(TestRollbackHandler::new("affinity", vec!["/tmp/a"]));
    registry.register(TestRollbackHandler::new(
        "profile",
        vec!["/tmp/b", "/tmp/c"],
    ));

    let candidates = registry.discover_all().unwrap();
    let previews = registry.preview_all().unwrap();

    assert_eq!(candidates.len(), 3);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.handler_id)
            .collect::<Vec<_>>(),
        vec!["affinity", "profile", "profile"]
    );
    assert_eq!(previews.len(), 3);
    assert!(previews.iter().all(|preview| preview.affected_tasks == 2));
}

#[test]
fn rollback_registry_restore_all_aggregates_partial_failures() {
    let mut registry = RollbackRegistry::new();
    registry.register(
        TestRollbackHandler::new("good", vec!["/tmp/good"]).with_restore_result(RollbackResult {
            handler_id: "good",
            restore_path: PathBuf::from("/tmp/good"),
            restored: 3,
            skipped_dead: 1,
            skipped_identity_mismatch: 2,
            legacy_unverified: 4,
            errors: 5,
            messages: Vec::new(),
        }),
    );
    registry.register(
        TestRollbackHandler::new("restore-error", vec!["/tmp/bad"]).with_restore_failure(),
    );
    registry.register(
        TestRollbackHandler::new("discover-error", vec!["/tmp/missing"]).with_discover_failure(),
    );

    let summary = registry.restore_all(RestoreAllInput { dry_run: false });

    assert_eq!(summary.restored_total, 3);
    assert_eq!(summary.skipped_dead, 1);
    assert_eq!(summary.skipped_identity_mismatch, 2);
    assert_eq!(summary.legacy_unverified, 4);
    assert_eq!(summary.errors, 7);
}

#[test]
fn rollback_registry_dry_run_restore_all_counts_previewed_records() {
    let mut registry = RollbackRegistry::new();
    registry.register(TestRollbackHandler::new("good", vec!["/tmp/a", "/tmp/b"]));
    registry
        .register(TestRollbackHandler::new("dry-run-error", vec!["/tmp/c"]).with_dry_run_failure());

    let summary = registry.restore_all(RestoreAllInput { dry_run: true });

    assert_eq!(summary.restored_total, 4);
    assert_eq!(summary.errors, 1);
    assert_eq!(summary.skipped_dead, 0);
}

#[test]
fn rollback_registry_routes_token_preview_and_restore_to_registered_handler() {
    let mut registry = RollbackRegistry::new();
    registry.register(TokenRollbackHandler);
    let token = RollbackToken::SysfsRestore {
        path: PathBuf::from("/tmp/test-knob"),
        original_value: "original".to_owned(),
    };

    let preview = registry.preview_token(&token).unwrap();
    let result = registry.restore_token(&token).unwrap();

    assert_eq!(preview.handler_id, "token");
    assert_eq!(preview.restore_path, PathBuf::from("/tmp/test-knob"));
    assert_eq!(preview.affected_tasks, 1);
    assert_eq!(result.handler_id, "token");
    assert_eq!(result.restored, 1);
    assert_eq!(result.messages, vec!["restored sysfs token"]);
}

#[test]
fn default_rollback_registry_has_handler_for_every_reversible_token_kind() {
    let registry = default_rollback_registry();
    let tokens = vec![
        (
            "cpu-affinity-rollback",
            RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-affinity-restore.json"),
                affected_tasks: 1,
            },
        ),
        (
            "nice-rollback",
            RollbackToken::NiceRestore {
                records: vec![crate::actions::NiceRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        1,
                        None,
                        Some("test".to_owned()),
                        None,
                        None,
                    ),
                    0,
                )],
            },
        ),
        (
            "ioprio-rollback",
            RollbackToken::IoPrioRestore {
                records: vec![crate::actions::IoPrioRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        1,
                        None,
                        Some("test".to_owned()),
                        None,
                        None,
                    ),
                    0,
                )],
            },
        ),
        (
            "uclamp-rollback",
            RollbackToken::UclampRestore {
                records: vec![crate::actions::UclampRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        1,
                        None,
                        Some("test".to_owned()),
                        None,
                        None,
                    ),
                    0,
                    1024,
                )],
            },
        ),
        (
            "cgroup-rollback",
            RollbackToken::CgroupRestore {
                records: vec![crate::actions::CgroupRestoreRecord::new(
                    crate::actions::TaskRestoreIdentity::observed(
                        1,
                        None,
                        Some("test".to_owned()),
                        None,
                        None,
                    ),
                    PathBuf::from("/sys/fs/cgroup"),
                )],
                cpuset: None,
            },
        ),
        (
            "irq-affinity-rollback",
            RollbackToken::IrqAffinityRestore {
                records: vec![crate::actions::IrqAffinityRestoreRecord {
                    irq: 1,
                    device_hint: "test".to_owned(),
                    original_smp_affinity: "1".to_owned(),
                }],
            },
        ),
        (
            "cpu-power-rollback",
            RollbackToken::CpuPowerRestore {
                records: vec![crate::actions::CpuPowerRestoreRecord {
                    path: PathBuf::from("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
                    original_value: "powersave".to_owned(),
                }],
            },
        ),
        (
            "gpu-power-rollback",
            RollbackToken::GpuPowerRestore {
                records: vec![crate::actions::GpuPowerRestoreRecord {
                    path: PathBuf::from(
                        "/sys/class/drm/card0/device/power_dpm_force_performance_level",
                    ),
                    original_value: "auto".to_owned(),
                }],
            },
        ),
        (
            "vm-knob-rollback",
            RollbackToken::VmKnobRestore {
                records: vec![crate::actions::VmKnobRestoreRecord {
                    path: PathBuf::from("/proc/sys/vm/swappiness"),
                    original_value: "60".to_owned(),
                }],
            },
        ),
        (
            "vm-knob-rollback",
            RollbackToken::SysfsRestore {
                path: PathBuf::from("/sys/devices/system/cpu/test-knob"),
                original_value: "0".to_owned(),
            },
        ),
    ];

    for (expected_handler, token) in tokens {
        assert!(
            registry
                .handlers()
                .iter()
                .any(|handler| handler.id() == expected_handler && handler.supports_token(&token)),
            "default registry must include {expected_handler} for token {token:?}"
        );
    }
}

#[test]
fn rollback_handler_default_token_preview_error_is_typed() {
    let handler = TestRollbackHandler::new("default-handler", vec![]);
    let token = RollbackToken::SysfsRestore {
        path: PathBuf::from("/sys/devices/system/cpu/test-knob"),
        original_value: "0".to_owned(),
    };

    let err = handler
        .dry_run_token(&token)
        .expect_err("default token preview should be unsupported");

    let typed = err
        .downcast_ref::<RollbackRegistryError>()
        .expect("default token preview error should be typed");

    assert_eq!(
        typed.reason_code(),
        "rollback_handler_token_preview_unsupported"
    );
    assert_eq!(typed.handler_id(), Some("default-handler"));
    assert_eq!(typed.token_kind(), "sysfs-restore");
    assert!(format!("{typed}").contains("rollback_handler_token_preview_unsupported"));
}

#[test]
fn rollback_handler_default_token_restore_error_is_typed() {
    let handler = TestRollbackHandler::new("default-handler", vec![]);
    let token = RollbackToken::SysfsRestore {
        path: PathBuf::from("/sys/devices/system/cpu/test-knob"),
        original_value: "0".to_owned(),
    };

    let err = handler
        .restore_token(&token)
        .expect_err("default token restore should be unsupported");

    let typed = err
        .downcast_ref::<RollbackRegistryError>()
        .expect("default token restore error should be typed");

    assert_eq!(
        typed.reason_code(),
        "rollback_handler_token_restore_unsupported"
    );
    assert_eq!(typed.handler_id(), Some("default-handler"));
    assert_eq!(typed.token_kind(), "sysfs-restore");
    assert!(format!("{typed}").contains("rollback_handler_token_restore_unsupported"));
}

#[test]
fn rollback_production_code_has_no_string_coded_anyhow_errors() {
    let source = include_str!("mod.rs");
    let production_source = source
        .split("#[cfg(test)]")
        .next()
        .expect("rollback module should have a production section");

    for forbidden in ["anyhow::bail!", "anyhow::anyhow!"] {
        assert!(
            !production_source.contains(forbidden),
            "production rollback module should use typed errors instead of string-coded {forbidden}"
        );
    }
}

#[test]
fn rollback_registry_missing_token_handler_error_is_typed() {
    let registry = RollbackRegistry::new();
    let token = RollbackToken::SysfsRestore {
        path: PathBuf::from("/sys/devices/system/cpu/test-knob"),
        original_value: "0".to_owned(),
    };

    let err = registry
        .preview_token(&token)
        .expect_err("empty registry should not support sysfs rollback token");

    let typed = err
        .downcast_ref::<RollbackRegistryError>()
        .expect("missing token handler error should be typed");

    assert_eq!(typed.reason_code(), "rollback_handler_for_token_missing");
    assert_eq!(typed.handler_id(), None);
    assert_eq!(typed.token_kind(), "sysfs-restore");
    assert!(format!("{typed}").contains("rollback_handler_for_token_missing"));
}

#[test]
fn rollback_registry_missing_token_restore_handler_error_is_typed() {
    let registry = RollbackRegistry::new();
    let token = RollbackToken::SysfsRestore {
        path: PathBuf::from("/sys/devices/system/cpu/test-knob"),
        original_value: "0".to_owned(),
    };

    let err = registry
        .restore_token(&token)
        .expect_err("empty registry should not support sysfs rollback token");

    let typed = err
        .downcast_ref::<RollbackRegistryError>()
        .expect("missing token restore handler error should be typed");

    assert_eq!(typed.reason_code(), "rollback_handler_for_token_missing");
    assert_eq!(typed.handler_id(), None);
    assert_eq!(typed.token_kind(), "sysfs-restore");
    assert!(format!("{typed}").contains("rollback_handler_for_token_missing"));
}
