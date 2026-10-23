use std::{cell::RefCell, path::Path, time::Duration};

use serde_json::json;

use crate::validation_campaign::{
    repo_pr::{Checks, Landed, PrChange, PrError, WaitPolicy, classify_checks, land},
    shell::{Shell, ShellOutput},
};

/// Answers commands by prefix and records everything it was asked to run.
struct Script {
    answers: Vec<(&'static str, ShellOutput)>,
    calls: RefCell<Vec<String>>,
    rollups: RefCell<Vec<String>>,
}

fn ok(stdout: &str) -> ShellOutput {
    ShellOutput {
        code: 0,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}
fn bad(stderr: &str) -> ShellOutput {
    ShellOutput {
        code: 1,
        stdout: String::new(),
        stderr: stderr.to_owned(),
    }
}

impl Script {
    fn new(answers: Vec<(&'static str, ShellOutput)>, rollups: Vec<serde_json::Value>) -> Self {
        Self {
            answers,
            calls: RefCell::default(),
            rollups: RefCell::new(
                rollups
                    .iter()
                    .rev()
                    .map(|r| json!({"statusCheckRollup": r}).to_string())
                    .collect(),
            ),
        }
    }
    fn called(&self, needle: &str) -> bool {
        self.calls.borrow().iter().any(|c| c.contains(needle))
    }
}

impl Shell for Script {
    fn run(&self, program: &str, args: &[&str], _: &Path) -> ShellOutput {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        if line.contains("--json statusCheckRollup") {
            let next = self.rollups.borrow_mut().pop().unwrap_or_default();
            return ok(&next);
        }
        self.answers
            .iter()
            .find(|(prefix, _)| line.starts_with(prefix))
            .map_or_else(
                || {
                    ok(if line.starts_with("git status --porcelain") {
                        ""
                    } else {
                        "deadbeef\n"
                    })
                },
                |(_, out)| out.clone(),
            )
    }
    fn sleep(&self, _: Duration) {}
}

fn change() -> PrChange<'static> {
    PrChange {
        branch: "campaign/x/integration",
        title: "t",
        body: "b",
        commit_message: "m",
    }
}

fn policy() -> WaitPolicy {
    WaitPolicy {
        poll: Duration::ZERO,
        max_polls: 4,
    }
}

const GREEN: fn() -> serde_json::Value =
    || json!([{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]);

#[test]
fn checks_are_green_only_when_every_counted_check_succeeded() {
    assert_eq!(classify_checks(&GREEN()), Checks::Green);
    let pending = json!([{"name":"a","status":"IN_PROGRESS","conclusion":""}]);
    assert_eq!(classify_checks(&pending), Checks::Pending);
    for conclusion in ["FAILURE", "CANCELLED", "SKIPPED", "NEUTRAL"] {
        let red = json!([{"name":"a","status":"COMPLETED","conclusion":conclusion}]);
        assert!(
            matches!(classify_checks(&red), Checks::Red(_)),
            "{conclusion}"
        );
    }
    // The hardware job is ignored; an empty rollup is never green.
    let only_hardware =
        json!([{"name":"live smoke tests","status":"COMPLETED","conclusion":"SKIPPED"}]);
    assert_eq!(classify_checks(&only_hardware), Checks::Pending);
    assert_eq!(classify_checks(&json!([])), Checks::Pending);
}

#[test]
fn a_change_is_branched_committed_pushed_checked_and_merged_in_order() {
    let shell = Script::new(
        vec![
            ("git diff --cached --quiet", bad("")),
            ("gh pr view campaign", bad("no pull request")),
            ("gh pr create", ok("https://example.test/pull/9\n")),
        ],
        vec![
            json!([{"name":"build","status":"IN_PROGRESS","conclusion":""}]),
            GREEN(),
        ],
    );
    let landed = land(&shell, Path::new("."), &change(), &|| Ok(()), &policy()).unwrap();
    assert_eq!(
        landed,
        Landed::Merged {
            commit: "deadbeef".to_owned()
        }
    );
    let calls = shell.calls.borrow().clone();
    let position = |needle: &str| {
        calls
            .iter()
            .position(|c| c.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing in {calls:?}"))
    };
    assert!(position("checkout -B") < position("git add -A"));
    assert!(position("git commit") < position("git push"));
    assert!(position("git push") < position("gh pr create"));
    assert!(position("gh pr create") < position("gh pr merge https://example.test/pull/9"));
}

#[test]
fn red_checks_are_never_merged_over() {
    let shell = Script::new(
        vec![
            ("git diff --cached --quiet", bad("")),
            ("gh pr view campaign", bad("none")),
            ("gh pr create", ok("https://example.test/pull/9\n")),
        ],
        vec![json!([{"name":"build","status":"COMPLETED","conclusion":"FAILURE"}])],
    );
    let error = land(&shell, Path::new("."), &change(), &|| Ok(()), &policy()).unwrap_err();
    assert!(matches!(error, PrError::Failed { ref stage, retryable: false, .. } if stage == "ci"));
    assert!(!shell.called("gh pr merge"));
}

#[test]
fn an_unauthenticated_gh_or_dirty_checkout_blocks_before_changing_anything() {
    let shell = Script::new(vec![("gh auth status", bad("not logged in"))], vec![]);
    assert!(matches!(
        land(&shell, Path::new("."), &change(), &|| Ok(()), &policy()),
        Err(PrError::Blocked(_))
    ));
    let dirty = Script::new(
        vec![
            ("gh pr view campaign", bad("none")),
            ("git status --porcelain", ok(" M file\n")),
        ],
        vec![],
    );
    assert!(matches!(
        land(&dirty, Path::new("."), &change(), &|| Ok(()), &policy()),
        Err(PrError::Blocked(_))
    ));
    assert!(!dirty.called("checkout"));
}

#[test]
fn an_empty_change_and_an_already_merged_pull_request_are_idempotent() {
    let nothing = Script::new(
        vec![
            ("gh pr view campaign", bad("none")),
            ("git diff --cached --quiet", ok("")),
        ],
        vec![],
    );
    assert_eq!(
        land(&nothing, Path::new("."), &change(), &|| Ok(()), &policy()).unwrap(),
        Landed::NoChange
    );
    assert!(!nothing.called("git commit") && !nothing.called("gh pr create"));
    let merged = Script::new(
        vec![(
            "gh pr view campaign",
            ok(r#"{"number":9,"state":"MERGED","url":"u"}"#),
        )],
        vec![],
    );
    assert!(matches!(
        land(
            &merged,
            Path::new("."),
            &change(),
            &|| panic!("must not reapply"),
            &policy()
        ),
        Ok(Landed::Merged { .. })
    ));
    assert!(!merged.called("git commit"));
}

#[test]
fn an_open_pull_request_is_resumed_not_recreated() {
    let open = Script::new(
        vec![(
            "gh pr view campaign",
            ok(r#"{"number":9,"state":"OPEN","url":"https://example.test/pull/9"}"#),
        )],
        vec![GREEN()],
    );
    assert!(matches!(
        land(
            &open,
            Path::new("."),
            &change(),
            &|| panic!("must not reapply"),
            &policy()
        ),
        Ok(Landed::Merged { .. })
    ));
    assert!(!open.called("gh pr create") && open.called("gh pr merge https://example.test/pull/9"));
}
