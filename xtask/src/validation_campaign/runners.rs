//! Task runners available in this build. Live suites run through a `SuiteBackend` (direct
//! subprocess here; the privileged runner arrives in a later phase). A kind without a real
//! runner reports an honest block, never a pass.
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Stdio},
};

use serde_json::Value;

use super::{
    evidence::{PipelineFailure, publish_bundle, record_failure},
    executor::{FingerprintExtras, RunContext, RunnerOutcome, TaskRunner},
    graph::{SourceBinding, TaskKind, TaskSpec},
    model::{BlockReason, FailureClass},
};

/// Executes an `xtask live-acceptance ...` invocation and returns its exit code.
pub trait SuiteBackend {
    fn execute(
        &self,
        program: &Path,
        args: &[String],
        cwd: &Path,
        log: &Path,
    ) -> std::io::Result<i32>;
}

pub struct DirectBackend;

impl SuiteBackend for DirectBackend {
    fn execute(
        &self,
        program: &Path,
        args: &[String],
        cwd: &Path,
        log: &Path,
    ) -> std::io::Result<i32> {
        if let Some(parent) = log.parent() {
            fs::create_dir_all(parent)?;
        }
        let stdout = fs::File::create(log)?;
        let status = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdout(stdout.try_clone()?)
            .stderr(stdout)
            .stdin(Stdio::null())
            .status()?;
        Ok(status.code().unwrap_or(-1))
    }
}

/// How a live suite is parameterized.
#[derive(Clone, Copy)]
pub enum SuiteShape {
    Seconds,
    Overhead,
}

pub struct LiveSuiteRunner {
    pub suite: &'static str,
    pub shape: SuiteShape,
    pub backend: Box<dyn SuiteBackend>,
}

fn publish_failure_class(stage: &str) -> FailureClass {
    match stage {
        // Storage hiccups are retried; a verified-bundle rejection means the tooling is wrong.
        "fsync" | "publish" | "receipt" => FailureClass::Retryable,
        _ => FailureClass::SoftwareDefect,
    }
}

impl TaskRunner for LiveSuiteRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let attempt_dir = context.dir.attempt_dir(spec.id, attempt);
        let raw = attempt_dir.join("raw");
        let mut args = vec![
            "live-acceptance".to_owned(),
            self.suite.to_owned(),
            "--stutter-bin".to_owned(),
            context.binaries.stutter.to_string_lossy().into_owned(),
            "--out".to_owned(),
            raw.to_string_lossy().into_owned(),
            "--seconds".to_owned(),
            context.manifest.overhead.seconds.to_string(),
        ];
        if matches!(self.shape, SuiteShape::Overhead) {
            args.extend([
                "--rounds".to_owned(),
                context.manifest.overhead.rounds.to_string(),
            ]);
        }
        let artifacts = Some(context.dir.relative(&attempt_dir));
        let log = attempt_dir.join("logs/suite.log");
        if let Err(error) =
            self.backend
                .execute(&context.binaries.xtask, &args, context.repository, &log)
        {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "launch".to_owned(),
                summary: format!("could not start the suite: {error}"),
                artifacts,
            };
        }
        let result: Option<Value> = fs::read(raw.join("result.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let Some(result) = result else {
            return RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "suite_result".to_owned(),
                summary: "the suite wrote no result.json".to_owned(),
                artifacts,
            };
        };
        match result["status"].as_str() {
            Some("BLOCKED") => {
                return RunnerOutcome::Blocked(BlockReason {
                    class: FailureClass::BlockedPrivilege,
                    detail: result["error"]
                        .as_str()
                        .unwrap_or("suite blocked")
                        .to_owned(),
                });
            }
            Some("PASS") => {}
            other => {
                return RunnerOutcome::Failed {
                    class: FailureClass::Retryable,
                    stage: "suite_result".to_owned(),
                    summary: format!(
                        "suite reported {}: {}",
                        other.unwrap_or("no status"),
                        result["error"].as_str().unwrap_or("no error recorded")
                    ),
                    artifacts,
                };
            }
        }
        let commit = match spec.binding {
            SourceBinding::ReleaseCandidate => {
                context.state.release_candidate.clone().unwrap_or_default()
            }
            _ => context.state.evidence_base.clone(),
        };
        match publish_bundle(
            context.dir,
            context.repository,
            spec.id,
            attempt,
            &raw,
            &commit,
            &["controlled_real", "real_workload"],
        ) {
            Ok(published) => RunnerOutcome::Passed {
                artifacts: Some(context.dir.relative(&published.bundle)),
                receipt: Some(context.dir.relative(&published.receipt)),
                limitations: Vec::new(),
                fingerprint: FingerprintExtras::default(),
                updates: Vec::new(),
            },
            Err(failure) => failed_publication(context, spec, attempt, &failure, artifacts),
        }
    }
}

