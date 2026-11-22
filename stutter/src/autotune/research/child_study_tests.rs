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
    assert!(bare.iter().all(|s| matches!(
        s.stage,
        Stage::Screen | Stage::Pilot | Stage::Confirm | Stage::Holdout
    )));
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
    let cpu_pilot = find(Profile::Cpu, Stage::Pilot);
    let cpu_confirm = find(Profile::Cpu, Stage::Confirm);
    assert!(cpu_screen.depends_on.is_empty());
    // The funnel is screen -> pilot -> confirmation, each bound to its predecessor's digest.
    assert_eq!(
        cpu_pilot.depends_on,
        std::slice::from_ref(&cpu_screen.study_sha256)
    );
    assert_eq!(
        cpu_confirm.depends_on,
        std::slice::from_ref(&cpu_pilot.study_sha256)
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
    // Enough for both screens plus a few attempts, but not a pilot, a confirmation or the action
    // study (whose off/on pairs need a comparison's worth of attempts).
    let funnel = fit_budget(&studies, tools * 2 + PILOT_ATTEMPTS - 1);
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

/// Run the whole plan the way the coordinator would: ask for the next study, record a result
/// from `outcome`, spend its attempts, and repeat.
fn simulate(
    studies: &[ChildStudy],
    mut budget: u32,
    outcome: impl Fn(&ChildStudy) -> StudyResult,
) -> Vec<(Profile, Stage, &'static str)> {
    let mut results = std::collections::BTreeMap::new();
    let mut log = Vec::new();
    loop {
        match next_study(studies, &results, budget) {
            NextStudy::Done => return log,
            NextStudy::Run(study) => {
                budget -= study.min_attempts;
                let result = outcome(study);
                results.insert(study.study_sha256.clone(), result);
                log.push((study.profile, study.stage, "ran"));
            }
            NextStudy::DeferBudget(study) => {
                results.insert(study.study_sha256.clone(), StudyResult::DeferredBudget);
                log.push((study.profile, study.stage, "deferred"));
            }
            NextStudy::Blocked(study) => {
                results.insert(study.study_sha256.clone(), StudyResult::Blocked);
                log.push((study.profile, study.stage, "blocked"));
            }
        }
    }
}

fn full_plan() -> Vec<ChildStudy> {
    let mut intent = intent();
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    intent.factors = vec![ResearchFactor::NtSync];
    plan(&intent).unwrap()
}

#[test]
fn a_fully_qualified_campaign_runs_every_study_in_funnel_order() {
    let studies = full_plan();
    let all: u32 = studies.iter().map(|s| s.min_attempts).sum();
    let log = simulate(&studies, all, |_| StudyResult::Qualified);
    assert_eq!(log.len(), studies.len());
    assert!(log.iter().all(|(_, _, what)| *what == "ran"));
    // Order is the plan's order: screens, then confirmations, then action, then features.
    let order: Vec<_> = log.iter().map(|(p, s, _)| (*p, *s)).collect();
    let planned: Vec<_> = studies.iter().map(|s| (s.profile, s.stage)).collect();
    assert_eq!(order, planned);
}

#[test]
fn an_unqualified_cpu_confirmation_blocks_the_action_and_feature_but_not_the_gpu_study() {
    let studies = full_plan();
    let all: u32 = studies.iter().map(|s| s.min_attempts).sum();
    let log = simulate(&studies, all, |s| {
        if s.profile == Profile::Cpu && s.stage == Stage::Confirm {
            StudyResult::Unqualified
        } else {
            StudyResult::Qualified
        }
    });
    let status = |profile, stage_match: fn(&Stage) -> bool| {
        log.iter()
            .find(|(p, s, _)| *p == profile && stage_match(s))
            .map(|(_, _, what)| *what)
    };
    assert_eq!(status(Profile::Gpu, |s| *s == Stage::Confirm), Some("ran"));
    // The held-out repetition of an unqualified confirmation never runs; the other profile's does.
    assert_eq!(
        status(Profile::Cpu, |s| *s == Stage::Holdout),
        Some("blocked")
    );
    assert_eq!(status(Profile::Gpu, |s| *s == Stage::Holdout), Some("ran"));
    assert_eq!(
        status(Profile::Long, |s| matches!(s, Stage::Action(_))),
        Some("blocked")
    );
    assert_eq!(
        status(Profile::Cpu, |s| matches!(s, Stage::Feature(_))),
        Some("blocked")
    );
}

#[test]
fn a_small_budget_defers_what_it_cannot_afford_and_never_runs_an_orphan() {
    let studies = full_plan();
    let tools = u32::try_from(intent().tools.len()).unwrap();
    // Enough for both screens only.
    let log = simulate(&studies, tools * 2, |_| StudyResult::Qualified);
    for (_, stage, what) in &log {
        match stage {
            Stage::Init | Stage::Screen => assert_eq!(*what, "ran"),
            // An unaffordable pilot is deferred, and everything that depends on it is blocked
            // rather than run on the leftover budget.
            Stage::Pilot => assert_eq!(*what, "deferred"),
            Stage::Confirm
            | Stage::Holdout
            | Stage::MatchedCachyOs
            | Stage::Action(_)
            | Stage::Feature(_) => {
                assert_eq!(*what, "blocked")
            }
        }
    }
    // Every study ends with a recorded outcome, so the campaign always terminates.
    assert_eq!(log.len(), studies.len());
}

#[test]
fn a_pilots_reservation_covers_three_runs_of_every_tool_the_campaign_authorizes() {
    use crate::autotune::research::{ProtonSource, ResearchTool};
    let mut intent = base_intent();
    for source in [
        ProtonSource::ValveStable,
        ProtonSource::ValveExperimental,
        ProtonSource::CachyOs,
        ProtonSource::CachyOsV3,
        ProtonSource::Em,
    ]
    .into_iter()
    .enumerate()
    {
        let (index, source) = source;
        intent.tools.push(ResearchTool {
            source,
            identity_sha256: format!("{:064x}", index + 20),
        });
    }
    assert!(intent.tools.len() >= 6);
    let tools = u32::try_from(intent.tools.len()).unwrap();
    let studies = plan(&intent).unwrap();
    let pilots: Vec<_> = studies.iter().filter(|s| s.stage == Stage::Pilot).collect();
    assert_eq!(pilots.len(), 2);
    assert!(
        pilots.iter().all(|p| p.min_attempts == 3 * tools),
        "{:?}",
        pilots.iter().map(|p| p.min_attempts).collect::<Vec<_>>()
    );
    // At the boundary: a budget that holds the screens but not the six-arm pilot defers it and what depends on it.
    let screens: u32 = studies
        .iter()
        .filter(|s| s.stage == Stage::Screen)
        .map(|s| s.min_attempts)
        .sum();
    let funnel = fit_budget(&studies, screens + 3 * tools - 1);
    assert!(
        funnel
            .deferred_budget
            .iter()
            .any(|s| s.stage == Stage::Pilot)
    );
    assert!(funnel.runnable.iter().all(|s| s.stage != Stage::Confirm));
}

#[test]
fn eight_long_attempts_per_action_are_counted_in_time_and_what_cannot_fit_is_named() {
    let mut intent = intent();
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    intent.long_attempt_seconds = Some(1200);
    let studies = plan(&intent).unwrap();
    intent.max_active_seconds = 8 * 3600;
    let action: Vec<_> = studies
        .iter()
        .filter(|s| matches!(s.stage, Stage::Action(_)))
        .collect();
    assert_eq!(action.len(), 1);
    // Eight twenty-minute attempts cost at least 160 minutes by themselves.
    assert!(estimated_active_seconds(std::slice::from_ref(action[0]), &intent) >= 160 * 60);
    // A budget of the whole estimate leaves nothing deferred; one second less defers the last study and no earlier one.
    intent.max_active_seconds = u32::try_from(estimated_active_seconds(&studies, &intent)).unwrap();
    assert!(deferred_for_time(&studies, &intent).is_empty());
    intent.max_active_seconds -= 1;
    let deferred = deferred_for_time(&studies, &intent);
    assert_eq!(deferred.len(), 1);
    assert_eq!(
        deferred[0].study_sha256,
        studies.last().unwrap().study_sha256
    );
}
