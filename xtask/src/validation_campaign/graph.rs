//! The Core Validation DAG. Dependencies and the named validation gates are encoded here so the agent never
//! reconstructs them from prose.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, ensure};

use super::model::{CampaignPhase, ValidationGate};

/// What executes a task; the executor dispatches runners on this.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TaskKind {
    OfflineReady,
    LiveObservation,
    GameRunnable,
    OverheadControlled,
    OverheadReal,
    ScoreRunnable,
    KernelMatrix,
    Corpus,
    FrameScore,
    EvidenceIntegration,
    DiagnosisCalibration,
    ScoreReview,
    CapabilityPolicy,
    ReleaseDocs,
    FreezeRc,
    RcOffline,
    RcLiveObservation,
    RcGameRunnable,
    RcAffinity,
    RcRecovery,
    ReleasePackage,
    ReleaseVerify,
    InstallSmoke,
    Tag,
}

/// Which commit a task's result is bound to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceBinding {
    /// Not tied to a specific frozen commit (gates over whatever `main` is).
    None,
    /// The campaign's `EVIDENCE_BASE`.
    EvidenceBase,
    /// The frozen release candidate.
    ReleaseCandidate,
}

#[derive(Clone, Debug)]
pub struct TaskSpec {
    pub id: &'static str,
    pub kind: TaskKind,
    pub phase: CampaignPhase,
    pub depends_on: &'static [&'static str],
    pub binding: SourceBinding,
    /// Only optional tasks may be skipped by manifest policy.
    pub optional: bool,
}

const fn task(
    id: &'static str,
    kind: TaskKind,
    phase: CampaignPhase,
    depends_on: &'static [&'static str],
    binding: SourceBinding,
) -> TaskSpec {
    TaskSpec {
        id,
        kind,
        phase,
        depends_on,
        binding,
        optional: false,
    }
}

const COLLECTION: &[&str] = &[
    "live-observation",
    "game-runnable",
    "overhead-controlled",
    "overhead-real",
    "score-runnable",
    "kernel-matrix",
    "corpus",
    "frame-score",
];
const RC_LIVE: &[&str] = &[
    "rc-live-observation",
    "rc-game-runnable",
    "rc-affinity",
    "rc-recovery",
];

pub fn core_validation_tasks() -> Vec<TaskSpec> {
    use CampaignPhase as P;
    use SourceBinding as B;
    use TaskKind as K;
    vec![
        task("offline-ready", K::OfflineReady, P::Preflight, &[], B::None),
        task(
            "live-observation",
            K::LiveObservation,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "game-runnable",
            K::GameRunnable,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "overhead-controlled",
            K::OverheadControlled,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "overhead-real",
            K::OverheadReal,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "score-runnable",
            K::ScoreRunnable,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "kernel-matrix",
            K::KernelMatrix,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "corpus",
            K::Corpus,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "frame-score",
            K::FrameScore,
            P::Collection,
            &["offline-ready"],
            B::EvidenceBase,
        ),
        task(
            "evidence-integration",
            K::EvidenceIntegration,
            P::Integration,
            COLLECTION,
            B::None,
        ),
        task(
            "diagnosis-calibration",
            K::DiagnosisCalibration,
            P::Integration,
            &["evidence-integration"],
            B::None,
        ),
        task(
            "score-review",
            K::ScoreReview,
            P::Integration,
            &["evidence-integration"],
            B::None,
        ),
        task(
            "capability-policy",
            K::CapabilityPolicy,
            P::Policy,
            &["diagnosis-calibration", "score-review"],
            B::None,
        ),
        task(
            "release-docs",
            K::ReleaseDocs,
            P::Policy,
            &["capability-policy"],
            B::None,
        ),
        task(
            "freeze-rc",
            K::FreezeRc,
            P::ReleaseCandidate,
            &["release-docs"],
            B::None,
        ),
        task(
            "rc-offline",
            K::RcOffline,
            P::ReleaseCandidate,
            &["freeze-rc"],
            B::ReleaseCandidate,
        ),
        task(
            "rc-live-observation",
            K::RcLiveObservation,
            P::ReleaseCandidate,
            &["freeze-rc"],
            B::ReleaseCandidate,
        ),
        task(
            "rc-game-runnable",
            K::RcGameRunnable,
            P::ReleaseCandidate,
            &["freeze-rc"],
            B::ReleaseCandidate,
        ),
        task(
            "rc-affinity",
            K::RcAffinity,
            P::ReleaseCandidate,
            &["freeze-rc"],
            B::ReleaseCandidate,
        ),
        task(
            "rc-recovery",
            K::RcRecovery,
            P::ReleaseCandidate,
            &["freeze-rc"],
            B::ReleaseCandidate,
        ),
        task(
            "release-package",
            K::ReleasePackage,
            P::Release,
            &["rc-offline"],
            B::ReleaseCandidate,
        ),
        task(
            "release-verify",
            K::ReleaseVerify,
            P::Release,
            &[
                "release-package",
                "rc-live-observation",
                "rc-game-runnable",
                "rc-affinity",
                "rc-recovery",
            ],
            B::ReleaseCandidate,
        ),
        task(
            "install-smoke",
            K::InstallSmoke,
            P::Release,
            &["release-package"],
            B::ReleaseCandidate,
        ),
        task(
            "tag",
            K::Tag,
            P::Release,
            &["release-verify", "install-smoke", "rc-offline"],
            B::ReleaseCandidate,
        ),
    ]
}

