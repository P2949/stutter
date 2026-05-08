#![allow(dead_code)]

use std::{collections::HashMap, path::Path, sync::OnceLock};

use serde::Deserialize;

use crate::process_tree::TaskClass;

const BUILTIN_ANANICY_RULES_JSON: &str =
    include_str!("../assets/community-rules/ananicy.generated.json");

#[derive(Debug, Clone, Deserialize)]
pub struct CommunityRulesFile {
    pub schema_version: u32,
    pub source: CommunityRulesSource,
    pub rules: Vec<CommunityRule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommunityRulesSource {
    pub name: String,
    pub repo: String,
    pub commit: String,
    pub generated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommunityRule {
    pub name: String,
    pub normalized_name: String,
    pub r#type: String,
    pub stutter_class: String,
    pub confidence: f32,
    pub source_path: String,
    #[serde(default)]
    pub context: Vec<String>,
    pub title: Option<String>,
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommunityRuleIdentitySource {
    ExeBasename,
    CmdlineBasename,
    ProcessComm,
    ThreadComm,
}

impl CommunityRuleIdentitySource {
    fn confidence_cap(self) -> f32 {
        match self {
            Self::ExeBasename => 0.90,
            Self::CmdlineBasename => 0.88,
            Self::ProcessComm | Self::ThreadComm => 0.75,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ExeBasename => "exe basename",
            Self::CmdlineBasename => "cmdline basename",
            Self::ProcessComm => "process comm",
            Self::ThreadComm => "thread comm",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CommunityProcessIdentity<'a> {
    pub thread_comm: &'a str,
    pub process_comm: &'a str,
    pub cmdline: &'a str,
    pub exe_path: &'a str,
    pub cgroup_path: &'a str,
}

#[derive(Debug, Clone)]
pub struct CommunityRuleHit {
    pub class: TaskClass,
    pub confidence: f32,
    pub rule_name: String,
    pub source_path: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CommunityRulesDb {
    rules_by_name: HashMap<String, Vec<CommunityRule>>,
}

static BUILTIN_RULES: OnceLock<CommunityRulesDb> = OnceLock::new();

pub fn classify_process_identity(
    identity: &CommunityProcessIdentity<'_>,
) -> Option<CommunityRuleHit> {
    builtin_rules().classify(identity, true)
}

fn builtin_rules() -> &'static CommunityRulesDb {
    BUILTIN_RULES.get_or_init(|| {
        CommunityRulesDb::from_json(BUILTIN_ANANICY_RULES_JSON)
            .expect("embedded community rules JSON must be valid")
    })
}

impl CommunityRulesDb {
    pub fn from_json(data: &str) -> anyhow::Result<Self> {
        let file: CommunityRulesFile = serde_json::from_str(data)?;
        anyhow::ensure!(
            file.schema_version == 1,
            "unsupported community rules schema version {}",
            file.schema_version
        );

        let mut rules_by_name: HashMap<String, Vec<CommunityRule>> = HashMap::new();
        for mut rule in file.rules {
            if rule.normalized_name.trim().is_empty() {
                rule.normalized_name =
                    normalize_process_name(&rule.name).unwrap_or_else(|| rule.name.clone());
            }

            rules_by_name
                .entry(rule.normalized_name.clone())
                .or_default()
                .push(rule);
        }

        Ok(Self { rules_by_name })
    }

    pub fn classify(
        &self,
        identity: &CommunityProcessIdentity<'_>,
        strict_context: bool,
    ) -> Option<CommunityRuleHit> {
        let candidates = identity_candidates(identity);
        for (candidate, source) in candidates {
            let Some(rules) = self.rules_by_name.get(&candidate) else {
                continue;
            };

            for rule in rules {
                let Some(class) = TaskClass::from_str_opt(&rule.stutter_class) else {
                    continue;
                };
                if class != TaskClass::Game {
                    continue;
                }

                let context_signal = game_context_signal(identity);
                if strict_context && rule_requires_context(rule) && context_signal.is_none() {
                    continue;
                }
                if rule.ambiguous && context_signal.is_none() {
                    continue;
                }

                let confidence_cap = if rule.ambiguous {
                    source.confidence_cap().min(0.70)
                } else {
                    source.confidence_cap()
                };
                let confidence = rule.confidence.min(confidence_cap);
                let context_label = context_signal.unwrap_or("exact-name");
                let reason = format!(
                    "community-rules: matched Ananicy rule '{}' from {}; via {}; context={}",
                    rule.name,
                    rule.source_path,
                    source.label(),
                    context_label
                );

                return Some(CommunityRuleHit {
                    class,
                    confidence,
                    rule_name: rule.name.clone(),
                    source_path: rule.source_path.clone(),
                    reason,
                });
            }
        }

        None
    }
}

pub fn normalize_process_name(value: &str) -> Option<String> {
    let mut value = value.trim();
    while let Some(stripped) = value.strip_suffix(" (deleted)") {
        value = stripped.trim_end();
    }

    value = value.trim_matches('"').trim_matches('\'');
    if value.is_empty() {
        return None;
    }

    let slash_normalized = value.replace('\\', "/");
    let basename = Path::new(&slash_normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(slash_normalized.as_str())
        .trim();

    if basename.is_empty() {
        None
    } else {
        Some(basename.to_ascii_lowercase())
    }
}

fn identity_candidates(
    identity: &CommunityProcessIdentity<'_>,
) -> Vec<(String, CommunityRuleIdentitySource)> {
    let mut candidates = Vec::new();
    push_candidate(
        &mut candidates,
        identity.exe_path,
        CommunityRuleIdentitySource::ExeBasename,
    );
    if let Some(first_arg) = first_cmdline_arg(identity.cmdline) {
        push_candidate(
            &mut candidates,
            first_arg,
            CommunityRuleIdentitySource::CmdlineBasename,
        );
    }
    push_candidate(
        &mut candidates,
        identity.process_comm,
        CommunityRuleIdentitySource::ProcessComm,
    );
    push_candidate(
        &mut candidates,
        identity.thread_comm,
        CommunityRuleIdentitySource::ThreadComm,
    );
    candidates
}

fn push_candidate(
    candidates: &mut Vec<(String, CommunityRuleIdentitySource)>,
    value: &str,
    source: CommunityRuleIdentitySource,
) {
    let Some(normalized) = normalize_process_name(value) else {
        return;
    };
    if candidates
        .iter()
        .any(|(candidate, _)| candidate == &normalized)
    {
        return;
    }
    candidates.push((normalized, source));
}

fn first_cmdline_arg(cmdline: &str) -> Option<&str> {
    if cmdline.contains('\0') {
        return cmdline.split('\0').find(|arg| !arg.trim().is_empty());
    }

    cmdline
        .split_whitespace()
        .find(|arg| !arg.trim().is_empty())
}

fn rule_requires_context(rule: &CommunityRule) -> bool {
    rule.ambiguous
        || rule
            .context
            .iter()
            .any(|context| context == "wine_or_proton_or_steam")
        || rule
            .source_path
            .to_ascii_lowercase()
            .contains("wine_proton")
}

fn game_context_signal(identity: &CommunityProcessIdentity<'_>) -> Option<&'static str> {
    let cmdline = identity.cmdline.to_ascii_lowercase();
    let exe_path = identity.exe_path.to_ascii_lowercase();
    let cgroup_path = identity.cgroup_path.to_ascii_lowercase();
    let process_comm = identity.process_comm.to_ascii_lowercase();
    let thread_comm = identity.thread_comm.to_ascii_lowercase();

    let combined = [
        cmdline.as_str(),
        exe_path.as_str(),
        cgroup_path.as_str(),
        process_comm.as_str(),
        thread_comm.as_str(),
    ]
    .join(" ");

    if combined.contains("steamapps/") || combined.contains("\\steamapps\\") {
        Some("steamapps")
    } else if combined.contains("compatdata/") || combined.contains("\\compatdata\\") {
        Some("compatdata")
    } else if combined.contains("app-steam") {
        Some("app-steam")
    } else if combined.contains("pressure-vessel") {
        Some("pressure-vessel")
    } else if combined.contains("pv-bwrap") {
        Some("pv-bwrap")
    } else if combined.contains("gamescope") {
        Some("gamescope")
    } else if combined.contains("wineserver") {
        Some("wineserver")
    } else if combined.contains("proton") {
        Some("proton")
    } else if combined.contains("wine") {
        Some("wine")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity<'a>(
        thread_comm: &'a str,
        process_comm: &'a str,
        cmdline: &'a str,
        exe_path: &'a str,
        cgroup_path: &'a str,
    ) -> CommunityProcessIdentity<'a> {
        CommunityProcessIdentity {
            thread_comm,
            process_comm,
            cmdline,
            exe_path,
            cgroup_path,
        }
    }

    #[test]
    fn normalize_basename_is_case_insensitive_and_strips_deleted_suffix() {
        assert_eq!(
            normalize_process_name("/games/KingdomCome.EXE (deleted)").as_deref(),
            Some("kingdomcome.exe")
        );
        assert_eq!(
            normalize_process_name(r#"C:\Games\Build.EXE"#).as_deref(),
            Some("build.exe")
        );
    }

    #[test]
    fn exact_exe_basename_match_classifies_game_with_context() {
        let hit = classify_process_identity(&identity(
            "KingdomCome.exe",
            "KingdomCome.exe",
            "/usr/bin/wine KingdomCome.exe",
            "/home/me/.steam/steamapps/common/KingdomCome/KingdomCome.exe",
            "/user.slice/app-steam-379430.scope",
        ))
        .unwrap();

        assert_eq!(hit.class, TaskClass::Game);
        assert!(hit.reason.contains("wine_proton_k.rules"));
    }

    #[test]
    fn case_insensitive_cmdline_basename_match_works_when_comm_is_truncated() {
        let hit = classify_process_identity(&identity(
            "KingdomCome",
            "KingdomCome",
            "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KINGDOMCOME.EXE --arg",
            "/usr/bin/wine",
            "/user.slice/app-steam-379430.scope",
        ))
        .unwrap();

        assert_eq!(hit.class, TaskClass::Game);
        assert!(hit.confidence <= 0.88);
        assert!(hit.reason.contains("cmdline basename"));
    }

    #[test]
    fn ambiguous_rule_without_context_does_not_match() {
        let hit = classify_process_identity(&identity(
            "build.exe",
            "build.exe",
            "/tmp/build.exe",
            "/tmp/build.exe",
            "/user.slice/app-builder.scope",
        ));

        assert!(hit.is_none());
    }

    #[test]
    fn ambiguous_rule_with_compatdata_context_can_match() {
        let hit = classify_process_identity(&identity(
            "build.exe",
            "build.exe",
            "/home/me/.steam/steamapps/compatdata/123/pfx/drive_c/build.exe",
            "/usr/bin/wine",
            "/user.slice/app-steam-123.scope",
        ))
        .unwrap();

        assert_eq!(hit.class, TaskClass::Game);
        assert!(hit.confidence <= 0.70);
    }
}
