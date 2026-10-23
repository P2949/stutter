//! The `overhead-real` task: the same four-mode overhead measurement as the controlled suite,
//! but over a real, representative fixed-work process (rebuilding a Stutter package). The cycle
//! count is calibrated once, frozen in campaign state, and bound into the task fingerprint, so a
//! changed workload invalidates the result instead of silently mixing populations.
use std::time::Instant;

use super::{
    backend::{SuiteBackend, backend_from_env},
    evidence::Accept,
    executor::{FingerprintExtras, RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::TaskSpec,
    model::FailureClass,
    runners::{LiveSuiteRunner, SuiteShape},
    workloads::{
        SystemTools, ToolHost,
        fixed_compile::{self, Identity},
    },
};

pub const FROZEN_CYCLES: &str = "overhead.cycles";
pub const FROZEN_WORKLOAD: &str = "overhead.workload_sha256";

/// Seconds one rebuild cycle takes on this machine.
pub type CycleTimer = dyn Fn(&RunContext<'_>) -> Result<f64, String>;

pub struct OverheadRealRunner {
    pub time_one_cycle: Box<CycleTimer>,
    pub backend: fn() -> Box<dyn SuiteBackend>,
}

impl OverheadRealRunner {
    pub fn system() -> Self {
        Self {
            time_one_cycle: Box::new(|context| {
                let target = fixed_compile::target_dir(&context.dir.0);
                fixed_compile::run_cycles(
                    &SystemTools,
                    context.repository,
                    &target,
                    1,
                    &Instant::now,
                )
            }),
            backend: backend_from_env,
        }
    }
}

fn version(program: &str) -> String {
    SystemTools
        .output(&super::workloads::CommandSpec::new(program, &["--version"]))
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

impl TaskRunner for OverheadRealRunner {
    fn run(&self, context: &RunContext<'_>, spec: &TaskSpec, attempt: u32) -> RunnerOutcome {
        let cycles = match context
            .state
            .frozen
            .get(FROZEN_CYCLES)
            .and_then(|v| v.parse::<u64>().ok())
        {
            Some(cycles) => cycles,
            None => {
                let single = match (self.time_one_cycle)(context) {
                    Ok(seconds) => seconds,
                    Err(error) => {
                        return RunnerOutcome::Failed {
                            class: FailureClass::Retryable,
                            stage: "calibrate".to_owned(),
                            summary: format!("could not time the fixed-compile workload: {error}"),
                            artifacts: None,
                        };
                    }
                };
                let cycles = fixed_compile::choose_cycles(
                    single,
                    context.manifest.overhead.seconds,
                    fixed_compile::MARGIN_SECONDS,
                );
                context.record(StateUpdate::Freeze {
                    key: FROZEN_CYCLES.to_owned(),
                    value: cycles.to_string(),
                });
                cycles
            }
        };
        let identity = fixed_compile::identity(&Identity {
            source_commit: &context.state.evidence_base,
            cargo_lock_sha256: &context.state.cargo_lock_sha256,
            cargo_version: &version("cargo"),
            rustc_version: &version("rustc"),
            cycles,
        });
        let workload_sha256 = fixed_compile::workload_sha256(&identity);
        context.record(StateUpdate::Freeze {
            key: FROZEN_WORKLOAD.to_owned(),
            value: workload_sha256.clone(),
        });
        let runner = LiveSuiteRunner {
            suite: "overhead",
            shape: SuiteShape::Overhead,
            backend: (self.backend)(),
            work_units: Some(cycles),
        };
        match runner.run_and_publish(context, spec, attempt, Accept::PassOnly) {
            Ok(published) => RunnerOutcome::Passed {
                artifacts: Some(context.dir.relative(&published.bundle)),
                receipt: Some(context.dir.relative(&published.receipt)),
                limitations: Vec::new(),
                fingerprint: Box::new(FingerprintExtras {
                    workload_sha256: Some(workload_sha256),
                    ..FingerprintExtras::default()
                }),
                updates: Vec::new(),
            },
            Err(outcome) => outcome,
        }
    }
}
