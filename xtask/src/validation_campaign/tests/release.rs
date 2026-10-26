use std::{cell::RefCell, fs, path::Path, time::Duration};

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    executor::{RESTART_KEY, RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::Graph,
    model::{BlockReason, FailureClass},
    release::{
        freeze::FreezeRcRunner,
        offline::RcOfflineRunner,
        package::{find_ebpf_object, is_candidate_bundle, observation_recording},
        tag::{acceptance_receipt_problem, create_tag},
    },
    repository::{Binaries, reports_revision},
    shell::{Shell, ShellOutput},
};

const RC: &str = "0123456789abcdef0123456789abcdef01234567";

type RunHook = Box<dyn Fn(&str)>;

/// Answers by command prefix (first match) and records every command.
struct Machine {
    answers: Vec<(&'static str, ShellOutput)>,
    calls: std::rc::Rc<RefCell<Vec<String>>>,
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
            calls: Default::default(),
            on_run: None,
        }
    }
    fn calls_handle(&self) -> std::rc::Rc<RefCell<Vec<String>>> {
        std::rc::Rc::clone(&self.calls)
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

fn tag_release() -> crate::validation_campaign::manifest::ReleaseSection {
    manifest().release
}

fn tag_machine(extra: Vec<(&'static str, ShellOutput)>) -> Machine {
    let mut answers = vec![
        ("git rev-parse HEAD", out(0, &format!("{RC}\n"))),
        ("git status --porcelain", out(0, "")),
    ];
    answers.splice(0..0, extra);
    Machine::new(answers)
}

/// Nothing about verification or tagging may reach a remote or a hosted service.
fn assert_local_only(machine: &Machine) {
    for forbidden in [
        "gh ",
        "git push",
        "ls-remote",
        "git pull",
        "git fetch",
        "origin",
    ] {
        assert!(!machine.did(forbidden), "ran `{forbidden}`");
    }
}

#[test]
fn a_fresh_tag_is_created_annotated_on_the_candidate_and_stays_local() {
    let machine = tag_machine(vec![("git rev-parse -q --verify", out(1, ""))]);
    assert!(matches!(
        create_tag(&machine, Path::new("."), &tag_release(), RC),
        RunnerOutcome::Passed { .. }
    ));
    assert!(machine.did(&format!("git tag -a v0.1.0 {RC} -m Stutter v0.1.0")));
    assert_local_only(&machine);
    assert!(!machine.did("--force") && !machine.did("tag -d") && !machine.did("tag -f"));
}

#[test]
fn an_existing_tag_elsewhere_is_never_moved_and_one_on_the_candidate_is_idempotent() {
    let elsewhere = tag_machine(vec![(
        "git rev-parse -q --verify",
        out(0, "ffffffffffffffffffffffffffffffffffffffff\n"),
    )]);
    let RunnerOutcome::Failed { summary, class, .. } =
        create_tag(&elsewhere, Path::new("."), &tag_release(), RC)
    else {
        panic!("must fail")
    };
    assert!(
        summary.contains("FAILURE_REQUIRES_REVIEW") && class == FailureClass::ValidationFailure
    );
    assert!(!elsewhere.did("git tag -a"));
    assert_local_only(&elsewhere);
    let complete = tag_machine(vec![(
        "git rev-parse -q --verify",
        out(0, &format!("{RC}\n")),
    )]);
    assert!(matches!(
        create_tag(&complete, Path::new("."), &tag_release(), RC),
        RunnerOutcome::Passed { .. }
    ));
    assert!(!complete.did("git tag -a"));
    assert_local_only(&complete);
}

#[test]
fn tagging_requires_the_exact_candidate_and_a_clean_tree() {
    let moved = Machine::new(vec![(
        "git rev-parse HEAD",
        out(0, "ffffffffffffffffffffffffffffffffffffffff\n"),
    )]);
    assert!(
        matches!(create_tag(&moved, Path::new("."), &tag_release(), RC), RunnerOutcome::Failed { stage, .. } if stage == "head")
    );
    let dirty = Machine::new(vec![
        ("git rev-parse HEAD", out(0, &format!("{RC}\n"))),
        ("git status --porcelain", out(0, " M x\n")),
    ]);
    assert!(
        matches!(create_tag(&dirty, Path::new("."), &tag_release(), RC), RunnerOutcome::Failed { stage, .. } if stage == "clean")
    );
    assert!(!dirty.did("git tag"));
}

struct ReceiptCheck;

impl TaskRunner for ReceiptCheck {
    fn run(
        &self,
        context: &RunContext<'_>,
        _: &crate::validation_campaign::graph::TaskSpec,
        _: u32,
    ) -> RunnerOutcome {
        match acceptance_receipt_problem(context, RC) {
            Ok(()) => RunnerOutcome::passed(),
            Err(problem) => RunnerOutcome::Failed {
                class: FailureClass::SoftwareDefect,
                stage: "acceptance".to_owned(),
                summary: problem,
                artifacts: None,
            },
        }
    }
}

#[test]
fn the_tag_needs_the_local_exact_candidate_acceptance_receipt() {
    for (name, receipt, passes) in [
        ("missing", None, false),
        (
            "failing",
            Some(format!(
                r#"{{"status":"FAIL","commit":"{RC}","fresh_checkout":true}}"#
            )),
            false,
        ),
        (
            "other-commit",
            Some(r#"{"status":"PASS","commit":"ffff","fresh_checkout":true}"#.to_owned()),
            false,
        ),
        (
            "not-fresh",
            Some(format!(
                r#"{{"status":"PASS","commit":"{RC}","fresh_checkout":false}}"#
            )),
            false,
        ),
        (
            "good",
            Some(format!(
                r#"{{"status":"PASS","commit":"{RC}","fresh_checkout":true}}"#
            )),
            true,
        ),
    ] {
        let scratch = Scratch::new(&format!("receipt-{name}"));
        let (dir, ..) = fresh(&scratch);
        if let Some(receipt) = &receipt {
            fs::create_dir_all(dir.0.join("release")).unwrap();
            fs::write(dir.0.join("release/offline-acceptance.json"), receipt).unwrap();
        }
        let (outcome, _) = context_run(&scratch, "tag", &ReceiptCheck, |_| {});
        assert_eq!(
            matches!(outcome, RunnerOutcome::Passed { .. }),
            passes,
            "{name}"
        );
    }
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
        calls: Default::default(),
        on_run: None,
    };
    let calls = machine.calls_handle();
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
    assert!(
        !calls.borrow().iter().any(|c| c.contains("pull")),
        "{:?}",
        calls.borrow()
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
fn offline_acceptance_needs_a_pass_receipt_for_the_exact_candidate_and_asks_no_hosted_service() {
    let scratch = Scratch::new("rc-offline");
    let (dir, ..) = fresh(&scratch);
    let receipt = dir.0.join("release/offline-acceptance.json");
    let mut machine = Machine::new(vec![]);
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
    let newer = scratch
        .path()
        .join("attempts/rc-live-observation/0002/raw/observe/run");
    fs::create_dir_all(&newer).unwrap();
    fs::write(newer.join("session.json"), "{}").unwrap();
    assert_eq!(observation_recording(scratch.path()), Some(newer));
}

#[test]
fn only_the_current_candidates_bundles_count() {
    let rc = "b3d344f14b25c82b3fcd293fda870068d01de53c";
    assert!(is_candidate_bundle(
        "live_affinity-b3d344f-1792819834659",
        rc
    ));
    assert!(!is_candidate_bundle(
        "live_affinity-fec0f0e-1792818002792",
        rc
    ));
    assert!(!is_candidate_bundle(".partial-0002", rc));
    assert!(!is_candidate_bundle("live_affinity-b3d344f-1", ""));
    assert!(is_candidate_bundle(
        "live_affinity-b3d344fa-1792819834659",
        "b3d344fa14b25c82b3fcd293fda870068d01de53"
    ));
    assert!(reports_revision("stutter 0.1.0 (git b3d344f)", rc));
    assert!(reports_revision("stutter 0.1.0 (git b3d344f1)", rc));
    assert!(!reports_revision("stutter 0.1.0 (git fec0f0e)", rc));
    assert!(!reports_revision("stutter 0.1.0 (git b3d3)", rc));
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
