#![allow(dead_code)]

use std::{collections::BTreeSet, path::Path};

use crate::{
    actions::{ActionWarning, SafetyClass, TuningAction, cpu_affinity::CpuAffinityProfileAction},
    profiles::Profile,
};

#[derive(Clone, Debug)]
pub enum CandidateAction {
    CpuAffinityProfile {
        profile_name: String,
        profile: Profile,
        tree_pid: u32,
    },
    #[cfg(test)]
    Fake {
        action_id: crate::actions::ActionId,
        safety_class: crate::actions::SafetyClass,
    },
}

impl CandidateAction {
    pub fn cpu_affinity_profile(profile: Profile, tree_pid: u32) -> Self {
        Self::CpuAffinityProfile {
            profile_name: profile.name.clone(),
            profile,
            tree_pid,
        }
    }

    pub fn profile_name(&self) -> &str {
        match self {
            Self::CpuAffinityProfile { profile_name, .. } => profile_name,
            #[cfg(test)]
            Self::Fake { .. } => "fake-profile",
        }
    }

    pub fn tree_pid(&self) -> u32 {
        match self {
            Self::CpuAffinityProfile { tree_pid, .. } => *tree_pid,
            #[cfg(test)]
            Self::Fake { .. } => 0,
        }
    }

    pub fn action_kind(&self) -> &'static str {
        match self {
            Self::CpuAffinityProfile { .. } => "cpu_affinity_profile",
            #[cfg(test)]
            Self::Fake { .. } => "fake",
        }
    }

    pub fn safety_class(&self) -> crate::actions::SafetyClass {
        match self {
            Self::CpuAffinityProfile { .. } => crate::actions::SafetyClass::ReversibleLowRisk,
            #[cfg(test)]
            Self::Fake { safety_class, .. } => safety_class.clone(),
        }
    }

    pub fn action_id(&self) -> crate::actions::ActionId {
        match self {
            Self::CpuAffinityProfile { profile_name, .. } => {
                crate::actions::ActionId(format!("cpu-affinity-profile:{}", profile_name))
            }
            #[cfg(test)]
            Self::Fake { action_id, .. } => action_id.clone(),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::CpuAffinityProfile {
                profile_name,
                tree_pid,
                ..
            } => {
                format!(
                    "apply CPU affinity profile '{}' to process tree {}",
                    profile_name, tree_pid
                )
            }
            #[cfg(test)]
            Self::Fake { action_id, .. } => format!("fake action {}", action_id.0),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneratedProfileCandidatePlan {
    pub optimization_candidates: Vec<CandidateAction>,
    pub recovery_fallback: Option<CandidateAction>,
    pub rejected: Vec<RejectedCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedCandidateProfile {
    pub profile_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateProfileStatus {
    pub matched_tasks: usize,
    pub dry_run_tasks: usize,
}

#[derive(Clone, Debug)]
pub struct CandidateDryRunRecord {
    pub candidate_name: String,
    pub affected_tasks: usize,
    pub warnings: Vec<ActionWarning>,
    pub safety_class: SafetyClass,
    pub eligible: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateSuggestion {
    pub candidate: String,
    pub action: String,
    pub affected_tasks: usize,
    pub safety: SafetyClass,
    pub reason: String,
    pub apply_command: String,
}

pub fn suggestion_from_dry_run_record(
    record: &CandidateDryRunRecord,
    tree_pid: u32,
    profile_path: Option<&Path>,
    reason: impl Into<String>,
) -> Option<CandidateSuggestion> {
    if !record.eligible {
        return None;
    }

    let profile_arg = profile_path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<generated-or-existing-profile>".to_owned());

    Some(CandidateSuggestion {
        candidate: record.candidate_name.clone(),
        action: "cpu-affinity-profile".to_owned(),
        affected_tasks: record.affected_tasks,
        safety: record.safety_class.clone(),
        reason: reason.into(),
        apply_command: format!(
            "stutter apply-profile --tree-pid {} --profile {}",
            tree_pid, profile_arg
        ),
    })
}

pub fn suggestions_from_dry_run_records(
    records: &[CandidateDryRunRecord],
    tree_pid: u32,
    profile_path: Option<&Path>,
    reason: &str,
) -> Vec<CandidateSuggestion> {
    records
        .iter()
        .filter_map(|record| {
            suggestion_from_dry_run_record(record, tree_pid, profile_path, reason.to_owned())
        })
        .collect()
}

pub fn render_candidate_suggestion(suggestion: &CandidateSuggestion) -> String {
    format!(
        "autotune suggestion:\n  candidate={}\n  action={}\n  affected_tasks={}\n  safety={:?}\n  reason=\"{}\"\n  apply_command=\"{}\"",
        shell_safe_value(&suggestion.candidate),
        shell_safe_value(&suggestion.action),
        suggestion.affected_tasks,
        suggestion.safety,
        escape_quoted_value(&suggestion.reason),
        escape_quoted_value(&suggestion.apply_command)
    )
}

pub fn render_candidate_suggestions(suggestions: &[CandidateSuggestion]) -> String {
    suggestions
        .iter()
        .map(render_candidate_suggestion)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn print_candidate_suggestions(suggestions: &[CandidateSuggestion]) {
    for suggestion in suggestions {
        println!("{}", render_candidate_suggestion(suggestion));
    }
}

fn shell_safe_value(value: &str) -> String {
    if value.is_empty() {
        return "-".to_owned();
    }

    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/' | '@'))
    {
        value.to_owned()
    } else {
        format!("\"{}\"", escape_quoted_value(value))
    }
}

fn escape_quoted_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }

    escaped
}

pub fn dry_run_candidates(candidates: &[CandidateAction]) -> Vec<CandidateDryRunRecord> {
    candidates.iter().map(dry_run_candidate).collect()
}

pub fn dry_run_candidate(candidate: &CandidateAction) -> CandidateDryRunRecord {
    match candidate {
        CandidateAction::CpuAffinityProfile {
            profile_name,
            profile,
            tree_pid,
        } => {
            let action = CpuAffinityProfileAction {
                tree_pid: *tree_pid,
                profile: profile.clone(),
                force_restore_overwrite: false,
            };
            let safety_class = action.safety_class();

            match action.dry_run() {
                Ok(state) => {
                    let affected_tasks = state.affected_tasks;
                    CandidateDryRunRecord {
                        candidate_name: profile_name.clone(),
                        affected_tasks,
                        warnings: state.warnings,
                        safety_class,
                        eligible: affected_tasks > 0,
                        reason: if affected_tasks == 0 {
                            Some("dry-run matched zero affected tasks".to_owned())
                        } else {
                            None
                        },
                    }
                }
                Err(err) => CandidateDryRunRecord {
                    candidate_name: profile_name.clone(),
                    affected_tasks: 0,
                    warnings: Vec::new(),
                    safety_class,
                    eligible: false,
                    reason: Some(format!("dry-run failed: {err:#}")),
                },
            }
        }
        #[cfg(test)]
        CandidateAction::Fake { .. } => {
            panic!("dry-run not implemented for Fake candidate");
        }
    }
}

pub fn generate_profile_candidates(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
) -> Vec<CandidateAction> {
    generate_profile_candidate_plan(profiles, tree_pid, current_profile).optimization_candidates
}

pub fn generate_profile_candidate_plan(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
) -> GeneratedProfileCandidatePlan {
    let recently_failed_profiles = BTreeSet::new();
    generate_profile_candidate_plan_with_history(
        profiles,
        tree_pid,
        current_profile,
        &recently_failed_profiles,
    )
}

pub fn generate_profile_candidate_plan_with_history(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
    recently_failed_profiles: &BTreeSet<String>,
) -> GeneratedProfileCandidatePlan {
    generate_profile_candidate_plan_with_checker(
        profiles,
        tree_pid,
        current_profile,
        recently_failed_profiles,
        |profile| check_profile_for_candidate(tree_pid, profile),
    )
}

fn generate_profile_candidate_plan_with_checker<F>(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
    recently_failed_profiles: &BTreeSet<String>,
    mut check_profile: F,
) -> GeneratedProfileCandidatePlan
where
    F: FnMut(&Profile) -> anyhow::Result<CandidateProfileStatus>,
{
    let mut preferred_candidates = Vec::new();
    let mut recently_failed_candidates = Vec::new();
    let mut recovery_fallback = None;
    let mut rejected = Vec::new();

    for profile in profiles {
        if Some(profile.name.as_str()) == current_profile {
            rejected.push(RejectedCandidateProfile {
                profile_name: profile.name.clone(),
                reason: "current profile".to_owned(),
            });
            continue;
        }

        let status = match check_profile(profile) {
            Ok(status) => status,
            Err(err) => {
                rejected.push(RejectedCandidateProfile {
                    profile_name: profile.name.clone(),
                    reason: format!("dry-run failed: {err:#}"),
                });
                continue;
            }
        };

        if status.matched_tasks == 0 {
            rejected.push(RejectedCandidateProfile {
                profile_name: profile.name.clone(),
                reason: "zero matched tasks".to_owned(),
            });
            continue;
        }

        let candidate = CandidateAction::cpu_affinity_profile(profile.clone(), tree_pid);

        if is_baseline_online_profile(&profile.name) {
            recovery_fallback = Some(candidate);
            continue;
        }

        if recently_failed_profiles.contains(&profile.name) {
            recently_failed_candidates.push(candidate);
        } else {
            preferred_candidates.push(candidate);
        }
    }

    preferred_candidates.extend(recently_failed_candidates);

    GeneratedProfileCandidatePlan {
        optimization_candidates: preferred_candidates,
        recovery_fallback,
        rejected,
    }
}

fn check_profile_for_candidate(
    tree_pid: u32,
    profile: &Profile,
) -> anyhow::Result<CandidateProfileStatus> {
    let matched_tasks = crate::profiles::profile_matched_task_count_for_tree(tree_pid, profile);
    let dry_run_records = crate::profiles::apply_profile_to_tree(tree_pid, profile, false, true)?;

    Ok(CandidateProfileStatus {
        matched_tasks,
        dry_run_tasks: dry_run_records.len(),
    })
}

fn is_baseline_online_profile(profile_name: &str) -> bool {
    profile_name == "baseline-online"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{affinity::CpuMask, process_tree::TaskClass, profiles::ProfileRule};

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

    fn status_for_profile(profile: &Profile) -> anyhow::Result<CandidateProfileStatus> {
        match profile.name.as_str() {
            "dry-run-fails" => anyhow::bail!("intentional dry-run failure"),
            "zero-match" => Ok(CandidateProfileStatus {
                matched_tasks: 0,
                dry_run_tasks: 0,
            }),
            _ => Ok(CandidateProfileStatus {
                matched_tasks: 2,
                dry_run_tasks: 1,
            }),
        }
    }

    #[test]
    fn generate_profile_candidates_excludes_current_profile() {
        let profiles = vec![profile("current"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            Some("current"),
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "current"
                    && rejected.reason == "current profile")
        );
    }

    #[test]
    fn generate_profile_candidates_excludes_profiles_that_fail_dry_run() {
        let profiles = vec![profile("dry-run-fails"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "dry-run-fails"
                    && rejected.reason.contains("dry-run failed"))
        );
    }

    #[test]
    fn generate_profile_candidates_excludes_zero_matched_tasks() {
        let profiles = vec![profile("zero-match"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "zero-match"
                    && rejected.reason == "zero matched tasks")
        );
    }

    #[test]
    fn generate_profile_candidates_puts_recently_failed_names_last() {
        let profiles = vec![
            profile("recently-failed"),
            profile("fresh"),
            profile("another-fresh"),
        ];
        let recently_failed_profiles = BTreeSet::from(["recently-failed".to_owned()]);

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &recently_failed_profiles,
            status_for_profile,
        );

        let names = plan
            .optimization_candidates
            .iter()
            .map(CandidateAction::profile_name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["fresh", "another-fresh", "recently-failed"]);
    }

    #[test]
    fn baseline_online_is_recovery_fallback_not_optimization_candidate() {
        let profiles = vec![profile("baseline-online"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert_eq!(
            plan.recovery_fallback
                .as_ref()
                .map(CandidateAction::profile_name),
            Some("baseline-online")
        );
    }

    #[test]
    fn public_generate_profile_candidates_returns_optimization_candidates_only() {
        let profiles = vec![profile("baseline-online"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(plan.recovery_fallback.is_some());
    }

    #[test]
    fn suggestion_from_dry_run_record_renders_requested_shape() {
        let record = CandidateDryRunRecord {
            candidate_name: "game-main-suggested".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            "scheduler pressure detected on Game/WineServer classes",
        )
        .unwrap();

        let rendered = render_candidate_suggestion(&suggestion);

        assert_eq!(
            rendered,
            "autotune suggestion:\n  candidate=game-main-suggested\n  action=cpu-affinity-profile\n  affected_tasks=31\n  safety=ReversibleLowRisk\n  reason=\"scheduler pressure detected on Game/WineServer classes\"\n  apply_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile>\""
        );
    }

    #[test]
    fn suggestion_from_dry_run_record_uses_existing_profile_path_when_available() {
        let record = CandidateDryRunRecord {
            candidate_name: "game-main-suggested".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            Some(Path::new("/tmp/profiles.toml")),
            "scheduler pressure detected on Game/WineServer classes",
        )
        .unwrap();

        assert_eq!(
            suggestion.apply_command,
            "stutter apply-profile --tree-pid 1234 --profile /tmp/profiles.toml"
        );
    }

    #[test]
    fn suggestion_from_dry_run_record_skips_ineligible_candidate() {
        let record = CandidateDryRunRecord {
            candidate_name: "bad-candidate".to_owned(),
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: false,
            reason: Some("dry-run matched zero affected tasks".to_owned()),
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_none());
    }

    #[test]
    fn render_candidate_suggestion_escapes_reason_and_apply_command() {
        let suggestion = CandidateSuggestion {
            candidate: "candidate with space".to_owned(),
            action: "cpu-affinity-profile".to_owned(),
            affected_tasks: 31,
            safety: SafetyClass::ReversibleLowRisk,
            reason: "scheduler \"pressure\"\nnext".to_owned(),
            apply_command:
                "stutter apply-profile --tree-pid 1234 --profile /tmp/profile \"quoted\".toml"
                    .to_owned(),
        };

        let rendered = render_candidate_suggestion(&suggestion);

        assert_eq!(
            rendered,
            "autotune suggestion:\n  candidate=\"candidate with space\"\n  action=cpu-affinity-profile\n  affected_tasks=31\n  safety=ReversibleLowRisk\n  reason=\"scheduler \\\"pressure\\\"\\nnext\"\n  apply_command=\"stutter apply-profile --tree-pid 1234 --profile /tmp/profile \\\"quoted\\\".toml\""
        );
    }

    #[test]
    fn dry_run_candidate_records_failure_as_ineligible() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("bad-tree"), 0);

        let record = dry_run_candidate(&candidate);

        assert_eq!(record.candidate_name, "bad-tree");
        assert_eq!(record.affected_tasks, 0);
        assert_eq!(record.safety_class, SafetyClass::ReversibleLowRisk);
        assert!(!record.eligible);
        assert!(
            record
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("dry-run failed")
        );
        assert!(
            record
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("tree pid must be greater than zero")
        );
    }

    #[test]
    fn dry_run_candidates_preserves_candidate_order() {
        let candidates = vec![
            CandidateAction::cpu_affinity_profile(profile("first"), 0),
            CandidateAction::cpu_affinity_profile(profile("second"), 0),
        ];

        let records = dry_run_candidates(&candidates);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].candidate_name, "first");
        assert_eq!(records[1].candidate_name, "second");
    }

    #[test]
    fn candidate_helpers_return_stable_metadata() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);

        assert_eq!(candidate.profile_name(), "game-main");
        assert_eq!(candidate.tree_pid(), 1234);
        assert_eq!(candidate.action_kind(), "cpu_affinity_profile");
    }
}
