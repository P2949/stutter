//! Task runners available in this build. Live suites run through a `SuiteBackend`: directly, or
//! through the privileged validation runner when `STUTTER_VALIDATION_RUNNER` names its socket.
//! A kind without a real runner reports an honest block, never a pass.
use std::{collections::BTreeMap, fs, process::Command};

use serde_json::Value;

use super::{
    backend::{BackendError, SuiteBackend, SuiteInvocation, backend_from_env},
    evidence::{
        Accept, PipelineFailure, PublishRequest, Published, publish_bundle, record_failure,
    },
    executor::{RunContext, RunnerOutcome, TaskRunner},
    graph::{SourceBinding, TaskKind, TaskSpec},
    model::{BlockReason, FailureClass},
};

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
    /// Cycles of the registered fixed-compile workload; `Some` makes this the real-workload suite.
    pub work_units: Option<u64>,
}

fn publish_failure_class(stage: &str) -> FailureClass {
    match stage {
        // Storage hiccups are retried; a verified-bundle rejection means the tooling is wrong.
        "fsync" | "publish" | "receipt" => FailureClass::Retryable,
        _ => FailureClass::SoftwareDefect,
    }
}

impl LiveSuiteRunner {
    /// Runs the suite and publishes its verified bundle. `accept` decides which results are
    /// publishable; every other outcome is returned as the task outcome to record.
    pub fn run_and_publish(
        &self,
        context: &RunContext<'_>,
        spec: &TaskSpec,
        attempt: u32,
        accept: Accept,
    ) -> Result<Published, RunnerOutcome> {
        let attempt_dir = context.dir.attempt_dir(spec.id, attempt);
        let raw = attempt_dir.join("raw");
        let artifacts = Some(context.dir.relative(&attempt_dir));
        let log = attempt_dir.join("logs/suite.log");
        let invocation = SuiteInvocation {
            task: spec.id,
            suite: self.suite,
            attempt,
            seconds: context.manifest.overhead.seconds,
            rounds: matches!(self.shape, SuiteShape::Overhead)
                .then_some(context.manifest.overhead.rounds),
            work_units: self.work_units,
            raw: &raw,
            log: &log,
            stutter: &context.binaries.stutter,
            xtask: &context.binaries.xtask,
            cwd: context.repository,
            campaign_id: &context.state.campaign_id,
            manifest_sha256: &context.state.manifest_sha256,
        };
        match self.backend.execute(&invocation) {
            Ok(_) => {}
            // Not authorized, or no privilege: a block, never a retry loop.
            Err(BackendError::Privilege(detail)) => {
                return Err(RunnerOutcome::Blocked(BlockReason {
                    class: FailureClass::BlockedPrivilege,
                    detail,
                }));
            }
            Err(BackendError::Failed(error)) => {
                return Err(RunnerOutcome::Failed {
                    class: FailureClass::Retryable,
                    stage: "launch".to_owned(),
                    summary: format!("could not start the suite: {error}"),
                    artifacts,
                });
            }
        }
        let result: Option<Value> = fs::read(raw.join("result.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let Some(result) = result else {
            return Err(RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "suite_result".to_owned(),
                summary: "the suite wrote no result.json".to_owned(),
                artifacts,
            });
        };
        if result["status"] == "BLOCKED" {
            return Err(RunnerOutcome::Blocked(BlockReason {
                class: FailureClass::BlockedPrivilege,
                detail: result["error"]
                    .as_str()
                    .unwrap_or("suite blocked")
                    .to_owned(),
            }));
        }
        if !accept.allows(&result) {
            return Err(RunnerOutcome::Failed {
                class: FailureClass::Retryable,
                stage: "suite_result".to_owned(),
                summary: format!(
                    "suite reported {}: {}",
                    result["status"].as_str().unwrap_or("no status"),
                    result["error"].as_str().unwrap_or("no error recorded")
                ),
                artifacts,
            });
        }
        let commit = match spec.binding {
            SourceBinding::ReleaseCandidate => {
                context.state.release_candidate.clone().unwrap_or_default()
            }
            _ => context.state.evidence_base.clone(),
        };
        publish_bundle(&PublishRequest {
            dir: context.dir,
            repository: context.repository,
            task: spec.id,
            attempt,
            raw: &raw,
            expected_commit: &commit,
            allowed_kinds: if self.work_units.is_some() {
                &["real_workload"]
            } else {
                &["controlled_real", "real_workload"]
            },
            accept,
        })
        .map_err(|failure| failed_publication(context, spec, attempt, &failure, artifacts))
    }
}

