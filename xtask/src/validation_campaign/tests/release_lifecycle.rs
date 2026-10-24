//! One end-to-end regression for the release-candidate lifecycle: collection finished, three
//! candidates (two with late defects), stale candidates' bundles and recordings left on disk, the
//! real freeze/offline/package/verify/smoke/tag runners driven by `continue_campaign` against a
//! stateful scripted machine, and real evidence bundles published through the evidence pipeline.
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use serde_json::{Value, json};

use super::{BASE, Scratch, fresh, manifest};
use crate::{
    evidence_bundle::{
        sha256::sha256_hex,
        tests::{Dir, raw},
    },
    validation_campaign::{
        evidence::{Accept, PublishRequest, publish_bundle},
        executor::{Executor, RESTART_KEY, RunContext, RunnerOutcome, TaskRunner},
        graph::{Graph, SourceBinding, TaskKind, TaskSpec, core_validation_tasks},
        model::{Boundary, CampaignMode, CampaignState, FailureClass, TaskStatus},
        release::{freeze::FreezeRcRunner, offline::RcOfflineRunner, package::*, tag::TagRunner},
        repo_pr::WaitPolicy,
        repository::Binaries,
        shell::{Shell, ShellOutput},
        state::{CampaignDir, write_atomic},
        verify::verify_state,
    },
};

const GREEN_CI: &str =
    r#"{"check_runs":[{"name":"build","status":"completed","conclusion":"success"}]}"#;
const BPF: &[u8] = b"\x7fELF-object";

fn commit(c: char) -> String {
    c.to_string().repeat(40)
}

#[derive(Default)]
struct World {
    repo: PathBuf,
    head: RefCell<String>,
    /// Abbreviation length printed by the freshly built binary (the packaged one uses the other).
    eight: Cell<bool>,
    smoke_fails_at: RefCell<BTreeSet<String>>,
    verify_fails_at: RefCell<BTreeSet<String>>,
    existing_tag: RefCell<Option<String>>,
    /// Per `release-verify` call: the revision it verified and the commit of each bundle given.
    verifies: RefCell<Vec<(String, Vec<String>)>>,
    validated: RefCell<Vec<String>>,
    tagged: RefCell<Vec<String>>,
    pushed: RefCell<Vec<String>>,
    captures: Cell<u64>,
}

impl World {
    fn abbreviation(&self, eight: bool) -> String {
        let head = self.head.borrow();
        head[..if eight { 8 } else { 7 }].to_owned()
    }
}

struct Machine(Rc<World>);

