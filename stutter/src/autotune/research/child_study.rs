//! Frozen child-study plan (PLAN section 1 nesting and section 6.3 funnel). One authorized
//! campaign is split into homogeneous, immutable child studies, each with its own question,
//! profile and digest, so graphics profiles, benchmark scenes and Stutter modes are never pooled.
//! Stages that do not fit the attempt budget are sealed `DeferredBudget`, never silently dropped
//! or reported as tested.

use serde::Serialize;

use super::{ResearchAction, ResearchFactor, ResearchIntentV1};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// 1920x1080 raster, frame generation off (provisional label; measurement decides).
    Cpu,
    /// 3440x1440 high raster, frame generation off (provisional label).
    Gpu,
    /// Long in-game work unit for the live Stutter action study.
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    /// Observe-only screen of every eligible tool, no Stutter mutation.
    Screen,
    /// Frozen confirmation of selected pairs after the screen.
    Confirm,
    /// Normal-controller action study (Supported actions only).
    Action(ResearchAction),
    /// One fork-local factor at a time.
    Feature(ResearchFactor),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChildStudy {
    pub profile: Profile,
    pub stage: Stage,
    /// Attempts this study needs at minimum; below this it cannot reach a verdict.
    pub min_attempts: u32,
    /// Digests of the studies that must yield valid evidence first; bound into `study_sha256` so
    /// a dependent study cannot be re-pointed at different predecessors.
    pub depends_on: Vec<String>,
    /// Digest binding the parent intent, profile, stage and requirement. Never reused.
    pub study_sha256: String,
}

/// AB/BA confirmation needs at least four independent pairs, so eight attempts.
const CONFIRM_ATTEMPTS: u32 = 8;
const FEATURE_ATTEMPTS: u32 = 8;
const ACTION_ATTEMPTS: u32 = 2;

fn study(
    intent_sha256: &str,
    profile: Profile,
    stage: Stage,
    min_attempts: u32,
    depends_on: Vec<String>,
) -> anyhow::Result<ChildStudy> {
    let study_sha256 = stutter_core::hash::sha256_hex(&serde_json::to_vec(&(
        intent_sha256,
        profile,
        stage,
        min_attempts,
        &depends_on,
    ))?);
    Ok(ChildStudy {
        profile,
        stage,
        min_attempts,
        depends_on,
        study_sha256,
    })
}

/// The ordered funnel for one intent: screens, confirmations, the supported-action study when the
/// intent authorizes an action, then one study per declared factor on the CPU profile.
pub fn plan(intent: &ResearchIntentV1) -> anyhow::Result<Vec<ChildStudy>> {
    let digest = intent.digest()?;
    let tools = u32::try_from(intent.tools.len())?;
    anyhow::ensure!(tools >= 2, "a comparison needs at least two eligible tools");
    let mut studies: Vec<ChildStudy> = Vec::new();
    for profile in [Profile::Cpu, Profile::Gpu] {
        studies.push(study(&digest, profile, Stage::Screen, tools, Vec::new())?);
    }
    for profile in [Profile::Cpu, Profile::Gpu] {
        let deps = dependency_digests(&studies, profile, Stage::Confirm);
        studies.push(study(
            &digest,
            profile,
            Stage::Confirm,
            CONFIRM_ATTEMPTS,
            deps,
        )?);
    }
    // One study per authorized action; which one actually runs is decided from the qualified
    // diagnosis at run time, never by enum order here.
    let mut actions = intent.actions.clone();
    actions.sort_unstable();
    actions.dedup();
    for action in actions {
        let deps = dependency_digests(&studies, Profile::Long, Stage::Action(action));
        studies.push(study(
            &digest,
            Profile::Long,
            Stage::Action(action),
            ACTION_ATTEMPTS,
            deps,
        )?);
    }
    let mut factors = intent.factors.clone();
    factors.sort_unstable();
    factors.dedup();
    for factor in factors {
        let deps = dependency_digests(&studies, Profile::Cpu, Stage::Feature(factor));
        studies.push(study(
            &digest,
            Profile::Cpu,
            Stage::Feature(factor),
            FEATURE_ATTEMPTS,
            deps,
        )?);
    }
    Ok(studies)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Funnel<'a> {
    pub runnable: Vec<&'a ChildStudy>,
    pub deferred_budget: Vec<&'a ChildStudy>,
}

/// The studies that must be admitted (and later yield valid evidence) before `study` may run.
/// Confirmation needs its profile's screen; the action and feature studies need the CPU screen
/// and confirmation. A budget alone never grants a stage whose prerequisites are not admitted.
fn prerequisites(study: &ChildStudy) -> Vec<(Profile, Stage)> {
    requires(study.profile, study.stage)
}

/// Digests of the already-built prerequisite studies, in a fixed order.
fn dependency_digests(studies: &[ChildStudy], profile: Profile, stage: Stage) -> Vec<String> {
    requires(profile, stage)
        .iter()
        .filter_map(|(p, s)| studies.iter().find(|c| c.profile == *p && c.stage == *s))
        .map(|c| c.study_sha256.clone())
        .collect()
}

