use std::path::Path;

use crate::actions::{
    ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction,
};

pub struct CpuAffinityProfileAction {
    pub tree_pid: u32,
    pub profile: crate::profiles::Profile,
    pub force_restore_overwrite: bool,
}

impl CpuAffinityProfileAction {
    pub fn apply_records(
        &self,
        dry_run: bool,
    ) -> anyhow::Result<Vec<crate::affinity::AffinityRecord>> {
        self.preflight()?;
        crate::profiles::apply_profile_to_tree(
            self.tree_pid,
            &self.profile,
            self.force_restore_overwrite,
            dry_run,
        )
    }

    fn preflight_for_restore_path(
        &self,
        restore_path: &Path,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        let mut warnings = Vec::new();
        if self.tree_pid == 0 {
            anyhow::bail!("tree pid must be greater than zero");
        }
        if self.profile.rules.is_empty() {
            anyhow::bail!("profile '{}' has no rules", self.profile.name);
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
        ActionId(format!("cpu-affinity-profile:{}", self.profile.name))
    }

    fn describe(&self) -> String {
        format!(
            "apply CPU affinity profile '{}' to process tree {}",
            self.profile.name, self.tree_pid
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleLowRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_for_restore_path(&crate::affinity::default_restore_path())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        let warnings = self.preflight()?;
        let records = crate::profiles::apply_profile_to_tree(
            self.tree_pid,
            &self.profile,
            self.force_restore_overwrite,
            true,
        )?;
        let summary =
            crate::profiles::profile_apply_summary_for_tree(self.tree_pid, &self.profile)?;
        Ok(ActionState {
            applied: false,
            affected_tasks: records.len(),
            checked_tasks: summary.checked_tasks,
            pending_changes: summary.pending_changes,
            warnings,
        })
    }

    fn apply(&self) -> anyhow::Result<RollbackToken> {
        self.preflight()?;
        let records = crate::profiles::apply_profile_to_tree(
            self.tree_pid,
            &self.profile,
            self.force_restore_overwrite,
            false,
        )?;
        Ok(RollbackToken::CpuAffinityRestoreFile {
            path: crate::affinity::default_restore_path(),
            affected_tasks: records.len(),
        })
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
        let RollbackToken::CpuAffinityRestoreFile { path, .. } = token else {
            anyhow::bail!("rollback token is not a CPU affinity restore file");
        };
        crate::affinity::restore_saved(path)?;
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
                affinity: CpuMask::parse("0").unwrap(),
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
            ActionId("cpu-affinity-profile:test-profile".to_owned())
        );
    }
}
