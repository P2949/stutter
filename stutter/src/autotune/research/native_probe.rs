//! Native-result adapter. The source must independently attest the game process and context;
//! this module only snapshots the result directory and invokes Cyberpunk's existing verifier.
use std::path::PathBuf;

use super::{
    cyberpunk::{
        BenchmarkDirectorySnapshot, CyberpunkBenchmarkAttempt, snapshot_benchmark_results,
        verify_benchmark_completion,
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
    fn begin(&mut self) -> anyhow::Result<()>;
    fn poll(&mut self) -> Observation;
}

pub struct NativeResultProbe<S> {
    results_dir: PathBuf,
    owner_uid: u32,
    source: S,
    before: Option<BenchmarkDirectorySnapshot>,
}

impl<S> NativeResultProbe<S> {
    pub fn new(results_dir: PathBuf, owner_uid: u32, source: S) -> Self {
        Self {
            results_dir,
            owner_uid,
            source,
            before: None,
        }
    }
}

impl<S: NativeObservationSource> ResultProbe for NativeResultProbe<S> {
    fn begin(&mut self) -> anyhow::Result<()> {
        self.before = None;
        let snapshot = snapshot_benchmark_results(&self.results_dir, self.owner_uid)?;
        self.source.begin()?;
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
                    Err(_) => GameState::InvalidNativeResult,
                }
            }
        }
    }
}
