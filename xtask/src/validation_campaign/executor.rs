//! Runs everything immediately runnable, one task at a time, until a genuine boundary.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use super::{
    graph::{Graph, SourceBinding, TaskKind, TaskSpec},
    manifest::CampaignManifest,
    model::{
        AttemptOutcome, BlockReason, Boundary, CampaignDecision, CampaignMode, CampaignState,
        FailureClass, TaskAttempt, TaskFingerprint, TaskStatus,
    },
    repository::{Binaries, check_source},
    state::{CampaignDir, save, unix_ms},
};

/// Frozen-state flag a task sets after rebuilding the binaries; `continue` then re-executes
/// itself from the fresh build instead of carrying on with the old code.
pub const RESTART_KEY: &str = "controller.restart";

/// Context handed to a runner; deliberately read-only.
pub struct RunContext<'a> {
    pub dir: &'a CampaignDir,
    pub state: &'a CampaignState,
    pub manifest: &'a CampaignManifest,
    pub repository: &'a Path,
    pub binaries: &'a Binaries,
    /// State updates a runner wants applied whatever its outcome (e.g. kernel stage progress).
    pub updates: &'a RefCell<Vec<StateUpdate>>,
}

impl RunContext<'_> {
    pub fn record(&self, update: StateUpdate) {
        self.updates.borrow_mut().push(update);
    }
}

/// Fingerprint dimensions only the runner knows.
#[derive(Clone, Debug, Default)]
pub struct FingerprintExtras {
    pub workload_sha256: Option<String>,
    pub scenario_sha256: Option<String>,
    pub kernel_release: Option<String>,
}

pub enum StateUpdate {
    SetReleaseCandidate(String),
    /// Digests of the binaries built at the release candidate.
    SetReleaseBinaries {
        stutter_sha256: String,
        xtask_sha256: String,
    },
    Freeze {
        key: String,
        value: String,
    },
    Decision {
        decision: String,
        limitations: Vec<String>,
    },
}

/// The reboot request, executed by the executor once the waiting state has been saved.
pub type AfterPersist = Box<dyn FnOnce() -> Result<(), String>>;

pub enum RunnerOutcome {
    Passed {
        artifacts: Option<String>,
        receipt: Option<String>,
        limitations: Vec<String>,
        fingerprint: Box<FingerprintExtras>,
        updates: Vec<StateUpdate>,
    },
    Failed {
        class: FailureClass,
        stage: String,
        summary: String,
        artifacts: Option<String>,
    },
    Blocked(BlockReason),
    /// The task asked for a reboot. `after_persist` performs the reboot request and runs only
    /// after the waiting state is durable, so the controller can be killed immediately after.
    WaitingForReboot {
        after_persist: Option<AfterPersist>,
    },
}

impl RunnerOutcome {
    pub fn waiting_for_reboot(after_persist: AfterPersist) -> Self {
        Self::WaitingForReboot {
            after_persist: Some(after_persist),
        }
    }

    pub fn passed() -> Self {
        Self::Passed {
            artifacts: None,
            receipt: None,
            limitations: Vec::new(),
            fingerprint: Box::default(),
            updates: Vec::new(),
        }
    }
}

pub trait TaskRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome;
}

type SourceCheck = dyn Fn(&CampaignState, &TaskSpec) -> Result<(), String>;

pub struct Executor {
    pub graph: Graph,
    pub manifest: CampaignManifest,
    pub runners: BTreeMap<TaskKind, Box<dyn TaskRunner>>,
    pub repository: PathBuf,
    pub binaries: Binaries,
    pub source_check: Box<SourceCheck>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskFlow {
    Done,
    Rebooting,
}

#[derive(Debug)]
pub struct ContinueReport {
    pub ran: Vec<String>,
    pub boundary: Boundary,
}

impl Executor {
    /// Production source check: the full integrity rules against the real repository.
    pub fn with_repository_check(mut self, manifest_path: PathBuf) -> Self {
        let binaries = self.binaries.clone();
        self.source_check =
            Box::new(move |state, spec| check_source(state, spec, &manifest_path, &binaries));
        self
    }