impl TaskRunner for LiveSuiteRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let accept = if self.suite == "score-degradation" {
            Accept::PassOrNonMonotonicScore
        } else {
            Accept::PassOnly
        };
        match self.run_and_publish(context, spec, attempt, accept) {
            Ok(published) => RunnerOutcome::Passed {
                artifacts: Some(context.dir.relative(&published.bundle)),
                receipt: Some(context.dir.relative(&published.receipt)),
                limitations: Vec::new(),
                fingerprint: Box::default(),
                updates: Vec::new(),
            },
            Err(outcome) => outcome,
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

/// A gate that did not pass, with the failure class its exit implies.
pub struct GateFailure {
    pub stage: String,
    pub class: FailureClass,
    pub summary: String,
}

/// Runs the gates in order in `repository`, logging each under `attempt_dir/logs`.
pub fn run_gates(
    gates: &[(String, Vec<String>)],
    repository: &std::path::Path,
    attempt_dir: &std::path::Path,
) -> Result<(), GateFailure> {
    for (name, argv) in gates {
        let log = attempt_dir.join("logs").join(format!("{name}.log"));
        let code = fs::create_dir_all(attempt_dir.join("logs"))
            .and_then(|()| fs::File::create(&log))
            .and_then(|file| {
                Command::new(&argv[0])
                    .args(&argv[1..])
                    .current_dir(repository)
                    .stdout(file.try_clone()?)
                    .stderr(file)
                    .status()
            })
            .map(|status| status.code().unwrap_or(-1));
        match code {
            Ok(0) => {}
            Ok(code) => {
                return Err(GateFailure {
                    stage: name.clone(),
                    class: FailureClass::SoftwareDefect,
                    summary: format!("offline gate {name} exited {code}"),
                });
            }
            Err(error) => {
                return Err(GateFailure {
                    stage: name.clone(),
                    class: FailureClass::Retryable,
                    summary: format!("could not run offline gate {name}: {error}"),
                });
            }
        }
    }
    Ok(())
}

impl TaskRunner for OfflineGateRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let attempt_dir = context.dir.attempt_dir(spec.id, attempt);
        let artifacts = Some(context.dir.relative(&attempt_dir));
        match run_gates(&self.gates, context.repository, &attempt_dir) {
            Ok(()) => RunnerOutcome::Passed {
                artifacts,
                receipt: None,
                limitations: Vec::new(),
                fingerprint: Box::default(),
                updates: Vec::new(),
            },
            Err(failure) => RunnerOutcome::Failed {
                class: failure.class,
                stage: failure.stage,
                summary: failure.summary,
                artifacts,
            },
        }
    }
}

pub fn default_runners() -> BTreeMap<TaskKind, Box<dyn TaskRunner>> {
    let mut runners: BTreeMap<TaskKind, Box<dyn TaskRunner>> = BTreeMap::new();
    let live = |suite, shape| {
        Box::new(LiveSuiteRunner {
            suite,
            shape,
            backend: backend_from_env(),
            work_units: None,
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
    runners.insert(
        TaskKind::KernelMatrix,
        Box::new(super::kernels::KernelMatrixRunner {
            host: Box::new(super::kernels::RunnerHost),
        }),
    );
    runners.insert(
        TaskKind::OverheadReal,
        Box::new(super::overhead_real::OverheadRealRunner::system()),
    );
    runners.insert(
        TaskKind::FrameScore,
        Box::new(super::frame_score::FrameScoreRunner {
            make_host: Box::new(super::host_factory::level_host),
        }),
    );
    runners.insert(
        TaskKind::DiagnosisCalibration,
        Box::new(super::calibration_task::DiagnosisCalibrationRunner::system()),
    );
    runners.insert(
        TaskKind::ScoreReview,
        Box::new(super::calibration_task::ScoreReviewRunner),
    );
    runners.insert(
        TaskKind::EvidenceIntegration,
        Box::new(super::integration::IntegrationRunner {
            tools: super::pr_task::PrTools::system(),
            entries: Box::new(super::integration::verified_entries),
        }),
    );
    runners.insert(
        TaskKind::CapabilityPolicy,
        Box::new(super::policy::CapabilityPolicyRunner {
            tools: super::pr_task::PrTools::system(),
        }),
    );
    runners.insert(
        TaskKind::ReleaseDocs,
        Box::new(super::policy::ReleaseDocsRunner {
            tools: super::pr_task::PrTools::system(),
        }),
    );
    runners.insert(
        TaskKind::Corpus,
        Box::new(super::corpus::CorpusRunner {
            factory: Box::new(super::host_factory::SystemHostFactory),
        }),
    );
    runners.insert(
        TaskKind::FreezeRc,
        Box::new(super::release::freeze::FreezeRcRunner {
            shell: Box::new(super::shell::SystemShell),
            repin: Box::new(super::release::freeze::repin_runner),
        }),
    );
    runners.insert(
        TaskKind::RcOffline,
        Box::new(super::release::offline::RcOfflineRunner {
            shell: Box::new(super::shell::SystemShell),
            wait: super::repo_pr::WaitPolicy::default(),
        }),
    );
    runners.insert(
        TaskKind::RcLiveObservation,
        live("observation", SuiteShape::Seconds),
    );
    runners.insert(
        TaskKind::RcGameRunnable,
        live("game-runnable", SuiteShape::Seconds),
    );
    runners.insert(TaskKind::RcAffinity, live("affinity", SuiteShape::Seconds));
    runners.insert(TaskKind::RcRecovery, live("recovery", SuiteShape::Seconds));
    runners.insert(
        TaskKind::ReleasePackage,
        Box::new(super::release::package::ReleasePackageRunner {
            shell: Box::new(super::shell::SystemShell),
        }),
    );
    runners.insert(
        TaskKind::ReleaseVerify,
        Box::new(super::release::package::ReleaseVerifyRunner {
            shell: Box::new(super::shell::SystemShell),
        }),
    );
    runners.insert(
        TaskKind::InstallSmoke,
        Box::new(super::release::package::InstallSmokeRunner {
            shell: Box::new(super::shell::SystemShell),
        }),
    );
    runners.insert(
        TaskKind::Tag,
        Box::new(super::release::tag::TagRunner {
            shell: Box::new(super::shell::SystemShell),
            wait: super::repo_pr::WaitPolicy::default(),
        }),
    );
    runners
}
