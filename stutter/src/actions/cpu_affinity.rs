use std::path::Path;

use crate::actions::{
    ActionBoundaryError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
    TuningAction,
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
        token_restore_result,
    },
};

pub(crate) struct CpuAffinityRollbackHandler;

impl RollbackHandler for CpuAffinityRollbackHandler {
    fn id(&self) -> &'static str {
        "cpu-affinity-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(ActionBoundaryError::missing_explicit_rollback_token(
            self.id(),
            "cpu-affinity-restore-file",
        )
        .into())
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(ActionBoundaryError::missing_explicit_rollback_token(
            self.id(),
            "cpu-affinity-restore-file",
        )
        .into())
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_cpu_affinity_restore_file().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "cpu-affinity-restore-file",
                token.kind(),
            )
            .into());
        }
        Ok(token_dry_run_preview(
            self.id(),
            token,
            "cpu-affinity-restore-file",
        ))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let Some((path, _)) = token.as_cpu_affinity_restore_file() else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "cpu-affinity-restore-file",
                token.kind(),
            )
            .into());
        };

        if crate::profile_restore::load_restore_state(path).is_ok() {
            let summary = crate::profile_restore::restore_saved(path)?;
            return Ok(token_restore_result(
                self.id(),
                token,
                summary.restored_total(),
                summary.skipped_dead + summary.skipped_identity_mismatch,
                vec![format!(
                    "affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={} errors={}",
                    summary.affinity,
                    summary.nice,
                    summary.ionice,
                    summary.skipped_dead,
                    summary.skipped_identity_mismatch,
                    summary.errors
                )],
            ));
        }

        let summary = crate::affinity::restore_saved(path)?;
        Ok(token_restore_result(
            self.id(),
            token,
            summary.restored,
            summary.skipped_dead + summary.skipped_identity_mismatch + summary.legacy_unverified,
            vec![format!(
                "restored={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={} errors={}",
                summary.restored,
                summary.skipped_dead,
                summary.skipped_identity_mismatch,
                summary.legacy_unverified,
                summary.errors
            )],
        ))
    }
}

pub struct CpuAffinityProfileAction {
    pub tree_pid: u32,
    pub profile: crate::profiles::Profile,
    pub force_restore_overwrite: bool,
}

impl CpuAffinityProfileAction {
    pub fn descriptor_with_persistent_effect(
        &self,
        persistent_effect: bool,
    ) -> crate::daemon_policy::ActionDescriptor {
        crate::daemon_policy::ActionDescriptor {
            action_id: self.id(),
            action_kind: "cpu_affinity_profile".to_owned(),
            safety_class: self.safety_class(),
            effect_scope: crate::daemon_policy::ActionEffectScope::LocalProcessTree,
            rollback: crate::daemon_policy::RollbackRequirement::RequiredBeforeApply,
            persistent_effect,
            touches_system_wide_state: false,
            requires_explicit_target: true,
            confidence: None,
        }
    }

    pub fn apply_cached_with_policy(
        &self,
        policy: &crate::daemon_policy::DaemonPolicy,
        dry_run: bool,
        cache: crate::profiles::ProfileApplyCache,
        persistent_effect: bool,
    ) -> anyhow::Result<(
        crate::profiles::ProfileApplyResult,
        crate::profiles::ProfileApplyCache,
    )> {
        self.preflight()?;
        let descriptor = self.descriptor_with_persistent_effect(persistent_effect);
        let intent = if dry_run {
            crate::daemon_policy::PolicyIntent::DryRun
        } else {
            crate::daemon_policy::PolicyIntent::Apply
        };
        policy
            .check_action(intent, &descriptor)
            .map_err(|err| anyhow::anyhow!("policy rejected: {err}"))?;

        let mut cache = cache;
        crate::profiles::apply_managed_profile_to_tree_cached(
            self.tree_pid,
            &self.profile,
            self.force_restore_overwrite,
            dry_run,
            &mut cache,
        )
        .map(|result| (result, cache))
    }