    pub fn continue_campaign(
        &self,
        dir: &CampaignDir,
        state: &mut CampaignState,
        max_tasks: Option<usize>,
    ) -> anyhow::Result<ContinueReport> {
        self.recover_interrupted(dir, state)?;
        let mut ran = Vec::new();
        let mut attempted = BTreeSet::new();
        loop {
            if max_tasks.is_some_and(|max| ran.len() >= max) {
                return Ok(ContinueReport {
                    ran,
                    boundary: Boundary::Idle,
                });
            }
            if state.mode == CampaignMode::Invalidated {
                break;
            }
            let next = state
                .runnable(&self.graph, true)
                .into_iter()
                .find(|id| !attempted.contains(id));
            let Some(id) = next else { break };
            attempted.insert(id);
            let Some(spec) = self.graph.get(id) else {
                continue;
            };
            // Only the task about to run is checked: once collection is done, `main` moving
            // (evidence integration, policy PRs) is expected and must not invalidate anything.
            if spec.binding != SourceBinding::None
                && let Err(reason) = (self.source_check)(state, spec)
            {
                state.invalidate(&self.graph, spec.binding);
                state.decisions.push(CampaignDecision {
                    task: id.to_owned(),
                    decision: format!("campaign invalidated: {reason}"),
                    limitations: Vec::new(),
                    unix_ms: unix_ms(),
                });
                save(dir, state)?;
                return Ok(ContinueReport {
                    ran,
                    boundary: Boundary::FailureRequiresSourceChange,
                });
            }
            let flow = self.run_task(dir, state, spec)?;
            ran.push(id.to_owned());
            if state.frozen.get(RESTART_KEY).is_some_and(|v| v == "1") {
                // The binaries were rebuilt: the remaining tasks must run in the new process.
                return Ok(ContinueReport {
                    ran,
                    boundary: Boundary::Idle,
                });
            }
            if flow == TaskFlow::Rebooting {
                // The machine is about to go down: start nothing else.
                return Ok(ContinueReport {
                    ran,
                    boundary: Boundary::WaitingForReboot,
                });
            }
        }
        Ok(ContinueReport {
            ran,
            boundary: self.boundary(state),
        })
    }

    /// A task still `Running` at startup was cut off (crash, kill, power loss). Its attempt is
    /// recorded as interrupted and it becomes runnable again; nothing is assumed about how far
    /// it got.
    fn recover_interrupted(
        &self,
        dir: &CampaignDir,
        state: &mut CampaignState,
    ) -> anyhow::Result<()> {
        let interrupted = state
            .tasks
            .values()
            .filter(|task| task.status == TaskStatus::Running)
            .map(|task| task.id.clone())
            .collect::<Vec<_>>();
        for id in interrupted {
            let Some(spec) = self.graph.get(&id) else {
                continue;
            };
            let attempt = TaskAttempt {
                number: state.tasks[&id].attempts.len() as u32 + 1,
                started_unix_ms: unix_ms(),
                finished_unix_ms: unix_ms(),
                outcome: AttemptOutcome::Failed {
                    class: FailureClass::Retryable,
                    stage: "interrupted".to_owned(),
                    summary: "the controller stopped while this task was running".to_owned(),
                },
                fingerprint: state.expected_fingerprint(spec),
                artifacts: None,
                receipt: None,
                limitations: Vec::new(),
            };
            state.record_attempt(dir, &id, attempt)?;
            if let Some(task) = state.tasks.get_mut(&id) {
                task.status = TaskStatus::Pending;
            }
        }
        save(dir, state)
    }

    /// Runs one task with its bounded retries, recording every attempt.
    fn run_task(
        &self,
        dir: &CampaignDir,
        state: &mut CampaignState,
        spec: &TaskSpec,
    ) -> anyhow::Result<TaskFlow> {
        let budget = self.manifest.retry.default_attempts as usize;
        loop {
            let number = state.tasks[spec.id].attempts.len() as u32 + 1;
            if let Some(task) = state.tasks.get_mut(spec.id) {
                task.status = TaskStatus::Running;
            }
            save(dir, state)?;
            let started = unix_ms();
            let collected = RefCell::new(Vec::new());
            let outcome = match self.runners.get(&spec.kind) {
                Some(runner) => runner.run(
                    &RunContext {
                        dir,
                        state,
                        manifest: &self.manifest,
                        repository: &self.repository,
                        binaries: &self.binaries,
                        updates: &collected,
                    },
                    spec,
                    number,
                ),
                None => RunnerOutcome::Blocked(BlockReason {
                    class: FailureClass::BlockedResource,
                    detail: format!("no runner is registered for {:?}", spec.kind),
                }),
            };
            let mut fingerprint = state.expected_fingerprint(spec);
            let mut after_persist = None;
            let (attempt_outcome, artifacts, receipt, limitations, updates) = match outcome {
                RunnerOutcome::Passed {
                    artifacts,
                    receipt,
                    limitations,
                    fingerprint: extras,
                    updates,
                } => {
                    merge_extras(&mut fingerprint, *extras);
                    (
                        AttemptOutcome::Passed,
                        artifacts,
                        receipt,
                        limitations,
                        updates,
                    )
                }
                RunnerOutcome::Failed {
                    class,
                    stage,
                    summary,
                    artifacts,
                } => (
                    AttemptOutcome::Failed {
                        class,
                        stage,
                        summary,
                    },
                    artifacts,
                    None,
                    Vec::new(),
                    Vec::new(),
                ),
                RunnerOutcome::Blocked(reason) => (
                    AttemptOutcome::Blocked(reason),
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                ),
                RunnerOutcome::WaitingForReboot {
                    after_persist: action,
                } => {
                    after_persist = action;
                    (
                        AttemptOutcome::WaitingForReboot,
                        None,
                        None,
                        Vec::new(),
                        Vec::new(),
                    )
                }
            };
            let retryable = matches!(
                &attempt_outcome,
                AttemptOutcome::Failed {
                    class: FailureClass::Retryable,
                    ..
                }
            );
            let attempt = TaskAttempt {
                number,
                started_unix_ms: started,
                finished_unix_ms: unix_ms(),
                outcome: attempt_outcome,
                fingerprint,
                artifacts,
                receipt,
                limitations,
            };
            let summary = match &attempt.outcome {
                AttemptOutcome::Failed { summary, .. } => summary.clone(),
                _ => String::new(),
            };
            let passed = attempt.outcome == AttemptOutcome::Passed;
            state.record_attempt(dir, spec.id, attempt)?;
            // Progress the runner recorded counts whatever the outcome; the outcome's own
            // updates only on a pass.
            self.apply_updates(state, spec, collected.take());
            if passed {
                self.apply_updates(state, spec, updates);
            }
            let failures = state.tasks[spec.id].failed_attempts();
            if retryable && failures >= budget {
                // Budget exhausted: an honest block with retained attempts, retried on the
                // next `continue`, never a silent skip.
                if let Some(task) = state.tasks.get_mut(spec.id) {
                    let reason = BlockReason {
                        class: FailureClass::BlockedResource,
                        detail: format!(
                            "retry budget exhausted after {failures} attempts: {summary}"
                        ),
                    };
                    task.status = TaskStatus::BlockedResource;
                    task.block = Some(reason);
                }
            }
            save(dir, state)?;
            if let Some(request_reboot) = after_persist {
                // The waiting state is durable; only now is the reboot requested.
                match request_reboot() {
                    Ok(()) => return Ok(TaskFlow::Rebooting),
                    Err(error) => {
                        let blocked = TaskAttempt {
                            number: state.tasks[spec.id].attempts.len() as u32 + 1,
                            started_unix_ms: unix_ms(),
                            finished_unix_ms: unix_ms(),
                            outcome: AttemptOutcome::Blocked(BlockReason {
                                class: FailureClass::BlockedResource,
                                detail: format!("the reboot request failed: {error}"),
                            }),
                            fingerprint: state.expected_fingerprint(spec),
                            artifacts: None,
                            receipt: None,
                            limitations: Vec::new(),
                        };
                        state.record_attempt(dir, spec.id, blocked)?;
                        save(dir, state)?;
                        return Ok(TaskFlow::Done);
                    }
                }
            }
            if !(retryable && failures < budget) {
                return Ok(TaskFlow::Done);
            }
        }
    }