fn done(code: i32, stdout: &str, stderr: &str) -> ShellOutput {
    ShellOutput {
        code,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

fn arg_after<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    args.iter().position(|a| *a == flag).map(|i| args[i + 1])
}

impl Machine {
    fn git(&self, args: &[&str]) -> ShellOutput {
        let world = &self.0;
        match args {
            ["checkout", _] | ["pull", _] | ["push", "origin", ..] => {
                if args[0] == "push" {
                    world.pushed.borrow_mut().push(args[2].to_owned());
                }
                done(0, "", "")
            }
            ["status", _] => done(0, "", ""),
            ["rev-parse", "HEAD"] => done(0, &format!("{}\n", world.head.borrow()), ""),
            ["rev-parse", "-q", "--verify", _] => match &*world.existing_tag.borrow() {
                Some(commit) => done(0, &format!("{commit}\n"), ""),
                None => done(1, "", ""),
            },
            ["ls-remote", ..] => match &*world.existing_tag.borrow() {
                Some(commit) => done(0, &format!("{commit}\trefs/tags/v0.1.0\n"), ""),
                None => done(0, "", ""),
            },
            ["tag", "-a", _, commit, ..] => {
                world.tagged.borrow_mut().push((*commit).to_owned());
                done(0, "", "")
            }
            _ => done(127, "", "unscripted git command"),
        }
    }

    fn xtask(&self, args: &[&str]) -> ShellOutput {
        let world = &self.0;
        match args[0] {
            "release-package" => {
                let out = PathBuf::from(arg_after(args, "--out").unwrap());
                fs::create_dir_all(&out).unwrap();
                let head = world.head.borrow().clone();
                let manifest = json!({"version":"0.1.0","git_revision":head,
                    "bpf_object":{"sha256":sha256_hex(BPF)}});
                fs::write(out.join("release-manifest.json"), manifest.to_string()).unwrap();
                fs::write(out.join("stutter-0.1.0-linux-x86_64.tar.gz"), b"tarball").unwrap();
                done(0, "", "")
            }
            "release-verify" => {
                let package = PathBuf::from(args[1]);
                let manifest: Value = serde_json::from_slice(
                    &fs::read(package.join("release-manifest.json")).unwrap(),
                )
                .unwrap();
                let revision = manifest["git_revision"].as_str().unwrap().to_owned();
                let commits: Vec<String> = args
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| **a == "--acceptance-bundle")
                    .map(|(i, _)| {
                        let env: Value = serde_json::from_slice(
                            &fs::read(Path::new(args[i + 1]).join("environment.json")).unwrap(),
                        )
                        .unwrap();
                        env["commit"].as_str().unwrap().to_owned()
                    })
                    .collect();
                world
                    .verifies
                    .borrow_mut()
                    .push((revision.clone(), commits.clone()));
                if world.verify_fails_at.borrow().contains(&revision) {
                    return done(1, "", "Error: scripted verification defect");
                }
                if commits.len() != 4 || commits.iter().any(|c| *c != revision) {
                    return done(1, "", "Error: a bundle is for another commit");
                }
                done(0, "", "")
            }
            _ => done(127, "", "unscripted xtask command"),
        }
    }

    fn packaged(&self, args: &[&str]) -> ShellOutput {
        let world = &self.0;
        if world
            .smoke_fails_at
            .borrow()
            .contains(&*world.head.borrow())
        {
            return done(1, "", "scripted install-smoke defect");
        }
        match args {
            ["--version"] => done(
                0,
                &format!(
                    "stutter 0.1.0 (git {})\n",
                    world.abbreviation(!world.eight.get())
                ),
                "",
            ),
            ["capabilities", "--json"] => {
                let supported = manifest().policy.capabilities.supported.clone();
                let list: Vec<Value> = supported
                    .iter()
                    .map(|id| json!({"id":id,"tier":"supported"}))
                    .collect();
                done(0, &Value::Array(list).to_string(), "")
            }
            ["doctor", "--json"] => done(1, "{\"overall\":\"Fail\"}", ""),
            ["restore", "--help"] | ["audit", "--help"] => done(0, "usage", ""),
            ["validate", recording] => {
                world.validated.borrow_mut().push((*recording).to_owned());
                done(0, "", "")
            }
            ["report", _, "--analysis-json"] => done(0, "{\"cluster_analysis\":{}}", ""),
            _ => done(127, "", "unscripted packaged command"),
        }
    }
}

impl Shell for Machine {
    fn run(&self, program: &str, args: &[&str], _: &Path) -> ShellOutput {
        let world = &self.0;
        if program == "git" {
            return self.git(args);
        }
        if program == "gh" {
            return done(0, GREEN_CI, "");
        }
        if program == "tar" {
            let install = PathBuf::from(arg_after(args, "-C").unwrap());
            fs::create_dir_all(install.join("lib/stutter")).unwrap();
            fs::create_dir_all(install.join("bin")).unwrap();
            fs::write(install.join("lib/stutter/stutter.bpf.o"), BPF).unwrap();
            return done(0, "", "");
        }
        if program == "cargo" {
            if args.contains(&"--release") {
                return done(0, "", "");
            }
            let head = world.head.borrow().clone();
            for name in ["stutter", "xtask"] {
                fs::write(
                    world.repo.join("target/debug").join(name),
                    format!("{name} built at {head}"),
                )
                .unwrap();
            }
            return done(0, "", "");
        }
        if program == "scripts/release/offline-acceptance.sh" {
            let receipt = json!({"status":"PASS","commit":args[0],"fresh_checkout":true});
            fs::write(args[1], receipt.to_string()).unwrap();
            return done(0, "", "");
        }
        if program.ends_with("install/bin/stutter") {
            return self.packaged(args);
        }
        if program.ends_with("target/debug/stutter") && args == ["--version"] {
            let version = format!(
                "stutter 0.1.0 (git {})\n",
                world.abbreviation(world.eight.get())
            );
            return done(0, &version, "");
        }
        if program.ends_with("target/debug/xtask") {
            return self.xtask(args);
        }
        done(127, "", &format!("unscripted command {program}"))
    }

