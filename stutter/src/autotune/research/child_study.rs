//! Frozen child-study plan (PLAN section 1 nesting and section 6.3 funnel). One authorized
//! campaign is split into homogeneous, immutable child studies, each with its own question,
//! profile and digest, so graphics profiles, benchmark scenes and Stutter modes are never pooled.
//! Stages that do not fit the attempt budget are sealed `DeferredBudget`, never silently dropped
//! or reported as tested.

use serde::{Deserialize, Serialize};

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
    /// Three runs per viable arm, to measure run-to-run variability. It freezes the
    /// confirmation rule (effect size and equivalence margin) before any confirmation run starts.
    Pilot,
    /// Frozen confirmation of the selected pair under the pilot's rule.
    Confirm,
    /// Held-out validation: the confirmed pair re-measured on fresh, separately seeded runs under the
    /// same frozen rule, so a winner is claimed only when an independent repetition agrees.
    Holdout,
    /// Preregistered matched comparison of CachyOS generic vs v3, its own frozen study.
    MatchedCachyOs,
    /// Normal-controller action study (Supported actions only).
    Action(ResearchAction),
    /// One fork-local factor at a time.
    Feature(ResearchFactor),
    /// One supervised, non-measuring launch per cold arm so its prefix and settings exist before any
    /// screen applies a profile (PLAN section 6.1). Present only when the intent froze cold arms.
    Init,
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

/// Which Proton arms a study runs across.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudyArms {
    /// Every eligible tool of the campaign (the screen).
    All,
    /// Every viable screen arm, subject to the campaign's frozen authorization.
    Viable,
    /// Exactly two of them, selected after the screen: the control and one candidate. Confirmation,
    /// feature and action studies compare two conditions, so their attempt counts divide across two
    /// arms whatever the campaign's tool count is.
    Pair,
    /// Exactly the arms the intent froze as cold: one initialization attempt each.
    Listed,
    /// One of them, fixed after the screen (the control), measured with a factor or an action off and
    /// on: a within-source treatment, not a comparison of Proton builds.
    Single,
}

impl ChildStudy {
    /// The treatment a within-source study switches off and on (none for any other study).
    pub fn treatment_spec(&self) -> Option<super::TreatmentSpec> {
        match self.stage {
            Stage::Feature(factor) => Some(super::TreatmentSpec::Factor(factor)),
            Stage::Action(action) => Some(super::TreatmentSpec::Action(action)),
            Stage::Init
            | Stage::Screen
            | Stage::Pilot
            | Stage::Confirm
            | Stage::Holdout
            | Stage::MatchedCachyOs => None,
        }
    }

    pub fn arms(&self) -> StudyArms {
        match self.stage {
            Stage::Init => StudyArms::Listed,
            Stage::Screen => StudyArms::All,
            Stage::Pilot => StudyArms::Viable,
            Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs => StudyArms::Pair,
            Stage::Action(_) | Stage::Feature(_) => StudyArms::Single,
        }
    }
}

/// AB/BA confirmation needs at least four independent pairs, so eight attempts.
/// Three runs for each of the two arms.
const PILOT_ATTEMPTS: u32 = 6;
const PILOT_RUNS_PER_ARM: u32 = 3;
const CONFIRM_ATTEMPTS: u32 = 8;
const HOLDOUT_ATTEMPTS: u32 = 8;
const FEATURE_ATTEMPTS: u32 = 8;
const ACTION_ATTEMPTS: u32 = 8;

#[path = "child_study_init.rs"]
mod init;

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

