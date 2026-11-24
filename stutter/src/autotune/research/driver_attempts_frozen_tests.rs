//! One selection per study: profile application, the attempt count, the child intent and the launches all use
//! the inputs frozen the first time they were asked for, and a selection that changes afterwards fails the study.

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use super::*;

fn runner(
    calls: Arc<AtomicU32>,
    flip_after: u32,
) -> (tempfile::TempDir, DriverStudyAttempts, ChildStudy) {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let parent = parent();
    let study = plan(&parent).unwrap().remove(0);
    let mut now = parent.issued_unix_seconds + 1;
    let runner = DriverStudyAttempts::new(
        File::open(root.path()).unwrap(),
        parent,
        Box::new(Host::finishing()),
        Box::new(move || {
            now += 1;
            now
        }),
        Box::new(|| Ok(())),
        Box::new(|_| std::path::PathBuf::from("/nonexistent")),
        Box::new(move |study| {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            let mut inputs = inputs_for(study);
            // After `flip_after` calls the upstream result "changes": another arm order.
            if call >= flip_after {
                inputs.arms.reverse();
            }
            Ok(inputs)
        }),
    );
    (root, runner, study)
}

#[test]
fn the_arms_the_profile_was_applied_to_are_the_arms_the_child_runs() {
    let calls = Arc::new(AtomicU32::new(0));
    let (_root, mut runner, study) = runner(calls.clone(), u32::MAX);
    let applied_to = runner.selected_arms(&study).unwrap().unwrap();
    // The count and the later reads reuse the frozen selection: the selector is asked once, plus the single
    // confirmation read when the child is created.
    runner.required_attempts(&study).unwrap();
    assert_eq!(runner.selected_arms(&study).unwrap().unwrap(), applied_to);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    runner.begin(&study, 100).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn a_selection_that_changed_between_profile_application_and_launch_fails_the_study() {
    // The first read (profile application) sees one order; by the time the child is created the selector says
    // another. The child is never created over arms the profile was not applied to.
    let calls = Arc::new(AtomicU32::new(0));
    let (root, mut runner, study) = runner(calls, 1);
    runner.selected_arms(&study).unwrap();
    let error = runner.begin(&study, 100).unwrap_err();
    assert!(
        format!("{error:#}").contains("SelectionChangedAfterItWasFrozen"),
        "{error:#}"
    );
    assert!(
        std::fs::read_dir(root.path().join("studies")).map_or(true, |mut d| d.next().is_none()),
        "no study directory was created"
    );
}

#[test]
fn the_admission_facts_are_the_active_seconds_left_and_the_least_one_attempt_can_take() {
    let calls = Arc::new(AtomicU32::new(0));
    let (_root, runner, _study) = runner(calls, u32::MAX);
    let facts = runner.time_facts().unwrap();
    let (remaining, short_min, long_min) = (facts.remaining, facts.short_min, facts.long_min);
    // Nothing spent yet: the whole authorization is left; a short attempt has a floor, and with no work unit a
    // long attempt is bounded below by the intent's own attempt limit.
    assert_eq!(remaining, u64::from(runner.parent.max_active_seconds));
    assert_eq!(
        short_min,
        crate::autotune::research::driver_attempts::persistence::MIN_SHORT_ATTEMPT_SECONDS
    );
    assert_eq!(long_min, u64::from(runner.parent.max_attempt_seconds));
}

#[test]
fn unreadable_spend_is_not_zero_and_a_study_is_refused_only_when_it_is_certain_not_to_fit() {
    use crate::autotune::research::{
        child_study::{Profile, Stage, TimeFeasibility, plan, time_feasibility},
        driver_attempts::TimeFacts,
    };
    let calls = Arc::new(AtomicU32::new(0));
    let (root, runner, _study) = runner(calls, u32::MAX);
    // A child whose spend record cannot be read is charged the whole authorization, never nothing.
    let studies = root.path().join("studies");
    std::fs::create_dir_all(studies.join("0123456789abcdef")).unwrap();
    std::fs::write(studies.join("0123456789abcdef").join("budget.json"), b"{").unwrap();
    assert_eq!(runner.time_facts().unwrap().remaining, 0);
    // And a campaign whose spend cannot be read at all is an error the funnel halts on, not zero spent.
    std::fs::remove_dir_all(&studies).unwrap();
    std::fs::write(&studies, b"not a directory").unwrap();
    assert_eq!(runner.time_facts(), Err("BudgetEvidenceUnreadable"));
    // The three answers over one plan.
    let facts = |remaining| TimeFacts {
        remaining,
        short_min: 60,
        long_min: 1_200,
        short_max: 600,
        long_max: 1_800,
    };
    let parent = super::parent();
    let studies = plan(&parent).unwrap();
    let pilot = studies.iter().find(|s| s.stage == Stage::Pilot).unwrap();
    assert!(pilot.least_attempts() < pilot.min_attempts);
    // A pilot that ends with one surviving arm needs three attempts: it is not refused for want of six.
    let three = u64::from(pilot.least_attempts()) * 60;
    assert_ne!(
        time_feasibility(pilot, facts(three)),
        TimeFeasibility::Impossible
    );
    assert_eq!(
        time_feasibility(pilot, facts(three - 1)),
        TimeFeasibility::Impossible
    );
    assert_eq!(
        time_feasibility(pilot, facts(u64::MAX / 2)),
        TimeFeasibility::Guaranteed
    );
    assert_eq!(
        time_feasibility(pilot, facts(u64::from(pilot.min_attempts) * 600 - 1)),
        TimeFeasibility::Uncertain
    );
    // A long study can never take less than its twenty-minute unit per attempt.
    if let Some(long) = studies.iter().find(|s| s.profile == Profile::Long) {
        let needed = u64::from(long.min_attempts) * 1_200;
        assert_eq!(
            time_feasibility(long, facts(needed - 1)),
            TimeFeasibility::Impossible
        );
        assert_ne!(
            time_feasibility(long, facts(needed)),
            TimeFeasibility::Impossible
        );
    }
}

/// The wall-clock deadline bounds the admission as well as the active budget: with the clock sampled, a study is
/// planned against the time left to the authorization's end when that is less than the active seconds left.
#[test]
fn the_time_left_to_the_deadline_bounds_the_admission_when_it_is_less_than_the_active_budget() {
    let calls = Arc::new(AtomicU32::new(0));
    let (_root, mut runner, _study) = runner(calls, u32::MAX);
    let deadline = runner.parent.deadline_unix_seconds;
    runner.set_now(deadline - 100);
    assert_eq!(runner.time_facts().unwrap().remaining, 100);
    // Past the deadline nothing is left; far from it the active budget is the bound.
    runner.set_now(deadline + 5);
    assert_eq!(runner.time_facts().unwrap().remaining, 0);
    runner.set_now(0);
    assert_eq!(
        runner.time_facts().unwrap().remaining,
        u64::from(runner.parent.max_active_seconds)
    );
}

/// An optional study (an action or a feature) is admitted only while its worst case fits beside the least the
/// undecided mandatory studies and the terminal verification need; mandatory work is never refused by the reserve.
#[test]
fn an_optional_study_is_deferred_when_its_worst_case_would_consume_the_mandatory_reserve() {
    use crate::autotune::research::{
        ResearchAction,
        child_study::{TERMINAL_RESERVE_SECONDS, would_consume_the_mandatory_reserve},
        driver_attempts::TimeFacts,
    };
    let mut parent = super::parent();
    parent.actions = vec![ResearchAction::CpuAffinityProfile];
    let studies = plan(&parent).unwrap();
    let action = studies
        .iter()
        .find(|s| matches!(s.stage, Stage::Action(_)))
        .expect("an action study is planned");
    let mandatory = studies.iter().find(|s| s.is_mandatory()).unwrap();
    assert!(!action.is_mandatory());
    let facts = |remaining| TimeFacts {
        remaining,
        short_min: 60,
        long_min: 1_200,
        short_max: 600,
        long_max: 1_800,
    };
    let worst = action.worst_case_seconds(facts(0));
    let floor = 5_000;
    let needed = worst + floor + TERMINAL_RESERVE_SECONDS;
    // Exactly enough for the worst case beside the reserve: admitted; one second short: deferred.
    assert!(!would_consume_the_mandatory_reserve(
        action,
        facts(needed),
        floor
    ));
    assert!(would_consume_the_mandatory_reserve(
        action,
        facts(needed - 1),
        floor
    ));
    // With no mandatory work left the reserve is only the terminal verification.
    assert!(!would_consume_the_mandatory_reserve(
        action,
        facts(worst + TERMINAL_RESERVE_SECONDS),
        0
    ));
    // Mandatory work is never refused by the reserve, however little time is left.
    assert!(!would_consume_the_mandatory_reserve(
        mandatory,
        facts(1),
        floor
    ));
}
