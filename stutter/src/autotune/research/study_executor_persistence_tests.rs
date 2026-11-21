//! A settled child whose durable accounting failed is retried through the resident executor, never
//! re-driven, never restored under a failed settlement, and bounded.

use super::*;

/// Wraps the plain fake: the first `failures` polls report a persistence error on a settled child.
struct Flaky {
    inner: Attempts,
    failures: u32,
    pending_flag: bool,
    failed_once: bool,
}

impl StudyAttempts for Flaky {
    fn has_progress(&mut self, study: &ChildStudy) -> bool {
        self.inner.has_progress(study)
    }
    fn poll(&mut self, study: &ChildStudy, budget: u32) -> anyhow::Result<Option<AttemptsOutcome>> {
        if self.failures > 0 {
            self.failures -= 1;
            self.failed_once = true;
            self.inner.log.borrow_mut().push("persist-failed");
            anyhow::bail!("disk full");
        }
        self.inner.poll(study, budget)
    }
    fn persistence_pending(&self) -> bool {
        self.pending_flag && self.failed_once && !self.inner.log.borrow().contains(&"run")
    }
    fn abort(&mut self) -> Result<(), &'static str> {
        self.inner.abort()
    }
    fn settle_interrupted(&mut self, study: &ChildStudy) -> Result<(), &'static str> {
        self.inner.settle_interrupted(study)
    }
}

fn flaky(
    study: &ChildStudy,
    failures: u32,
    pending_flag: bool,
) -> (HostStudyExecutor<Profiles, Flaky>, Log) {
    let (plain, log) = executor(Ok(true), Ok(()), Ok(good(study)));
    let HostStudyExecutor {
        profiles, attempts, ..
    } = plain;
    (
        HostStudyExecutor::new(
            profiles,
            Flaky {
                inner: attempts,
                failures,
                pending_flag,
                failed_once: false,
            },
        ),
        log,
    )
}

#[test]
fn a_settled_child_retries_only_its_persistence_and_is_then_judged_normally() {
    let study = cpu_screen();
    let (mut exec, log) = flaky(&study, 2, true);
    let mut done = None;
    for _ in 0..20 {
        if let Some(run) = exec.poll(&study, 100).unwrap() {
            done = Some(run);
            break;
        }
    }
    let done = done.expect("the study finishes after the retries");
    assert_eq!(done.result, StudyResult::Qualified);
    // No settlement was attempted again and the profile was restored exactly once, at the end.
    assert_eq!(
        *log.borrow(),
        [
            "recover",
            "apply",
            "persist-failed",
            "persist-failed",
            "run",
            "restore"
        ]
    );
}

#[test]
fn a_failure_that_is_not_a_pending_persistence_still_settles_before_anything_is_restored() {
    let study = cpu_screen();
    let (mut exec, log) = flaky(&study, 1, false);
    let mut error = None;
    for _ in 0..5 {
        match exec.poll(&study, 100) {
            Err(e) => {
                error = Some(e);
                break;
            }
            Ok(Some(_)) => break,
            Ok(None) => {}
        }
    }
    assert!(error.is_some());
    let log = log.borrow();
    assert!(log.contains(&"abort"));
    assert!(!log.contains(&"run"));
}