    fn sleep(&self, _: Duration) {}
}

/// A verifying raw suite directory for `commit`, with its own capture id and kernel.
fn raw_for(world: &World, suite: &str, commit: &str, kernel: &str) -> Dir {
    let raw_dir = raw(suite, "ok");
    let n = world.captures.get() + 1;
    world.captures.set(n);
    let capture = format!("{suite}-{}-{n}", &commit[..7]);
    for file in ["environment.json", "result.json"] {
        let path = raw_dir.join(file);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["capture_id"] = json!(capture);
        if file == "environment.json" {
            value["commit"] = json!(commit);
            value["binary_version"] = json!(format!("stutter 0.1.0 (git {})", &commit[..7]));
            value["kernel_release"] = json!(kernel);
        }
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    raw_dir
}

/// Publishes real, verifying bundles for the suite(s) a task owns, bound to the task's commit.
struct Publisher {
    world: Rc<World>,
    suite: &'static str,
    kernels: &'static [&'static str],
}

impl TaskRunner for Publisher {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let commit = match spec.binding {
            SourceBinding::ReleaseCandidate => context.state.release_candidate.clone().unwrap(),
            _ => context.state.evidence_base.clone(),
        };
        for kernel in self.kernels {
            let raw_dir = raw_for(&self.world, self.suite, &commit, kernel);
            let published = publish_bundle(&PublishRequest {
                dir: context.dir,
                repository: context.repository,
                task: spec.id,
                attempt,
                raw: &raw_dir.0,
                expected_commit: &commit,
                allowed_kinds: &["controlled_real", "real_workload"],
                accept: Accept::PassOnly,
            });
            if let Err(error) = published {
                return RunnerOutcome::Failed {
                    class: FailureClass::SoftwareDefect,
                    stage: "publish".to_owned(),
                    summary: format!("{error:?}"),
                    artifacts: None,
                };
            }
        }
        if spec.id == "rc-live-observation" {
            let recording = context
                .dir
                .attempt_dir(spec.id, attempt)
                .join("raw/recording");
            fs::create_dir_all(&recording).unwrap();
            fs::write(recording.join("session.json"), "{}").unwrap();
        }
        RunnerOutcome::passed()
    }
}

struct Pass;
impl TaskRunner for Pass {
    fn run(&self, _: &RunContext<'_>, _: &TaskSpec, _: u32) -> RunnerOutcome {
        RunnerOutcome::passed()
    }
}

fn wait() -> WaitPolicy {
    WaitPolicy {
        poll: Duration::ZERO,
        max_polls: 2,
    }
}

fn shell(world: &Rc<World>) -> Box<dyn Shell> {
    Box::new(Machine(Rc::clone(world)))
}

