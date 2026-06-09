//! Community-rules loading orchestration.
//!
//! Owns conversion from user configuration or source selection into loaded rule files/databases.
//! Does not own low-level directory scanning, path defaults, command handling, or classification.

#[cfg(test)]
use std::path::Path;

use super::{
    CommunityRulesConfig, CommunityRulesDb, CommunityRulesFile, CommunityRulesSourceKind,
    CommunityRulesStatus,
    loader::{LoadCommunityRulesInput, load_rules_db, load_rules_dir, load_rules_file},
    paths::{default_system_rules_dirs, default_user_rules_dir},
};

pub fn load_community_rules_status(config: &CommunityRulesConfig) -> CommunityRulesStatus {
    if !config.enabled {
        return CommunityRulesStatus::Disabled;
    }

    match load_community_rules(config) {
        Ok(db) => CommunityRulesStatus::Loaded { db },
        Err(error) => CommunityRulesStatus::Failed {
            error: format!("{error:#}"),
        },
    }
}

pub fn load_community_rules(config: &CommunityRulesConfig) -> anyhow::Result<CommunityRulesDb> {
    load_rules_db(LoadCommunityRulesInput {
        enabled: config.enabled,
        load_test_fixture: config.load_builtin_fixture,
        user_rules_dir: config.user_rules_dir.clone(),
        explicit_rules_files: config.explicit_rules_files.clone(),
        system_rules_dirs: default_system_rules_dirs(),
    })
}

pub fn load_community_rules_file(
    source: CommunityRulesSourceKind,
) -> anyhow::Result<CommunityRulesFile> {
    match source {
        #[cfg(test)]
        CommunityRulesSourceKind::BuiltinFixture => {
            load_rules_file(Path::new("__stutter_test_fixture__"))
        }
        CommunityRulesSourceKind::UserData => {
            let dir = default_user_rules_dir().ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot locate user community rules directory because neither XDG_DATA_HOME nor HOME is set"
                )
            })?;
            let mut files = load_rules_dir(&dir)?;
            files.drain(..).next().ok_or_else(|| {
                anyhow::anyhow!("no user community rules files found in {}", dir.display())
            })
        }
        CommunityRulesSourceKind::SystemData => {
            let mut files = Vec::new();
            for dir in default_system_rules_dirs() {
                files.extend(load_rules_dir(&dir)?);
            }
            files
                .drain(..)
                .next()
                .ok_or_else(|| anyhow::anyhow!("no system community rules files found"))
        }
        CommunityRulesSourceKind::ExplicitPath(path) => load_rules_file(&path),
    }
}

pub fn load_community_rules_db(
    source: CommunityRulesSourceKind,
) -> anyhow::Result<CommunityRulesDb> {
    CommunityRulesDb::from_file(load_community_rules_file(source)?)
}