/// The ordered funnel for one intent: screens, pilots, confirmations, the supported-action study when the
/// intent authorizes an action, then one study per declared factor on the CPU profile.
pub fn plan(intent: &ResearchIntentV1) -> anyhow::Result<Vec<ChildStudy>> {
    let digest = intent.digest()?;
    let tools = u32::try_from(intent.tools.len())?;
    anyhow::ensure!(tools >= 2, "a comparison needs at least two eligible tools");
    let mut studies: Vec<ChildStudy> = Vec::new();
    let screen_deps = init::study_before_screens(intent, &digest, &mut studies)?;
    for profile in [Profile::Cpu, Profile::Gpu] {
        studies.push(study(
            &digest,
            profile,
            Stage::Screen,
            tools,
            screen_deps.clone(),
        )?);
    }
    for profile in [Profile::Cpu, Profile::Gpu] {
        let deps = dependency_digests(&studies, profile, Stage::Pilot);
        // Three runs per arm: reserve the worst case of every tool being viable, so admission never assumes
        // attempts the pilot will actually consume (the pilot itself runs over the arms the screen kept).
        let reserved = tools.saturating_mul(PILOT_RUNS_PER_ARM);
        studies.push(study(&digest, profile, Stage::Pilot, reserved, deps)?);
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
    // The held-out repetition of each profile's confirmation, before anything is built on it.
    for profile in [Profile::Cpu, Profile::Gpu] {
        let deps = dependency_digests(&studies, profile, Stage::Holdout);
        studies.push(study(
            &digest,
            profile,
            Stage::Holdout,
            HOLDOUT_ATTEMPTS,
            deps,
        )?);
    }
    // The explicit CA↔C3 matched comparison study (PLAN §3.1).
    if intent
        .tools
        .iter()
        .any(|tool| tool.source == super::ProtonSource::CachyOs)
        && intent
            .tools
            .iter()
            .any(|tool| tool.source == super::ProtonSource::CachyOsV3)
    {
        let deps = dependency_digests(&studies, Profile::Cpu, Stage::MatchedCachyOs);
        studies.push(study(
            &digest,
            Profile::Cpu,
            Stage::MatchedCachyOs,
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
        Stage::Init | Stage::Screen => Vec::new(),
        Stage::Pilot => vec![(profile, Stage::Screen)],
        Stage::Confirm => vec![(profile, Stage::Pilot)],
        // The holdout runs under its pilot's rule and only after the confirmation it repeats qualified.
        Stage::Holdout => vec![(profile, Stage::Pilot), (profile, Stage::Confirm)],
        Stage::MatchedCachyOs => vec![(Profile::Cpu, Stage::Pilot)],
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyResult {
    /// Valid, qualified evidence: dependents may build on it.
    Qualified,
    /// Ran but its evidence is not qualified (for example unproven provenance or a refused
    /// comparison). Dependents must not run on it.
    Unqualified,
    Failed,
    DeferredBudget,
    /// Never ran because a prerequisite lacked qualified evidence.
    Blocked,
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

/// What the coordinator should do next with the ordered study plan.
#[derive(Debug, PartialEq, Eq)]
pub enum NextStudy<'a> {
    /// Start this study now; it is ready and fits the remaining attempt budget.
    Run(&'a ChildStudy),
    /// This study can never run on the remaining budget: record it `DeferredBudget`.
    DeferBudget(&'a ChildStudy),
    /// A prerequisite finished without qualified evidence: record this study `Blocked`.
    Blocked(&'a ChildStudy),
    /// Every study has a recorded result.
    Done,
}

/// Pick the first study in funnel order that has no recorded result. Order is never changed and
/// a cheaper later study never jumps a blocked or deferred earlier one: each decision is
/// recorded and the function is called again. Predecessors always precede dependents in the
/// plan, so a still-pending predecessor cannot be skipped over.
pub fn next_study<'a>(
    studies: &'a [ChildStudy],
    results: &std::collections::BTreeMap<String, StudyResult>,
    remaining_attempts: u32,
) -> NextStudy<'a> {
    let Some(study) = studies
        .iter()
        .find(|study| !results.contains_key(&study.study_sha256))
    else {
        return NextStudy::Done;
    };
    match ready_to_start(study, results) {
        Err(NotReady::PredecessorNotQualified) => NextStudy::Blocked(study),
        // A pending predecessor precedes this study in the plan, so it would have been chosen
        // first; reaching here means the plan lists a dependent before its prerequisite.
        Err(NotReady::PredecessorPending) => NextStudy::Blocked(study),
        Ok(()) if study.min_attempts <= remaining_attempts => NextStudy::Run(study),
        Ok(()) => NextStudy::DeferBudget(study),
    }
}

#[cfg(test)]
#[path = "child_study_tests.rs"]
mod tests;

/// What the whole funnel costs in active time if every study runs its reserved attempts at the attempt limit its
/// profile uses (a long action attempt is at least twenty minutes), in funnel order.
pub fn estimated_active_seconds(studies: &[ChildStudy], intent: &ResearchIntentV1) -> u64 {
    studies
        .iter()
        .map(|study| {
            let per_attempt = match study.profile {
                Profile::Long => intent
                    .long_attempt_seconds
                    .unwrap_or(intent.max_attempt_seconds),
                _ => intent.max_attempt_seconds,
            };
            u64::from(study.min_attempts).saturating_mul(u64::from(per_attempt))
        })
        .fold(0u64, u64::saturating_add)
}

/// The studies that cannot all fit the campaign's active time, so the plan says so up front instead of reserving
/// attempts it cannot afford in time: the first study in funnel order whose cumulative estimate passes
/// `max_active_seconds`, and every one after it. The runtime still enforces the real clock; this is the offline
/// feasibility statement for an authorization.
pub fn deferred_for_time<'a>(
    studies: &'a [ChildStudy],
    intent: &ResearchIntentV1,
) -> Vec<&'a ChildStudy> {
    let mut spent = 0u64;
    let mut over = false;
    let mut deferred = Vec::new();
    for study in studies {
        let one = estimated_active_seconds(std::slice::from_ref(study), intent);
        if over || spent.saturating_add(one) > u64::from(intent.max_active_seconds) {
            over = true;
            deferred.push(study);
        } else {
            spent += one;
        }
    }
    deferred
}
