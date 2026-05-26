//! Community-rules in-memory database.
//!
//! Owns rule indexing, file merging, and database construction. Does not own loading paths,
//! process classification policy, normalization definitions, or command output.

use std::collections::HashMap;

use super::{
    CommunityRule, CommunityRulesFile, is_guarded_community_rule_name, normalize_process_name,
};

#[derive(Debug, Clone)]
pub struct CommunityRulesDb {
    pub(super) rules_by_name: HashMap<String, Vec<CommunityRule>>,
}

impl CommunityRulesDb {
    pub fn empty() -> Self {
        Self {
            rules_by_name: HashMap::new(),
        }
    }

    pub fn rule_count(&self) -> usize {
        self.rules_by_name.values().map(Vec::len).sum()
    }

    pub fn from_files(files: Vec<CommunityRulesFile>) -> anyhow::Result<Self> {
        let mut db = Self::empty();
        for file in files {
            db.merge_file(file)?;
        }
        Ok(db)
    }

    pub fn merge_file(&mut self, file: CommunityRulesFile) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(file.schema_version, 1 | 2),
            "unsupported community rules schema version {}",
            file.schema_version
        );

        for mut rule in file.rules {
            if rule.normalized_name.trim().is_empty() {
                rule.normalized_name =
                    normalize_process_name(&rule.name).unwrap_or_else(|| rule.name.clone());
            }

            if is_guarded_community_rule_name(&rule.normalized_name) {
                rule.ambiguous = true;
            }

            if let Some(existing_rules) = self.rules_by_name.get(&rule.normalized_name) {
                for existing in existing_rules {
                    if existing.stutter_class != rule.stutter_class {
                        log::warn!(
                            "community rule conflict: '{}' maps to both '{}' (from {}) and '{}' (from {})",
                            rule.normalized_name,
                            existing.stutter_class,
                            existing.source_path,
                            rule.stutter_class,
                            rule.source_path
                        );
                    }
                }
            }

            self.rules_by_name
                .entry(rule.normalized_name.clone())
                .or_default()
                .push(rule);
        }

        Ok(())
    }

    pub fn from_json(data: &str) -> anyhow::Result<Self> {
        let file: CommunityRulesFile = serde_json::from_str(data)?;
        Self::from_file(file)
    }

    pub fn from_file(file: CommunityRulesFile) -> anyhow::Result<Self> {
        let mut db = Self::empty();
        db.merge_file(file)?;
        Ok(db)
    }
}