fn requires(profile: Profile, stage: Stage) -> Vec<(Profile, Stage)> {
    match stage {
        Stage::Screen => Vec::new(),
        Stage::Confirm => vec![(profile, Stage::Screen)],
        Stage::Action(_) | Stage::Feature(_) => {
            vec![
                (Profile::Cpu, Stage::Screen),
                (Profile::Cpu, Stage::Confirm),
            ]
        }
    }
}

/// Dependency-aware admission in funnel order. A study is admitted only if every prerequisite
/// was admitted and it fits the remaining budget; otherwise it is deferred. A cheap later study
/// is therefore never run as an orphan, and order and frozen requirements are never altered.
/// Admission is necessary, not sufficient: the coordinator still requires each predecessor to
/// have produced valid evidence before starting the dependent stage.
pub fn fit_budget(studies: &[ChildStudy], max_attempts: u32) -> Funnel<'_> {
    let mut remaining = max_attempts;
    let mut funnel = Funnel {
        runnable: Vec::new(),
        deferred_budget: Vec::new(),
    };
    for study in studies {
        let prerequisites_admitted = prerequisites(study).iter().all(|(profile, stage)| {
            funnel
                .runnable
                .iter()
                .any(|s| s.profile == *profile && s.stage == *stage)
        });
        if prerequisites_admitted && study.min_attempts <= remaining {
            remaining -= study.min_attempts;
            funnel.runnable.push(study);
        } else {
            funnel.deferred_budget.push(study);
        }
    }
    funnel
}
/// The sealed result of a finished child study, as the coordinator records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudyResult {
    /// Valid, qualified evidence: dependents may build on it.
    Qualified,
    /// Ran but its evidence is not qualified (for example unproven provenance or a refused
    /// comparison). Dependents must not run on it.
    Unqualified,
    Failed,
    DeferredBudget,
}

/// Why a dependent study may not start yet.
#[derive(Debug, PartialEq, Eq)]
pub enum NotReady {
    /// A prerequisite has no recorded result yet (still pending or never planned).
    PredecessorPending,
    /// A prerequisite finished without qualified evidence; this study can never run.
    PredecessorNotQualified,
}

