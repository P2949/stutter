//! Community-rules process classification.
//!
//! Owns matching process identity signals against the community-rules database and applying
//! context/confidence policy. Does not own database construction, loading, or normalization policy.

#[cfg(test)]
use std::sync::OnceLock;

use super::{
    CommunityRule, CommunityRulesDb, CommunityRulesSourceKind, load_community_rules_db,
    normalize_process_name,
};
use crate::process_tree::TaskClass;

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
            Self::ProcessComm => 0.75,
            Self::ThreadComm => 0.65,
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

pub fn classify_process_identity_with_db(
    db: &CommunityRulesDb,
    identity: &CommunityProcessIdentity<'_>,
) -> Option<CommunityRuleHit> {
    db.classify(identity, true)
}

#[cfg(test)]
static TEST_FIXTURE_RULES: OnceLock<CommunityRulesDb> = OnceLock::new();

#[cfg(test)]
pub fn classify_process_identity(
    identity: &CommunityProcessIdentity<'_>,
) -> Option<CommunityRuleHit> {
    test_fixture_rules().classify(identity, true)
}

#[cfg(test)]
fn test_fixture_rules() -> &'static CommunityRulesDb {
    TEST_FIXTURE_RULES.get_or_init(|| {
        load_community_rules_db(CommunityRulesSourceKind::BuiltinFixture).unwrap_or_else(|error| {
            panic!("embedded community rules test fixture JSON must be valid: {error:#}")
        })
    })
}

impl CommunityRulesDb {
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

                let Some(context_label) =
                    classification_context_label(class, rule, identity, source, strict_context)
                else {
                    continue;
                };

                let confidence_cap = confidence_cap_for_rule(class, rule, source);
                let confidence = rule.confidence.min(confidence_cap);
                let reason = format!(
                    "community-rules: matched community rule '{}' from {}; via {}; context={}",
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

fn classification_context_label(
    class: TaskClass,
    rule: &CommunityRule,
    identity: &CommunityProcessIdentity<'_>,
    source: CommunityRuleIdentitySource,
    strict_context: bool,
) -> Option<&'static str> {
    match class {
        TaskClass::Unknown => None,
        TaskClass::Game => game_rule_context_label(rule, identity, strict_context),
        TaskClass::GameScope | TaskClass::WineServer | TaskClass::SteamRuntime => {
            gaming_runtime_rule_context_label(identity, strict_context)
        }
        _ => non_game_rule_context_label(rule, source),
    }
}

fn game_rule_context_label(
    rule: &CommunityRule,
    identity: &CommunityProcessIdentity<'_>,
    strict_context: bool,
) -> Option<&'static str> {
    let context_signal = game_context_signal(identity);

    if strict_context && rule_requires_context(rule) && context_signal.is_none() {
        return None;
    }

    if rule.ambiguous && context_signal.is_none() {
        return None;
    }

    Some(context_signal.unwrap_or("exact-name"))
}

fn gaming_runtime_rule_context_label(
    identity: &CommunityProcessIdentity<'_>,
    strict_context: bool,
) -> Option<&'static str> {
    let context_signal = gaming_runtime_context_signal(identity);

    if strict_context && context_signal.is_none() {
        return None;
    }

    Some(context_signal.unwrap_or("exact-name"))
}

fn non_game_rule_context_label(
    rule: &CommunityRule,
    source: CommunityRuleIdentitySource,
) -> Option<&'static str> {
    if rule.ambiguous {
        return None;
    }

    match source {
        CommunityRuleIdentitySource::ExeBasename => Some("exact-exe"),
        CommunityRuleIdentitySource::CmdlineBasename => Some("exact-cmdline"),
        CommunityRuleIdentitySource::ProcessComm | CommunityRuleIdentitySource::ThreadComm => None,
    }
}

fn confidence_cap_for_rule(
    class: TaskClass,
    rule: &CommunityRule,
    source: CommunityRuleIdentitySource,
) -> f32 {
    let mut cap = source.confidence_cap();

    if rule.ambiguous {
        cap = cap.min(0.70);
    }

    match class {
        TaskClass::Unknown => 0.0,
        TaskClass::Game => cap,
        TaskClass::GameScope | TaskClass::WineServer | TaskClass::SteamRuntime => cap,
        TaskClass::Service | TaskClass::NetworkDaemon | TaskClass::StorageDaemon => {
            if service_rule_source_path_is_specific(rule) {
                cap.min(0.80)
            } else {
                cap.min(0.60)
            }
        }
        _ => cap.min(0.80),
    }
}

fn service_rule_source_path_is_specific(rule: &CommunityRule) -> bool {
    let source_path = rule.source_path.to_ascii_lowercase();

    if source_path.contains("systemd")
        || source_path.contains("dbus")
        || source_path.contains("network")
        || source_path.contains("storage")
        || source_path.contains("daemon")
        || source_path.contains("service")
    {
        return true;
    }

    source_path
        .split('/')
        .filter(|component| !component.trim().is_empty())
        .count()
        >= 3
}

pub(crate) fn rule_requires_context(rule: &CommunityRule) -> bool {
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

fn gaming_runtime_context_signal(identity: &CommunityProcessIdentity<'_>) -> Option<&'static str> {
    if let Some(signal) = game_context_signal(identity) {
        return Some(signal);
    }

    let combined = [
        identity.cmdline,
        identity.exe_path,
        identity.cgroup_path,
        identity.process_comm,
        identity.thread_comm,
    ]
    .join(" ")
    .to_ascii_lowercase();

    if combined.contains("steam-runtime") {
        Some("steam-runtime")
    } else if combined.contains("steamrt") {
        Some("steamrt")
    } else if combined.contains("steam-runtime-tools") {
        Some("steam-runtime-tools")
    } else {
        None
    }
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