fn executor(world: &Rc<World>) -> Executor {
    let publisher = |suite, kernels| -> Box<dyn TaskRunner> {
        Box::new(Publisher {
            world: Rc::clone(world),
            suite,
            kernels,
        })
    };
    let mut runners: BTreeMap<TaskKind, Box<dyn TaskRunner>> = BTreeMap::new();
    for spec in core_validation_tasks() {
        runners.insert(spec.kind, Box::new(Pass));
    }
    let kernels: &'static [&'static str] = &["6.12.1", "6.18.1", "7.2.1"];
    runners.insert(
        TaskKind::LiveObservation,
        publisher("live_observation", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::GameRunnable,
        publisher("live_game_runnable", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::OverheadControlled,
        publisher("live_overhead", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::OverheadReal,
        publisher("live_overhead", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::ScoreRunnable,
        publisher("live_score_degradation", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::KernelMatrix,
        publisher("live_kernel_compat", kernels),
    );
    runners.insert(
        TaskKind::RcLiveObservation,
        publisher("live_observation", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::RcGameRunnable,
        publisher("live_game_runnable", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::RcAffinity,
        publisher("live_affinity", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::RcRecovery,
        publisher("live_recovery", &["6.18.1"]),
    );
    runners.insert(
        TaskKind::FreezeRc,
        Box::new(FreezeRcRunner {
            shell: shell(world),
            repin: Box::new(|_, _, _| Ok(())),
        }),
    );
    runners.insert(
        TaskKind::RcOffline,
        Box::new(RcOfflineRunner {
            shell: shell(world),
            wait: wait(),
        }),
    );
    runners.insert(
        TaskKind::ReleasePackage,
        Box::new(ReleasePackageRunner {
            shell: shell(world),
        }),
    );
    runners.insert(
        TaskKind::ReleaseVerify,
        Box::new(ReleaseVerifyRunner {
            shell: shell(world),
        }),
    );
    runners.insert(
        TaskKind::InstallSmoke,
        Box::new(InstallSmokeRunner {
            shell: shell(world),
        }),
    );
    runners.insert(
        TaskKind::Tag,
        Box::new(TagRunner {
            shell: shell(world),
            wait: wait(),
        }),
    );
    let check_world = Rc::clone(world);
    Executor {
        graph: Graph::core().unwrap(),
        manifest: manifest(),
        runners,
        repository: world.repo.clone(),
        binaries: Binaries {
            stutter: world.repo.join("target/debug/stutter"),
            xtask: world.repo.join("target/debug/xtask"),
        },
        // `main` moving away from the frozen candidate is what the production check reports.
        source_check: Box::new(move |state, spec| {
            if spec.binding == SourceBinding::ReleaseCandidate
                && state.release_candidate.as_deref() != Some(check_world.head.borrow().as_str())
            {
                return Err("HEAD is not the release candidate".to_owned());
            }
            Ok(())
        }),
    }
}

fn setup(name: &str) -> (Scratch, Rc<World>, CampaignDir, CampaignState) {
    let scratch = Scratch::new(name);
    let (dir, mut state, _) = fresh(&scratch);
    write_atomic(&dir.manifest_snapshot(), super::MANIFEST_TEXT.as_bytes()).unwrap();
    state.manifest_sha256 = sha256_hex(super::MANIFEST_TEXT.as_bytes());
    state.evidence_base = BASE.to_owned();
    fs::create_dir_all(scratch.path().join("target/debug")).unwrap();
    let build = scratch.path().join("target/release/build/stutter-abc/out");
    fs::create_dir_all(&build).unwrap();
    fs::write(build.join("stutter"), BPF).unwrap();
    let world = Rc::new(World {
        repo: scratch.path().to_path_buf(),
        head: RefCell::new(commit('1')),
        ..World::default()
    });
    (scratch, world, dir, state)
}

fn drive(executor: &Executor, dir: &CampaignDir, state: &mut CampaignState) -> Boundary {
    for _ in 0..30 {
        let report = executor.continue_campaign(dir, state, None).unwrap();
        if report.boundary != Boundary::Idle {
            return report.boundary;
        }
        // The production controller consumes the restart request by re-executing itself.
        state.frozen.remove(RESTART_KEY);
    }
    panic!("the campaign never reached a boundary");
}

fn bundles(dir: &CampaignDir, task: &str) -> usize {
    fs::read_dir(dir.bundles().join(task)).map_or(0, |d| d.count())
}

#[test]
fn three_candidates_two_late_defects_one_tag_on_the_last() {
    let (_scratch, world, dir, mut state) = setup("lifecycle");
    let executor = executor(&world);
    let (rc1, rc2, rc3) = (commit('1'), commit('2'), commit('3'));

    // RC1: its install smoke finds a defect.
    world.smoke_fails_at.borrow_mut().insert(rc1.clone());
    assert_eq!(
        drive(&executor, &dir, &mut state),
        Boundary::FailureRequiresSourceChange
    );
    assert_eq!(state.release_candidate.as_deref(), Some(rc1.as_str()));
    assert_eq!(state.mode, CampaignMode::ReleaseCandidate);

    // The fix lands (main moves); RC2's verification finds a second defect.
    *world.head.borrow_mut() = rc2.clone();
    world.eight.set(true);
    world.verify_fails_at.borrow_mut().insert(rc2.clone());
    assert_eq!(
        drive(&executor, &dir, &mut state),
        Boundary::FailureRequiresSourceChange
    );
    assert_eq!(state.release_candidate.as_deref(), Some(rc2.as_str()));
    assert_eq!(
        bundles(&dir, "rc-affinity"),
        2,
        "RC1's bundles stay as history"
    );

    // The second fix lands; RC3 passes everything and is tagged.
    *world.head.borrow_mut() = rc3.clone();
    world.eight.set(false);
    assert_eq!(drive(&executor, &dir, &mut state), Boundary::Complete);
    assert_eq!(state.mode, CampaignMode::Complete);
    assert_eq!(state.release_candidate.as_deref(), Some(rc3.as_str()));

    // Stale candidates never certified the final one.
    assert_eq!(bundles(&dir, "rc-affinity"), 3);
    for (revision, commits) in world.verifies.borrow().iter() {
        assert!(
            commits.iter().all(|c| c == revision),
            "{revision}: {commits:?}"
        );
    }
    let (revision, commits) = world.verifies.borrow().last().cloned().unwrap();
    assert_eq!((revision.as_str(), commits.len()), (rc3.as_str(), 4));
    let manifest: Value = serde_json::from_slice(
        &fs::read(dir.0.join("release/package/release-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["git_revision"], rc3.as_str());

    // Install smoke validated the final candidate's observation, not an earlier attempt's.
    let validated = world.validated.borrow().clone();
    assert!(
        validated
            .last()
            .unwrap()
            .contains("rc-live-observation/0003/"),
        "{validated:?}"
    );

    // Bounded refreezing, not terminal invalidation; collection was never recollected.
    assert_eq!(state.tasks["freeze-rc"].attempts.len(), 3);
    assert_eq!(state.tasks["kernel-matrix"].attempts.len(), 1);
    assert_eq!(state.tasks["live-observation"].attempts.len(), 1);
    let refreezes = state
        .decisions
        .iter()
        .filter(|d| d.decision.starts_with("new release candidate required"))
        .count();
    assert_eq!(refreezes, 2);

    // Strict verification ignores the stale candidates' evidence.
    let problems = verify_state(&dir, &state, &executor.graph, &executor.manifest);
    assert!(problems.is_empty(), "{problems:?}");

    // One annotated tag, on the final candidate only, pushed once.
    assert_eq!(*world.tagged.borrow(), vec![rc3]);
    assert_eq!(*world.pushed.borrow(), vec!["v0.1.0".to_owned()]);
    assert_eq!(state.tasks["tag"].status, TaskStatus::Passed);
}

#[test]
fn a_conflicting_existing_tag_is_a_hard_failure_that_never_moves_it() {
    let (_scratch, world, dir, mut state) = setup("conflicting-tag");
    let executor = executor(&world);
    *world.existing_tag.borrow_mut() = Some(commit('9'));
    // A refused tag leaves nothing runnable: the campaign idles with the task Failed.
    loop {
        let report = executor.continue_campaign(&dir, &mut state, None).unwrap();
        state.frozen.remove(RESTART_KEY);
        if report.ran.is_empty() {
            break;
        }
    }
    assert_ne!(state.mode, CampaignMode::Complete);
    assert_eq!(state.tasks["tag"].status, TaskStatus::Failed);
    assert!(world.tagged.borrow().is_empty());
    assert!(world.pushed.borrow().is_empty());
}