    fn apply_updates(&self, state: &mut CampaignState, spec: &TaskSpec, updates: Vec<StateUpdate>) {
        for update in updates {
            match update {
                StateUpdate::SetReleaseCandidate(commit) => {
                    state.release_candidate = Some(commit);
                    state.mode = CampaignMode::ReleaseCandidate;
                }
                StateUpdate::SetReleaseBinaries {
                    stutter_sha256,
                    xtask_sha256,
                } => {
                    state.rc_stutter_sha256 = Some(stutter_sha256);
                    state.rc_xtask_sha256 = Some(xtask_sha256);
                }
                StateUpdate::Freeze { key, value } => {
                    state.frozen.insert(key, value);
                }
                StateUpdate::Decision {
                    decision,
                    limitations,
                } => {
                    state.decisions.push(CampaignDecision {
                        task: spec.id.to_owned(),
                        decision,
                        limitations,
                        unix_ms: unix_ms(),
                    });
                }
            }
        }
        if spec.kind == TaskKind::Tag {
            state.mode = CampaignMode::Complete;
        }
    }

    pub fn boundary(&self, state: &CampaignState) -> Boundary {
        if state.mode == CampaignMode::Invalidated {
            return Boundary::FailureRequiresSourceChange;
        }
        let required = self.graph.specs().filter(|spec| {
            state.tasks.get(spec.id).map(|t| t.status) != Some(TaskStatus::SkippedByPolicy)
        });
        if required
            .into_iter()
            .all(|spec| state.effective_status(spec).satisfies_dependents())
        {
            return Boundary::Complete;
        }
        let defect = state.tasks.values().any(|task| {
            task.status == TaskStatus::Failed
                && matches!(
                    task.attempts.last().map(|a| &a.outcome),
                    Some(AttemptOutcome::Failed {
                        class: FailureClass::SoftwareDefect,
                        ..
                    })
                )
        });
        let has = |status| state.tasks.values().any(|task| task.status == status);
        if defect {
            Boundary::FailureRequiresSourceChange
        } else if has(TaskStatus::WaitingForReboot) {
            Boundary::WaitingForReboot
        } else if has(TaskStatus::BlockedPrivilege) {
            Boundary::BlockedPrivilege
        } else if has(TaskStatus::BlockedResource) {
            Boundary::BlockedResource
        } else {
            Boundary::Idle
        }
    }
}

fn merge_extras(fingerprint: &mut TaskFingerprint, extras: FingerprintExtras) {
    fingerprint.workload_sha256 = extras.workload_sha256;
    fingerprint.scenario_sha256 = extras.scenario_sha256;
    fingerprint.kernel_release = extras.kernel_release;
}
