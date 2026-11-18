//! Native-result adapter. The source must independently attest the game process and context;
//! this module only snapshots the result directory and invokes Cyberpunk's existing verifier.
use std::path::PathBuf;

use super::{
    cyberpunk::{
        BenchmarkDirectorySnapshot, CyberpunkBenchmarkAttempt,
        snapshot_benchmark_results_or_absent, verify_benchmark_completion,
    },
    steam_host::GameState,
    steam_port::ResultProbe,
};

pub enum Observation {
    Starting,
    Running,
    Completed(Box<CyberpunkBenchmarkAttempt>),
    ExitedWithoutResult,
    IdentityMismatch,
}

/// Supplies independent process, entitlement, renderer and settings observations. Constructing
/// an observation is not itself proof; the production source still requires live qualification.
pub trait NativeObservationSource {
    fn begin(&mut self, arm: super::ProtonSource) -> anyhow::Result<()>;
    fn poll(&mut self) -> Observation;
    /// The game process this attempt's source has sighted and attested, if any.
    fn sighted_process(&self) -> Option<super::action_child::ChildTarget> {
        None
    }
}

/// The native results directory of an arm.
pub type ResultsFor = Box<dyn Fn(super::ProtonSource) -> PathBuf + Send>;

pub struct NativeResultProbe<S> {
    results_for: ResultsFor,
    results_dir: PathBuf,
    owner_uid: u32,
    source: S,
    before: Option<BenchmarkDirectorySnapshot>,
}

impl<S> NativeResultProbe<S> {
    pub fn new(results_dir: PathBuf, owner_uid: u32, source: S) -> Self {
        let fixed = results_dir.clone();
        Self::per_arm(
            Box::new(move |_| fixed.clone()),
            results_dir,
            owner_uid,
            source,
        )
    }

    /// Each arm observes its own prefix's results; `initial` is only used before `begin`.
    pub fn per_arm(results_for: ResultsFor, initial: PathBuf, owner_uid: u32, source: S) -> Self {
        Self {
            results_for,
            results_dir: initial,
            owner_uid,
            source,
            before: None,
        }
    }
}

impl<S: NativeObservationSource> ResultProbe for NativeResultProbe<S> {
    fn game_process(&self) -> Option<super::action_child::ChildTarget> {
        self.source.sighted_process()
    }

    fn begin(&mut self, arm: super::ProtonSource) -> anyhow::Result<()> {
        self.before = None;
        self.results_dir = (self.results_for)(arm);
        let snapshot = snapshot_benchmark_results_or_absent(&self.results_dir, self.owner_uid)?;
        self.source.begin(arm)?;
        self.before = Some(snapshot);
        Ok(())
    }

    fn poll(&mut self) -> GameState {
        let Some(before) = self.before.as_ref() else {
            return GameState::InvalidNativeResult;
        };
        match self.source.poll() {
            Observation::Starting => GameState::Starting,
            Observation::Running => GameState::Running,
            Observation::ExitedWithoutResult => GameState::ExitedWithoutResult,
            Observation::IdentityMismatch => GameState::IdentityMismatch,
            Observation::Completed(attempt) => {
                match verify_benchmark_completion(&self.results_dir, before, &attempt) {
                    Ok(completion) => GameState::Result {
                        completion: Box::new(completion),
                    },
                    Err(error)
                        if error
                            .downcast_ref::<super::cyberpunk::NoFreshBenchmarkResult>()
                            .is_some() =>
                    {
                        GameState::ExitedWithoutResult
                    }
                    Err(error) => {
                        log::warn!("native result rejected: {error:#}");
                        GameState::InvalidNativeResult
                    }
                }
            }
        }
    }
}