/// The named validation gates: the task sets whose success makes each area of the validation
/// verified. Nothing here refers to a hosting service.
pub fn validation_gates() -> Vec<ValidationGate> {
    let gate = |name: &str, title: &str, tasks: &[&str]| ValidationGate {
        name: name.to_owned(),
        title: title.to_owned(),
        tasks: tasks.iter().map(|task| (*task).to_owned()).collect(),
    };
    let mut release = vec![
        "evidence-integration",
        "capability-policy",
        "rc-offline",
        "release-package",
        "release-verify",
        "install-smoke",
        "tag",
    ];
    release.extend(RC_LIVE);
    vec![
        gate(
            "observation",
            "live observation and game-runnable evidence",
            &["live-observation", "game-runnable"],
        ),
        gate(
            "kernel-compatibility",
            "kernel compatibility on distinct real kernels",
            &["kernel-matrix"],
        ),
        gate(
            "profiler-overhead",
            "profiler overhead, controlled and real workload",
            &["overhead-controlled", "overhead-real"],
        ),
        gate(
            "diagnosis-corpus",
            "provenance-backed diagnosis corpus",
            &["corpus"],
        ),
        gate(
            "diagnosis-calibration",
            "diagnosis calibration",
            &["diagnosis-calibration"],
        ),
        gate(
            "score-evaluation",
            "StutterScore evaluation",
            &["score-runnable", "frame-score", "score-review"],
        ),
        gate(
            "capability-policy",
            "capability tiers",
            &["capability-policy"],
        ),
        gate(
            "release-readiness",
            "exact-RC acceptance and v0.1.0 release",
            &release,
        ),
    ]
}

/// A validated task graph.
pub struct Graph {
    tasks: BTreeMap<&'static str, TaskSpec>,
    order: Vec<&'static str>,
}

impl Graph {
    pub fn new(specs: Vec<TaskSpec>) -> anyhow::Result<Self> {
        let mut tasks = BTreeMap::new();
        for spec in specs {
            let id = spec.id;
            ensure!(tasks.insert(id, spec).is_none(), "duplicate task {id}");
        }
        for spec in tasks.values() {
            for dependency in spec.depends_on {
                ensure!(
                    tasks.contains_key(dependency),
                    "task {} depends on unknown task {dependency}",
                    spec.id
                );
            }
        }
        // Kahn's algorithm; leftover tasks mean a cycle.
        let mut remaining: BTreeMap<&str, usize> =
            tasks.values().map(|s| (s.id, s.depends_on.len())).collect();
        let mut order = Vec::new();
        let mut frontier: Vec<&'static str> = tasks
            .values()
            .filter(|s| s.depends_on.is_empty())
            .map(|s| s.id)
            .collect();
        while let Some(id) = frontier.pop() {
            order.push(id);
            for dependent in tasks.values().filter(|s| s.depends_on.contains(&id)) {
                let count = remaining.entry(dependent.id).or_default();
                *count -= 1;
                if *count == 0 {
                    frontier.push(dependent.id);
                }
            }
        }
        if order.len() != tasks.len() {
            bail!("task graph has a dependency cycle");
        }
        Ok(Self { tasks, order })
    }

    pub fn core() -> anyhow::Result<Self> {
        Self::new(core_validation_tasks())
    }

    /// Only tasks the graph marks optional may be skipped by manifest policy; skipping a gate
    /// would turn missing evidence into a pass.
    pub fn check_skip_policy(&self, skip: &[String]) -> anyhow::Result<()> {
        for id in skip {
            let spec = self
                .tasks
                .get(id.as_str())
                .ok_or_else(|| anyhow::anyhow!("policy.skip_tasks names unknown task {id}"))?;
            ensure!(
                spec.optional,
                "task {id} is not optional and cannot be skipped by policy"
            );
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&TaskSpec> {
        self.tasks.get(id)
    }

    pub fn specs(&self) -> impl Iterator<Item = &TaskSpec> {
        self.tasks.values()
    }

    /// Topological order, dependencies first (deterministic for equal inputs).
    pub fn topological(&self) -> Vec<&'static str> {
        let mut order = self.order.clone();
        // Stable presentation: dependencies first, ties by phase then id.
        order.sort_by_key(|id| (self.depth(id), self.tasks[id].phase, *id));
        order
    }

    fn depth(&self, id: &str) -> usize {
        self.tasks[id]
            .depends_on
            .iter()
            .map(|dependency| self.depth(dependency) + 1)
            .max()
            .unwrap_or(0)
    }

    /// Every task that (transitively) depends on `id`.
    pub fn dependents(&self, id: &str) -> BTreeSet<&'static str> {
        let mut found = BTreeSet::new();
        let mut stack = vec![id.to_owned()];
        while let Some(current) = stack.pop() {
            for spec in self.tasks.values() {
                if spec.depends_on.contains(&current.as_str()) && found.insert(spec.id) {
                    stack.push(spec.id.to_owned());
                }
            }
        }
        found
    }
}
