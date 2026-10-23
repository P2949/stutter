//! Persistent campaign state. Every write is temp file + `sync_all` + rename + parent
//! directory sync; attempts are append-only and also written as immutable per-attempt files.
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail, ensure};

use super::{
    graph::{Graph, SourceBinding, TaskSpec},
    manifest::CampaignManifest,
    model::{
        AttemptOutcome, CampaignMode, CampaignState, SCHEMA_VERSION, TaskAttempt, TaskFingerprint,
        TaskState, TaskStatus,
    },
};

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Durable replacement of `path`: no reader ever sees a partial file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent", path.display()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("state"),
        std::process::id()
    ));
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Layout of one campaign instance under `.llm-runs/validation/<campaign-id>/`.
#[derive(Clone, Debug)]
pub struct CampaignDir(pub PathBuf);

impl CampaignDir {
    pub fn under(repository: &Path, campaign_id: &str) -> Self {
        Self(repository.join(".llm-runs/validation").join(campaign_id))
    }
    pub fn state_path(&self) -> PathBuf {
        self.0.join("campaign.json")
    }
    pub fn manifest_snapshot(&self) -> PathBuf {
        self.0.join("manifest.snapshot.toml")
    }
    pub fn attempt_dir(&self, task: &str, number: u32) -> PathBuf {
        self.0
            .join("attempts")
            .join(task)
            .join(format!("{number:04}"))
    }
    pub fn bundles(&self) -> PathBuf {
        self.0.join("bundles")
    }
    pub fn receipts(&self) -> PathBuf {
        self.0.join("receipts")
    }
    pub fn indexes(&self) -> PathBuf {
        self.0.join("indexes")
    }
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.0)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }
}

pub fn save(dir: &CampaignDir, state: &CampaignState) -> anyhow::Result<()> {
    write_atomic(&dir.state_path(), &serde_json::to_vec_pretty(state)?)
}

pub fn load(dir: &CampaignDir) -> anyhow::Result<CampaignState> {
    let path = dir.state_path();
    let state: CampaignState = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )
    .with_context(|| format!("campaign state {} is corrupt", path.display()))?;
    ensure!(
        state.schema_version == SCHEMA_VERSION,
        "unsupported campaign schema {}",
        state.schema_version
    );
    Ok(state)
}

/// What a fresh campaign is created from; captured by `init` after source verification.
pub struct InitInputs {
    pub campaign_id: String,
    pub repository: PathBuf,
    pub evidence_base: String,
    pub manifest_sha256: String,
    pub cargo_lock_sha256: String,
    pub xtask_sha256: String,
    pub stutter_sha256: String,
    pub uid: u32,
    pub gid: u32,
}

impl CampaignState {
    pub fn init(inputs: InitInputs, graph: &Graph, manifest: &CampaignManifest) -> Self {
        let tasks = graph
            .specs()
            .map(|spec| {
                let mut state = TaskState::new(spec.id);
                if manifest
                    .policy
                    .skip_tasks
                    .iter()
                    .any(|skip| skip == spec.id)
                {
                    state.status = TaskStatus::SkippedByPolicy;
                }
                (spec.id.to_owned(), state)
            })
            .collect();
        Self {
            schema_version: SCHEMA_VERSION,
            campaign_id: inputs.campaign_id,
            generation: 1,
            mode: CampaignMode::Collector,
            repository: inputs.repository,
            evidence_base: inputs.evidence_base,
            manifest_sha256: inputs.manifest_sha256,
            cargo_lock_sha256: inputs.cargo_lock_sha256,
            xtask_sha256: inputs.xtask_sha256,
            stutter_sha256: inputs.stutter_sha256,
            uid: inputs.uid,
            gid: inputs.gid,
            created_unix_ms: unix_ms(),
            tasks,
            decisions: Vec::new(),
            frozen: Default::default(),
            release_candidate: None,
            rc_xtask_sha256: None,
            rc_stutter_sha256: None,
        }
    }

    /// The fingerprint a result for `spec` must carry to count as current.
    pub fn expected_fingerprint(&self, spec: &TaskSpec) -> TaskFingerprint {
        let (commit, digests) = match spec.binding {
            SourceBinding::None => (self.evidence_base.clone(), None),
            SourceBinding::EvidenceBase => (
                self.evidence_base.clone(),
                Some((self.stutter_sha256.clone(), self.xtask_sha256.clone())),
            ),
            SourceBinding::ReleaseCandidate => (
                self.release_candidate.clone().unwrap_or_default(),
                Some((
                    self.rc_stutter_sha256.clone().unwrap_or_default(),
                    self.rc_xtask_sha256.clone().unwrap_or_default(),
                )),
            ),
        };
        TaskFingerprint {
            task: spec.id.to_owned(),
            evidence_commit: commit,
            manifest_sha256: self.manifest_sha256.clone(),
            stutter_sha256: digests.as_ref().map(|(stutter, _)| stutter.clone()),
            xtask_sha256: digests.map(|(_, xtask)| xtask),
            ..TaskFingerprint::default()
        }
    }