fn failed_publication(
    context: &RunContext<'_>,
    spec: &TaskSpec,
    attempt: u32,
    failure: &PipelineFailure,
    artifacts: Option<String>,
) -> RunnerOutcome {
    // The failure record is best-effort metadata; the attempt itself is already retained.
    let _ = record_failure(context.dir, spec.id, attempt, failure);
    RunnerOutcome::Failed {
        class: publish_failure_class(&failure.stage),
        stage: failure.stage.clone(),
        summary: failure.summary.clone(),
        artifacts,
    }
}

/// The offline gates every campaign starts from (the implementation acceptance gate).
pub const OFFLINE_GATES: &[(&str, &[&str])] = &[
    ("fmt", &["cargo", "fmt", "--all", "--check"]),
    (
        "no-allow-attrs",
        &["cargo", "run", "-q", "-p", "xtask", "--", "no-allow-attrs"],
    ),
    (
        "dependency-hygiene",
        &[
            "cargo",
            "run",
            "-q",
            "-p",
            "xtask",
            "--",
            "dependency-hygiene",
        ],
    ),
    ("build-gate", &["scripts/smoke/build.sh"]),
    (
        "objective-docs",
        &[
            "cargo",
            "run",
            "-q",
            "-p",
            "xtask",
            "--",
            "objective-docs",
            "--check",
        ],
    ),
    (
        "fixture-check",
        &["cargo", "run", "-q", "-p", "xtask", "--", "fixture-check"],
    ),
    (
        "fixture-coverage",
        &[
            "cargo",
            "run",
            "-q",
            "-p",
            "xtask",
            "--",
            "fixture-coverage",
        ],
    ),
    (
        "capability-check",
        &[
            "cargo",
            "run",
            "-q",
            "-p",
            "xtask",
            "--",
            "capability-check",
        ],
    ),
    ("autotune-safety", &["scripts/check-autotune-safety.sh"]),
    (
        "offline-recommendation",
        &["scripts/smoke/offline_recommendation.sh"],
    ),
    ("advisor-offline", &["scripts/smoke/advisor_offline.sh"]),
    ("xtask-tests", &["cargo", "test", "-p", "xtask"]),
    ("diff-check", &["git", "diff", "--check"]),
];

pub struct OfflineGateRunner {
    pub gates: Vec<(String, Vec<String>)>,
}

impl OfflineGateRunner {
    pub fn standard() -> Self {
        Self {
            gates: OFFLINE_GATES
                .iter()
                .map(|(name, argv)| {
                    (
                        (*name).to_owned(),
                        argv.iter().map(|a| (*a).to_owned()).collect(),
                    )
                })
                .collect(),
        }
    }
}

