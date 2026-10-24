use std::{cell::RefCell, fs, path::Path, time::Duration};

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    executor::{RESTART_KEY, RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::Graph,
    model::{BlockReason, FailureClass},
    release::{
        ci_must_be_green,
        freeze::FreezeRcRunner,
        normalized_check_runs,
        offline::RcOfflineRunner,
        package::{find_ebpf_object, observation_recording},
        tag::create_tag,
    },
    repo_pr::WaitPolicy,
    repository::Binaries,
    shell::{Shell, ShellOutput},
};

const RC: &str = "0123456789abcdef0123456789abcdef01234567";

type RunHook = Box<dyn Fn(&str)>;

/// Answers by command prefix (first match) and records every command.
struct Machine {
    answers: Vec<(&'static str, ShellOutput)>,
    calls: RefCell<Vec<String>>,
    on_run: Option<RunHook>,
}

fn out(code: i32, stdout: &str) -> ShellOutput {
    ShellOutput {
        code,
        stdout: stdout.to_owned(),
        stderr: String::new(),
    }
}

impl Machine {
    fn new(answers: Vec<(&'static str, ShellOutput)>) -> Self {
        Self {
            answers,
            calls: RefCell::default(),
            on_run: None,
        }
    }
    fn did(&self, needle: &str) -> bool {
        self.calls.borrow().iter().any(|c| c.contains(needle))
    }
}

impl Shell for Machine {
    fn run(&self, program: &str, args: &[&str], _: &Path) -> ShellOutput {
        let line = format!("{program} {}", args.join(" "));
        self.calls.borrow_mut().push(line.clone());
        if let Some(hook) = &self.on_run {
            hook(&line);
        }
        self.answers
            .iter()
            .find(|(p, _)| line.starts_with(p))
            .map_or_else(|| out(0, ""), |(_, o)| o.clone())
    }
    fn sleep(&self, _: Duration) {}
}

fn wait() -> WaitPolicy {
    WaitPolicy {
        poll: Duration::ZERO,
        max_polls: 3,
    }
}

const GREEN_CI: &str = r#"{"check_runs":[{"name":"build","status":"completed","conclusion":"success"},{"name":"live smoke tests","status":"completed","conclusion":"skipped"}]}"#;

#[test]
fn exact_commit_ci_is_normalized_and_only_conclusive_success_passes() {
    assert_eq!(normalized_check_runs(GREEN_CI)[0]["conclusion"], "SUCCESS");
    let machine = Machine::new(vec![("gh api", out(0, GREEN_CI))]);
    assert!(ci_must_be_green(&machine, Path::new("."), RC, &wait()).is_ok());
    for (body, expect_failed) in [
        (
            r#"{"check_runs":[{"name":"build","status":"completed","conclusion":"failure"}]}"#,
            true,
        ),
        (
            r#"{"check_runs":[{"name":"build","status":"completed","conclusion":"cancelled"}]}"#,
            true,
        ),
        (
            r#"{"check_runs":[{"name":"build","status":"in_progress","conclusion":null}]}"#,
            false,
        ),
        (r#"{"check_runs":[]}"#, false),
    ] {
        let machine = Machine::new(vec![("gh api", out(0, body))]);
        let outcome = ci_must_be_green(&machine, Path::new("."), RC, &wait()).unwrap_err();
        assert_eq!(
            matches!(
                outcome,
                RunnerOutcome::Failed {
                    class: FailureClass::SoftwareDefect,
                    ..
                }
            ),
            expect_failed,
            "{body}"
        );
    }
    let unreadable = Machine::new(vec![("gh api", out(1, ""))]);
    assert!(matches!(
        ci_must_be_green(&unreadable, Path::new("."), RC, &wait()),
        Err(RunnerOutcome::Blocked(_))
    ));
}

fn tag_release() -> crate::validation_campaign::manifest::ReleaseSection {
    manifest().release
}

fn tag_machine(extra: Vec<(&'static str, ShellOutput)>) -> Machine {
    let mut answers = vec![
        ("git rev-parse HEAD", out(0, &format!("{RC}\n"))),
        ("git status --porcelain", out(0, "")),
        ("gh api", out(0, GREEN_CI)),
    ];
    answers.splice(0..0, extra);
    Machine::new(answers)
}

#[test]
fn a_fresh_tag_is_created_annotated_on_the_candidate_and_pushed() {
    let machine = tag_machine(vec![
        ("git rev-parse -q --verify", out(1, "")),
        ("git ls-remote", out(0, "")),
    ]);
    assert!(matches!(
        create_tag(&machine, Path::new("."), &tag_release(), RC, &wait()),
        RunnerOutcome::Passed { .. }
    ));
    assert!(machine.did(&format!("git tag -a v0.1.0 {RC} -m Stutter v0.1.0")));
    assert!(machine.did("git push origin v0.1.0"));
    assert!(!machine.did("--force") && !machine.did("tag -d") && !machine.did("tag -f"));
}

#[test]
fn an_existing_tag_elsewhere_is_never_moved_and_one_on_the_candidate_is_idempotent() {
    let elsewhere = tag_machine(vec![(
        "git rev-parse -q --verify",
        out(0, "ffffffffffffffffffffffffffffffffffffffff\n"),
    )]);
    let RunnerOutcome::Failed { summary, class, .. } =
        create_tag(&elsewhere, Path::new("."), &tag_release(), RC, &wait())
    else {
        panic!("must fail")
    };
    assert!(
        summary.contains("FAILURE_REQUIRES_REVIEW") && class == FailureClass::ValidationFailure
    );
    assert!(!elsewhere.did("git tag -a") && !elsewhere.did("git push"));
    let on_origin = tag_machine(vec![
        ("git rev-parse -q --verify", out(1, "")),
        ("git ls-remote", out(0, "abc\trefs/tags/v0.1.0\n")),
    ]);
    assert!(matches!(
        create_tag(&on_origin, Path::new("."), &tag_release(), RC, &wait()),
        RunnerOutcome::Failed { .. }
    ));
    // Created locally by an interrupted attempt: only the push is repeated.
    let local_only = tag_machine(vec![
        ("git rev-parse -q --verify", out(0, &format!("{RC}\n"))),
        ("git ls-remote", out(0, "")),
    ]);
    assert!(matches!(
        create_tag(&local_only, Path::new("."), &tag_release(), RC, &wait()),
        RunnerOutcome::Passed { .. }
    ));
    assert!(local_only.did("git push origin v0.1.0") && !local_only.did("git tag -a"));
    let complete = tag_machine(vec![
        ("git rev-parse -q --verify", out(0, &format!("{RC}\n"))),
        ("git ls-remote", out(0, "abc\trefs/tags/v0.1.0\n")),
    ]);
    assert!(matches!(
        create_tag(&complete, Path::new("."), &tag_release(), RC, &wait()),
        RunnerOutcome::Passed { .. }
    ));
    assert!(!complete.did("git push"));
}

#[test]
fn tagging_requires_the_exact_candidate_a_clean_tree_and_green_ci() {
    let moved = Machine::new(vec![(
        "git rev-parse HEAD",
        out(0, "ffffffffffffffffffffffffffffffffffffffff\n"),
    )]);
    assert!(
        matches!(create_tag(&moved, Path::new("."), &tag_release(), RC, &wait()), RunnerOutcome::Failed { stage, .. } if stage == "head")
    );
    let dirty = Machine::new(vec![
        ("git rev-parse HEAD", out(0, &format!("{RC}\n"))),
        ("git status --porcelain", out(0, " M x\n")),
    ]);
    assert!(
        matches!(create_tag(&dirty, Path::new("."), &tag_release(), RC, &wait()), RunnerOutcome::Failed { stage, .. } if stage == "clean")
    );
    let red = tag_machine(vec![(
        "gh api",
        out(
            0,
            r#"{"check_runs":[{"name":"b","status":"completed","conclusion":"failure"}]}"#,
        ),
    )]);
    assert!(
        matches!(create_tag(&red, Path::new("."), &tag_release(), RC, &wait()), RunnerOutcome::Failed { stage, .. } if stage == "ci")
    );
    assert!(!red.did("git tag"));
}

fn context_run(
    scratch: &Scratch,
    kind: &str,
    runner: &dyn TaskRunner,
    configure: impl FnOnce(&mut crate::validation_campaign::model::CampaignState),
) -> (RunnerOutcome, Vec<StateUpdate>) {
    let (dir, mut state, _) = fresh(scratch);
    configure(&mut state);
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
    let spec = Graph::core().unwrap().get(kind).unwrap().clone();
    let outcome = runner.run(&context, &spec, 1);
    (outcome, updates.into_inner())
}

#[test]
fn freezing_records_the_candidate_its_own_binary_digests_and_asks_for_a_restart() {
    let scratch = Scratch::new("freeze");
    fs::create_dir_all(scratch.path().join("target/debug")).unwrap();
    fs::write(scratch.path().join("target/debug/stutter"), b"stutter-bin").unwrap();
    fs::write(scratch.path().join("target/debug/xtask"), b"xtask-bin").unwrap();
    let version = format!("stutter 0.1.0 (git {})\n", &RC[..7]);
    let stutter_path = scratch.path().join("target/debug/stutter");
    let machine = Machine::new(vec![
        ("git rev-parse HEAD", out(0, &format!("{RC}\n"))),
        ("git status --porcelain", out(0, "")),
    ]);
    let version_cmd = format!("{} --version", stutter_path.display());
    let machine = Machine {
        answers: [
            vec![(
                Box::leak(version_cmd.into_boxed_str()) as &str,
                out(0, &version),
            )],
            machine.answers,
        ]
        .concat(),
        calls: RefCell::default(),
        on_run: None,
    };
    let repinned = std::rc::Rc::new(RefCell::new(Vec::new()));
    let seen = std::rc::Rc::clone(&repinned);
    let runner = FreezeRcRunner {
        shell: Box::new(machine),
        repin: Box::new(move |_, stutter, xtask| {
            seen.borrow_mut()
                .push((stutter.to_owned(), xtask.to_owned()));
            Ok(())
        }),
    };
    let (outcome, _) = context_run(&scratch, "freeze-rc", &runner, |_| {});
    let RunnerOutcome::Passed { updates, .. } = outcome else {
        panic!("freeze must pass")
    };
    assert!(matches!(&updates[0], StateUpdate::SetReleaseCandidate(c) if c == RC));
    assert!(
        matches!(&updates[1], StateUpdate::SetReleaseBinaries { stutter_sha256, xtask_sha256 } if stutter_sha256.len() == 64 && xtask_sha256.len() == 64 && stutter_sha256 != xtask_sha256)
    );
    assert!(
        matches!(&updates[2], StateUpdate::Freeze { key, value } if key == RESTART_KEY && value == "1")
    );
    assert_eq!(
        repinned.borrow().len(),
        1,
        "the runner is re-pinned to the new digests exactly once"
    );
}

#[test]
fn a_dirty_main_a_wrong_revision_or_a_refused_repin_never_freezes_a_candidate() {
    let scratch = Scratch::new("freeze-bad");
    let dirty = FreezeRcRunner {
        shell: Box::new(Machine::new(vec![(
            "git status --porcelain",
            out(0, " M file\n"),
        )])),
        repin: Box::new(|_, _, _| Ok(())),
    };
    assert!(matches!(
        context_run(&scratch, "freeze-rc", &dirty, |_| {}).0,
        RunnerOutcome::Blocked(_)
    ));
    let scratch = Scratch::new("freeze-privilege");
    fs::create_dir_all(scratch.path().join("target/debug")).unwrap();
    fs::write(scratch.path().join("target/debug/stutter"), b"s").unwrap();
    fs::write(scratch.path().join("target/debug/xtask"), b"x").unwrap();
    let version_cmd: &'static str = Box::leak(
        format!(
            "{} --version",
            scratch.path().join("target/debug/stutter").display()
        )
        .into_boxed_str(),
    );
    let privileged = FreezeRcRunner {
        shell: Box::new(Machine::new(vec![
            ("git rev-parse HEAD", out(0, &format!("{RC}\n"))),
            (
                version_cmd,
                out(0, &format!("stutter 0.1.0 (git {})\n", &RC[..7])),
            ),
        ])),
        repin: Box::new(|_, _, _| {
            Err(BlockReason {
                class: FailureClass::BlockedPrivilege,
                detail: "no sudo".to_owned(),
            })
        }),
    };
    let RunnerOutcome::Blocked(block) = context_run(&scratch, "freeze-rc", &privileged, |_| {}).0
    else {
        panic!("must block")
    };
    assert_eq!(block.class, FailureClass::BlockedPrivilege);
}

#[test]
fn offline_acceptance_needs_a_pass_receipt_for_the_exact_candidate_and_green_ci() {
    let scratch = Scratch::new("rc-offline");
    let (dir, ..) = fresh(&scratch);
    let receipt = dir.0.join("release/offline-acceptance.json");
    let mut machine = Machine::new(vec![("gh api", out(0, GREEN_CI))]);
    let path = receipt.clone();
    machine.on_run = Some(Box::new(move |line| {
        if line.starts_with("scripts/release/offline-acceptance.sh") {
            fs::write(
                &path,
                format!(r#"{{"status":"PASS","commit":"{RC}","fresh_checkout":true}}"#),
            )
            .unwrap();
        }
    }));
    let runner = RcOfflineRunner {
        shell: Box::new(machine),
        wait: wait(),
    };
    let (outcome, _) = context_run(&scratch, "rc-offline", &runner, |state| {
        state.release_candidate = Some(RC.to_owned())
    });
    assert!(matches!(outcome, RunnerOutcome::Passed { .. }));
    let scratch = Scratch::new("rc-offline-fail");
    let (dir, ..) = fresh(&scratch);
    let path = dir.0.join("release/offline-acceptance.json");
    let mut failing = Machine::new(vec![("scripts/release/offline-acceptance.sh", out(1, ""))]);
    failing.on_run = Some(Box::new(move |line| {
        if line.starts_with("scripts/release/offline-acceptance.sh") {
            fs::write(
                &path,
                format!(r#"{{"status":"FAIL","commit":"{RC}","fresh_checkout":true}}"#),
            )
            .unwrap();
        }
    }));
    let runner = RcOfflineRunner {
        shell: Box::new(failing),
        wait: wait(),
    };
    let (outcome, _) = context_run(&scratch, "rc-offline", &runner, |state| {
        state.release_candidate = Some(RC.to_owned())
    });
    assert!(matches!(
        outcome,
        RunnerOutcome::Failed {
            class: FailureClass::Retryable,
            ..
        }
    ));
    let (unfrozen, _) = context_run(
        &Scratch::new("rc-offline-none"),
        "rc-offline",
        &RcOfflineRunner {
            shell: Box::new(Machine::new(vec![])),
            wait: wait(),
        },
        |_| {},
    );
    assert!(matches!(unfrozen, RunnerOutcome::Blocked(_)));
}

#[test]
fn the_ebpf_object_is_the_newest_build_output_and_the_recording_is_found_by_its_session() {
    let scratch = Scratch::new("package-find");
    let build = scratch.path().join("build");
    for name in ["stutter-aaa", "other-bbb"] {
        fs::create_dir_all(build.join(name).join("out")).unwrap();
        fs::write(build.join(name).join("out/stutter"), b"\x7fELF").unwrap();
    }
    assert_eq!(
        find_ebpf_object(&build),
        Some(build.join("stutter-aaa/out/stutter"))
    );
    assert_eq!(find_ebpf_object(&scratch.path().join("absent")), None);
    let recording = scratch
        .path()
        .join("attempts/rc-live-observation/0001/raw/observe/run");
    fs::create_dir_all(&recording).unwrap();
    assert_eq!(observation_recording(scratch.path()), None);
    fs::write(recording.join("session.json"), "{}").unwrap();
    assert_eq!(observation_recording(scratch.path()), Some(recording));
}

#[test]
fn every_task_kind_has_a_real_runner() {
    let runners = crate::validation_campaign::runners::default_runners();
    for spec in Graph::core().unwrap().specs() {
        assert!(
            runners.contains_key(&spec.kind),
            "{} has no runner",
            spec.id
        );
    }
}
