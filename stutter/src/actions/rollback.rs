use std::path::PathBuf;

use crate::actions::token::RollbackToken;

pub struct RollbackRegistry {
    handlers: Vec<Box<dyn RollbackHandler>>,
}

impl RollbackRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn register<H>(&mut self, handler: H)
    where
        H: RollbackHandler + 'static,
    {
        self.handlers.push(Box::new(handler));
    }

    pub fn handlers(&self) -> &[Box<dyn RollbackHandler>] {
        &self.handlers
    }

    pub fn discover_all(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        let mut candidates = Vec::new();
        for handler in &self.handlers {
            candidates.extend(handler.discover()?);
        }
        Ok(candidates)
    }

    pub fn preview_all(&self) -> anyhow::Result<Vec<RollbackPreview>> {
        let mut previews = Vec::new();
        for handler in &self.handlers {
            for candidate in handler.discover()? {
                previews.push(handler.dry_run(&candidate)?);
            }
        }
        Ok(previews)
    }

    pub fn restore_all(&self, input: RestoreAllInput) -> RestoreAllSummary {
        let mut summary = RestoreAllSummary::default();

        for handler in &self.handlers {
            let candidates = match handler.discover() {
                Ok(candidates) => candidates,
                Err(err) => {
                    log::warn!(
                        "rollback_discover_failed handler={} err={err:#}",
                        handler.id()
                    );
                    summary.errors += 1;
                    continue;
                }
            };

            for candidate in candidates {
                if input.dry_run {
                    match handler.dry_run(&candidate) {
                        Ok(preview) => {
                            summary.restored_total += preview.affected_tasks;
                        }
                        Err(err) => {
                            log::warn!(
                                "rollback_dry_run_failed handler={} path={} err={err:#}",
                                handler.id(),
                                candidate.restore_path.display()
                            );
                            summary.errors += 1;
                        }
                    }
                    continue;
                }

                match handler.restore(candidate) {
                    Ok(result) => {
                        summary.restored_total += result.restored;
                        summary.skipped_dead += result.skipped_dead;
                        summary.skipped_identity_mismatch += result.skipped_identity_mismatch;
                        summary.legacy_unverified += result.legacy_unverified;
                        summary.errors += result.errors;
                    }
                    Err(err) => {
                        log::warn!(
                            "rollback_restore_failed handler={} err={err:#}",
                            handler.id()
                        );
                        summary.errors += 1;
                    }
                }
            }
        }

        summary
    }

    pub fn preview_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        self.handler_for_token(token)?.dry_run_token(token)
    }

    pub fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        self.handler_for_token(token)?.restore_token(token)
    }

    fn handler_for_token(&self, token: &RollbackToken) -> anyhow::Result<&dyn RollbackHandler> {
        self.handlers
            .iter()
            .map(|handler| handler.as_ref())
            .find(|handler| handler.supports_token(token))
            .ok_or_else(|| anyhow::anyhow!("no rollback handler registered for token {token:?}"))
    }
}

pub fn default_rollback_registry() -> RollbackRegistry {
    let mut registry = RollbackRegistry::new();
    registry.register(crate::actions::cpu_affinity::CpuAffinityRollbackHandler);
    registry.register(crate::actions::nice::NiceRollbackHandler);
    registry.register(crate::actions::ioprio::IoPrioRollbackHandler);
    registry.register(crate::actions::uclamp::UclampRollbackHandler);
    registry.register(crate::actions::cgroup::CgroupRollbackHandler);
    registry.register(crate::actions::irq_affinity::IrqAffinityRollbackHandler);
    registry.register(crate::actions::cpu_power::CpuPowerRollbackHandler);
    registry.register(crate::actions::gpu_power::GpuPowerRollbackHandler);
    registry.register(crate::actions::vm_knobs::VmKnobRollbackHandler);
    registry
}

impl Default for RollbackRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub trait RollbackHandler {
    fn id(&self) -> &'static str;
    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>>;
    fn dry_run(&self, candidate: &RollbackCandidate) -> anyhow::Result<RollbackPreview>;
    fn restore(&self, candidate: RollbackCandidate) -> anyhow::Result<RollbackResult>;

    fn supports_token(&self, _token: &RollbackToken) -> bool {
        false
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!(
            "rollback handler {} does not support token preview for {token:?}",
            self.id()
        )
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        anyhow::bail!(
            "rollback handler {} does not support token restore for {token:?}",
            self.id()
        )
    }
}

#[derive(Debug, Clone)]
pub struct RollbackCandidate {
    pub handler_id: &'static str,
    pub restore_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct RollbackPreview {
    pub handler_id: &'static str,
    pub restore_path: PathBuf,
    pub affected_tasks: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct RollbackResult {
    pub handler_id: &'static str,
    pub restore_path: PathBuf,
    pub restored: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct RestoreAllInput {
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreAllSummary {
    pub restored_total: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
}

pub(crate) fn token_dry_run_preview(
    handler_id: &'static str,
    token: &RollbackToken,
    rollback_kind: &'static str,
) -> RollbackPreview {
    RollbackPreview {
        handler_id,
        restore_path: token.restore_path().cloned().unwrap_or_default(),
        affected_tasks: token.affected_tasks(),
        message: format!(
            "would restore rollback_kind={} affected_items={}",
            rollback_kind,
            token.affected_tasks()
        ),
    }
}

pub(crate) fn token_restore_result(
    handler_id: &'static str,
    token: &RollbackToken,
    restored: usize,
    skipped: usize,
    messages: Vec<String>,
) -> RollbackResult {
    RollbackResult {
        handler_id,
        restore_path: token.restore_path().cloned().unwrap_or_default(),
        restored,
        skipped_dead: skipped,
        skipped_identity_mismatch: 0,
        legacy_unverified: 0,
        errors: 0,
        messages,
    }
}

#[cfg(test)]
mod tests {
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
            TestRollbackHandler::new("good", vec!["/tmp/good"]).with_restore_result(
                RollbackResult {
                    handler_id: "good",
                    restore_path: PathBuf::from("/tmp/good"),
                    restored: 3,
                    skipped_dead: 1,
                    skipped_identity_mismatch: 2,
                    legacy_unverified: 4,
                    errors: 5,
                    messages: Vec::new(),
                },
            ),
        );
        registry.register(
            TestRollbackHandler::new("restore-error", vec!["/tmp/bad"]).with_restore_failure(),
        );
        registry.register(
            TestRollbackHandler::new("discover-error", vec!["/tmp/missing"])
                .with_discover_failure(),
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
        registry.register(
            TestRollbackHandler::new("dry-run-error", vec!["/tmp/c"]).with_dry_run_failure(),
        );

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
                        path: PathBuf::from(
                            "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
                        ),
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
                    .any(|handler| handler.id() == expected_handler
                        && handler.supports_token(&token)),
                "default registry must include {expected_handler} for token {token:?}"
            );
        }
    }
}