    fn preflight_for_restore_path(
        &self,
        restore_path: &Path,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        let mut warnings = Vec::new();
        if self.tree_pid == 0 {
            return Err(ActionBoundaryError::InvalidTargetTid {
                action_kind: "cpu_affinity_profile",
                tid: self.tree_pid,
            }
            .into());
        }
        if self.profile.rules.is_empty() {
            return Err(ActionBoundaryError::InvalidRequest {
                action_kind: "cpu_affinity_profile",
                reason: format!("profile '{}' has no rules", self.profile.name),
            }
            .into());
        }
        if restore_path.exists() && !self.force_restore_overwrite {
            warnings.push(ActionWarning {
                message: format!(
                    "restore file already exists at {}; new affinity records will be merged",
                    restore_path.display()
                ),
            });
        }
        Ok(warnings)
    }
}

impl TuningAction for CpuAffinityProfileAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!("cpu-affinity-profile:{}", self.profile.name))
    }

    fn describe(&self) -> String {
        format!(
            "apply CPU affinity profile '{}' to process tree {}",
            self.profile.name, self.tree_pid
        )
    }

    fn safety_class(&self) -> SafetyClass {
        if crate::profiles::profile_uses_priority_actions(&self.profile) {
            SafetyClass::ReversibleMediumRisk
        } else {
            SafetyClass::ReversibleLowRisk
        }
    }

    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        self.descriptor_with_persistent_effect(false)
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_for_restore_path(&crate::profile_restore::default_restore_path())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        let warnings = self.preflight()?;
        let result = crate::profiles::apply_managed_profile_to_tree(
            self.tree_pid,
            &self.profile,
            self.force_restore_overwrite,
            true,
        )?;
        let summary =
            crate::profiles::profile_apply_summary_for_tree(self.tree_pid, &self.profile)?;
        Ok(ActionState {
            applied: false,
            affected_tasks: result.affected_tasks(),
            checked_tasks: summary.checked_tasks,
            pending_changes: summary.pending_changes,
            warnings,
        })
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        let res: Result<RollbackToken, crate::actions::PartialApplyError> = (|| {
            self.preflight()?;
            let result = crate::profiles::apply_managed_profile_to_tree(
                self.tree_pid,
                &self.profile,
                self.force_restore_overwrite,
                false,
            )?;
            Ok(RollbackToken::CpuAffinityRestoreFile {
                path: crate::profile_restore::default_restore_path(),
                affected_tasks: result.affected_tasks(),
            })
        })();
        res
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        let warnings = self.preflight()?;
        let summary =
            crate::profiles::profile_apply_summary_for_tree(self.tree_pid, &self.profile)?;
        Ok(ActionState {
            applied: summary.checked_tasks > 0 && summary.pending_changes == 0,
            affected_tasks: summary.checked_tasks,
            checked_tasks: summary.checked_tasks,
            pending_changes: summary.pending_changes,
            warnings,
        })
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some((path, _)) = token.as_cpu_affinity_restore_file() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("cpu-affinity-restore-file"),
            )
            .into());
        };
        crate::profile_restore::restore_saved(path)
            .map(|_| ())
            .or_else(|_| crate::affinity::restore_saved(path).map(|_| ()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::{
        actions::TuningAction,
        affinity::CpuMask,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
    };

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn action() -> CpuAffinityProfileAction {
        CpuAffinityProfileAction {
            tree_pid: u32::MAX,
            profile: profile("test-profile"),
            force_restore_overwrite: false,
        }
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-action-test-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn safety_class_is_reversible_low_risk() {
        assert_eq!(action().safety_class(), SafetyClass::ReversibleLowRisk);
    }

    #[test]
    fn preflight_warns_when_restore_file_exists_and_force_is_false() {
        let dir = temp_dir("preflight");
        let restore_path = dir.join("restore.json");
        fs::write(&restore_path, "{}").unwrap();

        let warnings = action().preflight_for_restore_path(&restore_path).unwrap();

        assert!(
            warnings
                .iter()
                .any(|warning| warning.message.contains("restore file already exists"))
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dry_run_does_not_write_restore_file() {
        let dir = temp_dir("dry-run");
        let restore_path = dir.join("restore.json");

        let state = action().dry_run().unwrap();

        assert!(!state.applied);
        assert!(!restore_path.exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn action_id_includes_profile_name() {
        assert_eq!(
            action().id(),
            ActionId::new("cpu-affinity-profile:test-profile".to_owned())
        );
    }

    #[test]
    fn descriptor_can_mark_persistent_effect() {
        let descriptor = action().descriptor_with_persistent_effect(true);

        assert!(descriptor.persistent_effect);
        assert_eq!(descriptor.action_kind, "cpu_affinity_profile");
    }
}