impl TaskRunner for OfflineGateRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let attempt_dir = context.dir.attempt_dir(spec.id, attempt);
        let artifacts = Some(context.dir.relative(&attempt_dir));
        for (name, argv) in &self.gates {
            let log = attempt_dir.join("logs").join(format!("{name}.log"));
            let code = fs::create_dir_all(attempt_dir.join("logs"))
                .and_then(|()| fs::File::create(&log))
                .and_then(|file| {
                    Command::new(&argv[0])
                        .args(&argv[1..])
                        .current_dir(context.repository)
                        .stdout(file.try_clone()?)
                        .stderr(file)
                        .status()
                })
                .map(|status| status.code().unwrap_or(-1));
            match code {
                Ok(0) => {}
                Ok(code) => {
                    return RunnerOutcome::Failed {
                        class: FailureClass::SoftwareDefect,
                        stage: name.clone(),
                        summary: format!("offline gate {name} exited {code}"),
                        artifacts,
                    };
                }
                Err(error) => {
                    return RunnerOutcome::Failed {
                        class: FailureClass::Retryable,
                        stage: name.clone(),
                        summary: format!("could not run offline gate {name}: {error}"),
                        artifacts,
                    };
                }
            }
        }
        RunnerOutcome::Passed {
            artifacts,
            receipt: None,
            limitations: Vec::new(),
            fingerprint: FingerprintExtras::default(),
            updates: Vec::new(),
        }
    }
}

/// Placeholder for kinds whose real runner lands in a later phase; an honest block.
pub struct NotYetAvailable(pub &'static str);

impl TaskRunner for NotYetAvailable {
    fn run(&self, _context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        RunnerOutcome::Blocked(BlockReason {
            class: FailureClass::BlockedResource,
            detail: format!("the {} runner is not implemented in this build", self.0),
        })
    }
}

pub fn default_runners() -> BTreeMap<TaskKind, Box<dyn TaskRunner>> {
    let mut runners: BTreeMap<TaskKind, Box<dyn TaskRunner>> = BTreeMap::new();
    let live = |suite, shape| {
        Box::new(LiveSuiteRunner {
            suite,
            shape,
            backend: Box::new(DirectBackend),
        }) as Box<dyn TaskRunner>
    };
    runners.insert(
        TaskKind::OfflineReady,
        Box::new(OfflineGateRunner::standard()),
    );
    runners.insert(
        TaskKind::LiveObservation,
        live("observation", SuiteShape::Seconds),
    );
    runners.insert(
        TaskKind::GameRunnable,
        live("game-runnable", SuiteShape::Seconds),
    );
    runners.insert(
        TaskKind::OverheadControlled,
        live("overhead", SuiteShape::Overhead),
    );
    runners.insert(
        TaskKind::ScoreRunnable,
        live("score-degradation", SuiteShape::Seconds),
    );
    for (kind, name) in [
        (TaskKind::OverheadReal, "real-workload overhead"),
        (TaskKind::KernelMatrix, "kernel campaign"),
        (TaskKind::Corpus, "corpus"),
        (TaskKind::FrameScore, "frame score series"),
        (TaskKind::EvidenceIntegration, "evidence integration"),
        (TaskKind::DiagnosisCalibration, "calibration"),
        (TaskKind::ScoreReview, "score review"),
        (TaskKind::CapabilityPolicy, "capability policy"),
        (TaskKind::ReleaseDocs, "release docs"),
        (TaskKind::FreezeRc, "release-candidate freeze"),
        (TaskKind::RcOffline, "exact-RC offline acceptance"),
        (TaskKind::RcLiveObservation, "exact-RC observation"),
        (TaskKind::RcGameRunnable, "exact-RC game-runnable"),
        (TaskKind::RcAffinity, "exact-RC affinity"),
        (TaskKind::RcRecovery, "exact-RC recovery"),
        (TaskKind::ReleasePackage, "release packaging"),
        (TaskKind::ReleaseVerify, "release verification"),
        (TaskKind::InstallSmoke, "installation smoke"),
        (TaskKind::Tag, "tag"),
    ] {
        runners.insert(kind, Box::new(NotYetAvailable(name)));
    }
    runners
}
