//! Community-rules data model.
//!
//! Owns serialized rule files, rule-source metadata, user configuration, and load status models.
//! Does not own loading, database lookup, classification, or command rendering.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{CommunityRulesDb, default_user_rules_dir};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommunityRulesFile {
    pub schema_version: u32,
    pub source: CommunityRulesSource,
    pub rules: Vec<CommunityRule>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommunityRulesSource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommunityRule {
    pub name: String,
    pub normalized_name: String,
    pub r#type: String,
    pub stutter_class: String,
    pub confidence: f32,
    pub source_path: String,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default)]
    pub ambiguous: bool,
    #[serde(default)]
    pub specificity: RuleSpecificity,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct RuleSpecificity {
    pub exact_exe: bool,
    pub exact_comm: bool,
    pub regex_count: usize,
    pub wildcard_count: usize,
}

#[derive(Debug, Clone)]
pub enum CommunityRulesSourceKind {
    #[cfg(test)]
    BuiltinFixture,
    UserData,
    SystemData,
    ExplicitPath(PathBuf),
}

#[derive(Debug, Clone)]
pub struct CommunityRulesConfig {
    pub enabled: bool,
    pub load_builtin_fixture: bool,
    pub user_rules_dir: Option<PathBuf>,
    pub explicit_rules_files: Vec<PathBuf>,
}

impl Default for CommunityRulesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            load_builtin_fixture: cfg!(test),
            user_rules_dir: default_user_rules_dir(),
            explicit_rules_files: Vec::new(),
        }
    }
}

impl CommunityRulesConfig {
    pub fn from_config_file(file: crate::config_file::CommunityRulesConfigFile) -> Self {
        let mut config = Self::default();

        if let Some(enabled) = file.enabled {
            config.enabled = enabled;
        }

        config.explicit_rules_files = file.paths.unwrap_or_default();

        if let Some(sources) = file.sources {
            let wants_user = sources
                .iter()
                .any(|source| source.trim().eq_ignore_ascii_case("user"));
            let wants_fixture = sources.iter().any(|source| {
                let source = source.trim();
                source.eq_ignore_ascii_case("fixture")
                    || source.eq_ignore_ascii_case("builtin")
                    || source.eq_ignore_ascii_case("builtin_fixture")
                    || source.eq_ignore_ascii_case("builtin-fixture")
            });

            config.user_rules_dir = if wants_user {
                default_user_rules_dir()
            } else {
                None
            };
            config.load_builtin_fixture = wants_fixture;
        }

        config
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommunityRulesMetadataFile {
    pub schema_version: u32,
    pub name: String,
    pub license: String,
    pub source_repo: Option<String>,
    pub source_commit: Option<String>,
    pub generated_at: String,
    pub generated_by: String,
    pub rule_file: String,
}

#[derive(Debug, Clone)]
pub enum CommunityRulesStatus {
    Loaded { db: CommunityRulesDb },
    Disabled,
    Failed { error: String },
}

impl CommunityRulesStatus {
    pub fn as_db(&self) -> Option<&CommunityRulesDb> {
        match self {
            Self::Loaded { db } => Some(db),
            Self::Disabled | Self::Failed { .. } => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Loaded { .. } => "loaded",
            Self::Disabled => "disabled",
            Self::Failed { .. } => "failed",
        }
    }
}
