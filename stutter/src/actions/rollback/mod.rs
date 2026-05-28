use std::path::PathBuf;

use crate::actions::token::RollbackToken;

#[derive(Debug, thiserror::Error)]
pub enum RollbackRegistryError {
    #[error(
        "rollback_handler_token_preview_unsupported: handler {handler_id} does not support token preview for {token_kind}"
    )]
    TokenPreviewUnsupported {
        handler_id: &'static str,
        token_kind: &'static str,
    },

    #[error(
        "rollback_handler_token_restore_unsupported: handler {handler_id} does not support token restore for {token_kind}"
    )]
    TokenRestoreUnsupported {
        handler_id: &'static str,
        token_kind: &'static str,
    },
}

#[cfg_attr(not(test), expect(dead_code))]
impl RollbackRegistryError {
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::TokenPreviewUnsupported { .. } => "rollback_handler_token_preview_unsupported",
            Self::TokenRestoreUnsupported { .. } => "rollback_handler_token_restore_unsupported",
        }
    }

    pub const fn handler_id(&self) -> &'static str {
        match self {
            Self::TokenPreviewUnsupported { handler_id, .. }
            | Self::TokenRestoreUnsupported { handler_id, .. } => handler_id,
        }
    }

    pub const fn token_kind(&self) -> &'static str {
        match self {
            Self::TokenPreviewUnsupported { token_kind, .. }
            | Self::TokenRestoreUnsupported { token_kind, .. } => token_kind,
        }
    }
}

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
        Err(RollbackRegistryError::TokenPreviewUnsupported {
            handler_id: self.id(),
            token_kind: token.kind(),
        }
        .into())
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        Err(RollbackRegistryError::TokenRestoreUnsupported {
            handler_id: self.id(),
            token_kind: token.kind(),
        }
        .into())
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
mod tests;
