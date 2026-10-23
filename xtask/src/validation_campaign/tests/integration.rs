use std::{cell::RefCell, fs, path::Path, time::Duration};

use serde_json::{Value, json};

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::Graph,
    integration::{
        FIXTURES, INDEX, IntegrationRunner, LEDGER, copy_fixtures, ledger_entry, merge_index,
    },
    pr_task::PrTools,
    repo_pr::WaitPolicy,
    repository::Binaries,
    shell::{Shell, ShellOutput},
};

fn entry(suite: &str, id: &str) -> Value {
    json!({"suite": suite, "capture_id": id, "status": "PASS", "source_commit": "c"})
}

#[test]
fn merging_is_deduplicated_ordered_and_idempotent() {
    let existing =
        json!({"schema_version": 1, "note": "n", "entries": [entry("live_affinity", "a-1")]});
    let new = vec![
        entry("live_observation", "o-2"),
        entry("live_affinity", "a-1"),
        entry("live_game_runnable", "g-1"),
        entry("live_observation", "o-2"),
    ];
    let (merged, added) = merge_index(&existing, &new);
    let ids: Vec<&str> = merged["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["capture_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["a-1", "g-1", "o-2"]);
    assert_eq!(added.len(), 2);
    assert_eq!(merged["note"], "n");
    let (again, none) = merge_index(&merged, &new);
    assert!(none.is_empty() && again == merged);
}

#[test]
fn fixtures_are_copied_once_and_never_overwritten_with_different_content() {
    let scratch = Scratch::new("integration-fixtures");
    let from = scratch.path().join("from");
    let to = scratch.path().join("to");
    fs::create_dir_all(from.join("cap_a")).unwrap();
    fs::create_dir_all(from.join(".staging-b")).unwrap();
    fs::write(from.join("cap_a/fixture.toml"), "name = \"a\"\n").unwrap();
    fs::write(from.join(".staging-b/fixture.toml"), "x").unwrap();
    assert_eq!(copy_fixtures(&from, &to), Ok(1));
    assert_eq!(copy_fixtures(&from, &to), Ok(1));
    assert!(!to.join(".staging-b").exists());
    fs::write(from.join("cap_a/fixture.toml"), "name = \"changed\"\n").unwrap();
    assert!(
        copy_fixtures(&from, &to)
            .unwrap_err()
            .contains("different content")
    );
    assert_eq!(copy_fixtures(&scratch.path().join("absent"), &to), Ok(0));
}

#[test]
fn the_ledger_entry_is_deterministic_and_names_kernels_and_counts() {
    let added = vec![
        json!({"suite": "live_kernel_compat", "capture_id": "k1", "kernel_release": "6.18.1"}),
        json!({"suite": "live_kernel_compat", "capture_id": "k2", "kernel_release": "6.12.9"}),
        entry("live_observation", "o"),
    ];
    let a = ledger_entry("core-1", "abc", &added, 66, &json!({"scheduler": 1}));
    assert_eq!(
        a,
        ledger_entry("core-1", "abc", &added, 66, &json!({"scheduler": 1}))
    );
    assert!(
        a.contains("live_kernel_compat x2")
            && a.contains("6.12.9, 6.18.1")
            && a.contains("fixtures added: 66")
    );
}

struct Merging {
    calls: RefCell<Vec<String>>,
}

impl Shell for Merging {
    fn run(&self, program: &str, args: &[&str], _: &Path) -> ShellOutput {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        let ok = |s: &str| ShellOutput {
            code: 0,
            stdout: s.to_owned(),
            stderr: String::new(),
        };
        if line.starts_with("gh pr view campaign") {
            return ShellOutput {
                code: 1,
                ..ShellOutput::default()
            };
        }
        if line.starts_with("git diff --cached --quiet") {
            return ShellOutput {
                code: 1,
                ..ShellOutput::default()
            };
        }
        if line.contains("statusCheckRollup") {
            return ok(
                r#"{"statusCheckRollup":[{"name":"build","status":"COMPLETED","conclusion":"SUCCESS"}]}"#,
            );
        }
        if line.starts_with("gh pr create") {
            return ok("https://example.test/pull/1\n");
        }
        ok(if line.starts_with("git status --porcelain") {
            ""
        } else {
            "cafe\n"
        })
    }
    fn sleep(&self, _: Duration) {}
}

#[test]
fn integration_writes_index_fixtures_and_ledger_then_lands_one_pull_request() {
    let scratch = Scratch::new("integration-run");
    let (dir, state, _) = fresh(&scratch);
    let repo = scratch.path();
    fs::create_dir_all(repo.join("docs/evidence")).unwrap();
    fs::write(repo.join(INDEX), r#"{"schema_version":1,"entries":[]}"#).unwrap();
    fs::write(repo.join(LEDGER), "# Status\n").unwrap();
    fs::create_dir_all(dir.0.join("corpus/fixtures/cap_a")).unwrap();
    fs::write(
        dir.0.join("corpus/fixtures/cap_a/fixture.toml"),
        "name = \"a\"\n",
    )
    .unwrap();
    let manifest = manifest();
    let binaries = Binaries {
        stutter: "s".into(),
        xtask: "x".into(),
    };
    let updates = RefCell::new(Vec::new());
    let context = RunContext {
        dir: &dir,
        state: &state,
        manifest: &manifest,
        repository: repo,
        binaries: &binaries,
        updates: &updates,
    };
    let shell = Merging {
        calls: RefCell::default(),
    };
    let runner = IntegrationRunner {
        tools: PrTools {
            shell: Box::new(shell),
            wait: WaitPolicy {
                poll: Duration::ZERO,
                max_polls: 2,
            },
            gates: Vec::new(),
        },
        entries: Box::new(|_| Ok(vec![entry("live_observation", "o-1")])),
    };
    let spec = Graph::core()
        .unwrap()
        .get("evidence-integration")
        .unwrap()
        .clone();
    assert!(matches!(
        runner.run(&context, &spec, 1),
        RunnerOutcome::Passed { .. }
    ));
    let index: Value = serde_json::from_slice(&fs::read(repo.join(INDEX)).unwrap()).unwrap();
    assert_eq!(index["entries"][0]["capture_id"], "o-1");
    assert!(repo.join(FIXTURES).join("cap_a/fixture.toml").is_file());
    assert!(
        fs::read_to_string(repo.join(LEDGER))
            .unwrap()
            .contains("## Campaign test-campaign: evidence integration")
    );
}

#[test]
fn a_bundle_that_no_longer_verifies_fails_integration_as_a_software_defect() {
    let scratch = Scratch::new("integration-bad");
    let (dir, state, _) = fresh(&scratch);
    let manifest = manifest();
    let binaries = Binaries {
        stutter: "s".into(),
        xtask: "x".into(),
    };
    let updates = RefCell::new(Vec::new());
    let context = RunContext {
        dir: &dir,
        state: &state,
        manifest: &manifest,
        repository: scratch.path(),
        binaries: &binaries,
        updates: &updates,
    };
    let runner = IntegrationRunner {
        tools: PrTools {
            shell: Box::new(Merging {
                calls: RefCell::default(),
            }),
            wait: WaitPolicy::default(),
            gates: Vec::new(),
        },
        entries: Box::new(|_| {
            Err("published bundle live-observation/x no longer verifies".to_owned())
        }),
    };
    let spec = Graph::core()
        .unwrap()
        .get("evidence-integration")
        .unwrap()
        .clone();
    assert!(
        matches!(runner.run(&context, &spec, 1), RunnerOutcome::Failed { stage, .. } if stage == "reverify")
    );
}