/// Dependency gate: a study may start only when every prerequisite it is bound to has a recorded
/// qualified result. Admission by budget alone never satisfies this.
pub fn ready_to_start(
    study: &ChildStudy,
    results: &std::collections::BTreeMap<String, StudyResult>,
) -> Result<(), NotReady> {
    let mut pending = false;
    for digest in &study.depends_on {
        match results.get(digest) {
            Some(StudyResult::Qualified) => {}
            Some(_) => return Err(NotReady::PredecessorNotQualified),
            None => pending = true,
        }
    }
    if pending {
        Err(NotReady::PredecessorPending)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::research::{ProtonSource, ResearchTool, tests::intent as base_intent};

    fn intent() -> ResearchIntentV1 {
        let mut intent = base_intent();
        intent.tools.push(ResearchTool {
            source: ProtonSource::ValveStable,
            identity_sha256: "b".repeat(64),
        });
        intent
    }

    #[test]
    fn plan_is_deterministic_and_never_pools_profiles() {
        let mut intent = intent();
        intent.actions = vec![ResearchAction::CpuAffinityProfile];
        intent.factors = vec![ResearchFactor::NtSync, ResearchFactor::WineWayland];
        let first = plan(&intent).unwrap();
        assert_eq!(first, plan(&intent).unwrap());
        let digests: std::collections::BTreeSet<_> =
            first.iter().map(|s| s.study_sha256.clone()).collect();
        assert_eq!(digests.len(), first.len(), "every child study is distinct");
        assert!(
            first
                .iter()
                .any(|s| s.profile == Profile::Cpu && s.stage == Stage::Screen)
        );
        assert!(
            first
                .iter()
                .any(|s| s.profile == Profile::Gpu && s.stage == Stage::Screen)
        );
        // The action study and features exist only because the intent authorized them.
        assert_eq!(
            first
                .iter()
                .filter(|s| matches!(s.stage, Stage::Action(_)))
                .count(),
            1
        );
        assert_eq!(
            first
                .iter()
                .filter(|s| matches!(s.stage, Stage::Feature(_)))
                .count(),
            2
        );
        intent.actions.clear();
        intent.factors.clear();
        let bare = plan(&intent).unwrap();
        assert!(
            bare.iter()
                .all(|s| matches!(s.stage, Stage::Screen | Stage::Confirm))
        );
    }

    #[test]
    fn a_changed_intent_changes_every_study_digest() {
        let a = plan(&intent()).unwrap();
        let mut other = intent();
        other.build_id += 1;
        let b = plan(&other).unwrap();
        assert!(
            a.iter()
                .zip(&b)
                .all(|(x, y)| x.study_sha256 != y.study_sha256)
        );
    }

    #[test]
    fn studies_that_do_not_fit_are_deferred_not_dropped() {
        let mut intent = intent();
        intent.factors = vec![ResearchFactor::NtSync];
        let studies = plan(&intent).unwrap();
        let tools = u32::try_from(intent.tools.len()).unwrap();
        // Enough for both screens and one confirmation only.
        let funnel = fit_budget(&studies, tools * 2 + CONFIRM_ATTEMPTS);
        assert_eq!(
            funnel.runnable.len() + funnel.deferred_budget.len(),
            studies.len()
        );
        assert!(
            funnel
                .deferred_budget
                .iter()
                .any(|s| matches!(s.stage, Stage::Feature(_)))
        );
        assert_eq!(fit_budget(&studies, 0).runnable.len(), 0);
    }

    #[test]
    fn dependent_studies_are_bound_to_their_predecessors_digests() {
        let mut intent = intent();
        intent.actions = vec![ResearchAction::CpuAffinityProfile];
        intent.factors = vec![ResearchFactor::NtSync];
        let studies = plan(&intent).unwrap();
        let find = |profile, stage| {
            studies
                .iter()
                .find(|s| s.profile == profile && s.stage == stage)
                .unwrap()
        };
        let cpu_screen = find(Profile::Cpu, Stage::Screen);
        let cpu_confirm = find(Profile::Cpu, Stage::Confirm);
        assert!(cpu_screen.depends_on.is_empty());
        assert_eq!(
            cpu_confirm.depends_on,
            std::slice::from_ref(&cpu_screen.study_sha256)
        );
        let action = studies
            .iter()
            .find(|s| matches!(s.stage, Stage::Action(_)))
            .unwrap();
        assert_eq!(
            action.depends_on,
            [
                cpu_screen.study_sha256.clone(),
                cpu_confirm.study_sha256.clone()
            ]
        );
        // The digest covers the dependency list, so re-pointing a study changes its identity.
        let rebound = study(
            "x",
            action.profile,
            action.stage,
            action.min_attempts,
            Vec::new(),
        )
        .unwrap();
        let original = study(
            "x",
            action.profile,
            action.stage,
            action.min_attempts,
            action.depends_on.clone(),
        )
        .unwrap();
        assert_ne!(rebound.study_sha256, original.study_sha256);
    }

    #[test]
    fn a_dependent_study_starts_only_on_qualified_recorded_predecessors() {
        let mut intent = intent();
        intent.actions = vec![ResearchAction::CpuAffinityProfile];
        let studies = plan(&intent).unwrap();
        let action = studies
            .iter()
            .find(|s| matches!(s.stage, Stage::Action(_)))
            .unwrap();
        let mut results = std::collections::BTreeMap::new();
        // Nothing recorded yet: pending, not runnable.
        assert_eq!(
            ready_to_start(action, &results),
            Err(NotReady::PredecessorPending)
        );
        // Only the screen is recorded: the confirmation is still pending.
        results.insert(action.depends_on[0].clone(), StudyResult::Qualified);
        assert_eq!(
            ready_to_start(action, &results),
            Err(NotReady::PredecessorPending)
        );
        // An unqualified confirmation blocks the action permanently.
        results.insert(action.depends_on[1].clone(), StudyResult::Unqualified);
        assert_eq!(
            ready_to_start(action, &results),
            Err(NotReady::PredecessorNotQualified)
        );
        for blocked in [StudyResult::Failed, StudyResult::DeferredBudget] {
            results.insert(action.depends_on[1].clone(), blocked);
            assert_eq!(
                ready_to_start(action, &results),
                Err(NotReady::PredecessorNotQualified)
            );
        }
        results.insert(action.depends_on[1].clone(), StudyResult::Qualified);
        assert_eq!(ready_to_start(action, &results), Ok(()));
        // A screen has no prerequisites and is always ready.
        let screen = studies.iter().find(|s| s.stage == Stage::Screen).unwrap();
        assert_eq!(ready_to_start(screen, &Default::default()), Ok(()));
    }

    #[test]
    fn a_small_budget_never_admits_an_orphan_action_or_confirmation() {
        let mut intent = intent();
        intent.actions = vec![ResearchAction::CpuAffinityProfile];
        intent.factors = vec![ResearchFactor::NtSync];
        let studies = plan(&intent).unwrap();
        let tools = u32::try_from(intent.tools.len()).unwrap();
        // Enough for both screens plus the cheap action study, but not the confirmations.
        let funnel = fit_budget(&studies, tools * 2 + ACTION_ATTEMPTS);
        assert!(funnel.runnable.iter().all(|s| s.stage == Stage::Screen));
        assert!(
            funnel
                .deferred_budget
                .iter()
                .any(|s| matches!(s.stage, Stage::Action(_)))
        );
        // A budget that covers only the GPU screen cannot admit CPU-dependent stages.
        let gpu_only = fit_budget(&studies, tools);
        assert!(gpu_only.runnable.iter().all(|s| s.stage == Stage::Screen));
        // With everything affordable, every dependent stage is admitted.
        let all: u32 = studies.iter().map(|s| s.min_attempts).sum();
        assert_eq!(fit_budget(&studies, all).runnable.len(), studies.len());
    }
}
