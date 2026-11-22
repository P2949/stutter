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