    /// A stored `Passed` only counts while its fingerprint matches the current one. Optional
    /// fingerprint dimensions (workload, scenario, kernel) are carried by the result and are
    /// compared field by field only where the current expectation names them.
    pub fn effective_status(&self, spec: &TaskSpec) -> TaskStatus {
        let Some(task) = self.tasks.get(spec.id) else {
            return TaskStatus::Pending;
        };
        if task.status != TaskStatus::Passed {
            return task.status;
        }
        let expected = self.expected_fingerprint(spec);
        match &task.fingerprint {
            Some(actual) if fingerprint_current(actual, &expected) => TaskStatus::Passed,
            _ => TaskStatus::Invalidated,
        }
    }

    pub fn dependencies_satisfied(&self, graph: &Graph, spec: &TaskSpec) -> bool {
        spec.depends_on.iter().all(|dependency| {
            graph
                .get(dependency)
                .is_some_and(|dep| self.effective_status(dep).satisfies_dependents())
        })
    }

    /// Tasks that can run now, in deterministic topological order.
    pub fn ready(&self, graph: &Graph) -> Vec<&'static str> {
        self.runnable(graph, false)
    }

    /// Like `ready`, optionally also re-trying tasks that stopped on a block or a reboot wait
    /// (a block may have been resolved by the machine since).
    pub fn runnable(&self, graph: &Graph, include_blocked: bool) -> Vec<&'static str> {
        if matches!(
            self.mode,
            CampaignMode::Invalidated | CampaignMode::Complete
        ) {
            return Vec::new();
        }
        graph
            .topological()
            .into_iter()
            .filter(|id| {
                let Some(spec) = graph.get(id) else {
                    return false;
                };
                let status = self.effective_status(spec);
                (matches!(
                    status,
                    TaskStatus::Pending | TaskStatus::Ready | TaskStatus::Invalidated
                ) || (include_blocked
                    && matches!(
                        status,
                        TaskStatus::BlockedResource
                            | TaskStatus::BlockedPrivilege
                            | TaskStatus::WaitingForReboot
                    )))
                    && self.dependencies_satisfied(graph, spec)
            })
            .collect()
    }

    /// Appends an attempt and moves the task to the status it implies.
    pub fn record_attempt(
        &mut self,
        dir: &CampaignDir,
        task: &str,
        attempt: TaskAttempt,
    ) -> anyhow::Result<()> {
        let entry = self
            .tasks
            .get_mut(task)
            .with_context(|| format!("unknown task {task}"))?;
        ensure!(
            attempt.number as usize == entry.attempts.len() + 1,
            "attempt numbers are sequential; expected {}",
            entry.attempts.len() + 1
        );
        // Immutable per-attempt record first, so campaign.json can always be rebuilt from it.
        write_atomic(
            &dir.attempt_dir(task, attempt.number).join("attempt.json"),
            &serde_json::to_vec_pretty(&attempt)?,
        )?;
        match &attempt.outcome {
            AttemptOutcome::Passed => {
                entry.status = TaskStatus::Passed;
                entry.fingerprint = Some(attempt.fingerprint.clone());
                entry.block = None;
            }
            AttemptOutcome::Failed { .. } => entry.status = TaskStatus::Failed,
            AttemptOutcome::Blocked(reason) => {
                entry.status = match reason.class {
                    super::model::FailureClass::BlockedPrivilege => TaskStatus::BlockedPrivilege,
                    _ => TaskStatus::BlockedResource,
                };
                entry.block = Some(reason.clone());
            }
            AttemptOutcome::WaitingForReboot => entry.status = TaskStatus::WaitingForReboot,
        }
        entry.attempts.push(attempt);
        Ok(())
    }

    /// Source changed: the campaign stops. Tasks bound to the failed commit and everything that
    /// depends on them become `Invalidated`; their retained attempts stay readable as history.
    /// A moved release candidate invalidates only the exact-RC work, never the collection.
    pub fn invalidate(&mut self, graph: &Graph, binding: SourceBinding) {
        self.mode = CampaignMode::Invalidated;
        let bound = graph
            .specs()
            .filter(|spec| spec.binding == binding)
            .map(|spec| spec.id)
            .collect::<Vec<_>>();
        for id in bound {
            let affected = std::iter::once(id).chain(graph.dependents(id));
            for affected_id in affected {
                if let Some(task) = self.tasks.get_mut(affected_id)
                    && matches!(
                        task.status,
                        TaskStatus::Passed
                            | TaskStatus::Ready
                            | TaskStatus::Running
                            | TaskStatus::Pending
                    )
                {
                    task.status = TaskStatus::Invalidated;
                }
            }
        }
    }

    pub fn require_mode(&self, allowed: &[CampaignMode]) -> anyhow::Result<()> {
        if !allowed.contains(&self.mode) {
            bail!(
                "campaign is in mode {:?}, which does not allow this",
                self.mode
            );
        }
        Ok(())
    }
}

/// `actual` is current iff every dimension the expectation names agrees.
fn fingerprint_current(actual: &TaskFingerprint, expected: &TaskFingerprint) -> bool {
    let same_optional =
        |want: &Option<String>, have: &Option<String>| want.is_none() || want == have;
    actual.task == expected.task
        && actual.evidence_commit == expected.evidence_commit
        && actual.manifest_sha256 == expected.manifest_sha256
        && same_optional(&expected.stutter_sha256, &actual.stutter_sha256)
        && same_optional(&expected.xtask_sha256, &actual.xtask_sha256)
        && same_optional(&expected.workload_sha256, &actual.workload_sha256)
        && same_optional(&expected.scenario_sha256, &actual.scenario_sha256)
        && same_optional(&expected.kernel_release, &actual.kernel_release)
}
