//! Versioned fixed-work completion evidence, independent of compiler tooling.
use serde::{Deserialize, Serialize};
mod reader;
pub use reader::WorkloadProgressReader;
mod runner;
pub use runner::run_fixed_work;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    Compile,
    Custom,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixedWorkIdentity {
    pub kind: WorkloadKind,
    /// Immutable source tree identity: git:<tree object ID> or sha256:<digest>.
    pub source_tree: String,
    pub target: String,
    pub build_state: String,
    pub cache_policy: String,
    pub job_count: u32,
    pub command: Vec<String>,
    pub working_directory: String,
    pub toolchain: String,
    pub environment: String,
}

impl FixedWorkIdentity {
    pub fn validate(&self) -> Result<(), String> {
        let valid_source = self
            .source_tree
            .split_once(':')
            .is_some_and(|(kind, digest)| {
                matches!((kind, digest.len()), ("git", 40 | 64) | ("sha256", 64))
                    && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            });
        if !valid_source {
            return Err("fixed work requires an immutable source tree identity".to_owned());
        }
        if self.job_count == 0
            || self
                .command
                .first()
                .is_none_or(|value| value.trim().is_empty())
            || [
                &self.target,
                &self.build_state,
                &self.cache_policy,
                &self.working_directory,
                &self.toolchain,
                &self.environment,
            ]
            .into_iter()
            .any(|value| value.trim().is_empty())
        {
            return Err("fixed-work identity is incomplete".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProgressSource {
    ScenarioCompletion,
    ExternalMarker,
    CustomCommand,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkloadPhase {
    #[serde(rename = "WORKLOAD_START")]
    Start,
    #[serde(rename = "WORKLOAD_COMPLETE")]
    Complete,
    #[serde(rename = "WORKLOAD_FAILED")]
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadProgress {
    pub schema_version: u32,
    pub execution_id: String,
    pub identity: FixedWorkIdentity,
    pub source: ProgressSource,
    pub phase: WorkloadPhase,
    pub target_root_pid: u32,
    pub target_starttime_ticks: u64,
    pub monotonic_ns: u64,
    pub elapsed_ns: u64,
    pub completed_units: u64,
    pub total_units: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadProgressRecord {
    pub elapsed_ms: u64,
    pub progress: WorkloadProgress,
}

impl WorkloadProgress {
    pub fn validate(&self) -> Result<(), String> {
        self.identity.validate()?;
        if self.schema_version != 1
            || self.execution_id.trim().is_empty()
            || self.target_root_pid == 0
            || self.target_starttime_ticks == 0
            || self.monotonic_ns == 0
            || self.total_units.is_none_or(|total| total == 0)
            || self
                .total_units
                .is_some_and(|total| self.completed_units > total)
        {
            return Err(
                "invalid workload progress version, identity, timestamp or units".to_owned(),
            );
        }
        match self.phase {
            WorkloadPhase::Start if self.completed_units != 0 || self.elapsed_ns != 0 => {
                Err("workload start must have zero elapsed time and completed units".to_owned())
            }
            WorkloadPhase::Complete
                if Some(self.completed_units) != self.total_units || self.elapsed_ns == 0 =>
            {
                Err(
                    "workload completion must finish all fixed work with positive elapsed time"
                        .to_owned(),
                )
            }
            WorkloadPhase::Failed => Err("workload execution failed".to_owned()),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FixedWorkMeasurement {
    pub start: WorkloadProgress,
    pub complete: WorkloadProgress,
}

impl FixedWorkMeasurement {
    pub fn validate(&self) -> Result<(), String> {
        self.start.validate()?;
        self.complete.validate()?;
        if self.start.phase != WorkloadPhase::Start
            || self.complete.phase != WorkloadPhase::Complete
            || self.start.execution_id != self.complete.execution_id
            || self.start.identity != self.complete.identity
            || self.start.source != self.complete.source
            || self.start.target_root_pid != self.complete.target_root_pid
            || self.start.target_starttime_ticks != self.complete.target_starttime_ticks
            || self.start.total_units != self.complete.total_units
            || self
                .complete
                .monotonic_ns
                .checked_sub(self.start.monotonic_ns)
                != Some(self.complete.elapsed_ns)
        {
            return Err("fixed-work completion does not match its start marker".to_owned());
        }
        Ok(())
    }

    pub fn elapsed_ns(&self) -> Result<u64, String> {
        self.validate()?;
        Ok(self.complete.elapsed_ns)
    }
}

/// Exactly one unambiguous successful execution must be wholly represented in
/// the measurement window. Never select the most favorable of several builds.
pub(crate) fn completed_fixed_work<'a>(
    markers: impl Iterator<Item = &'a WorkloadProgress>,
) -> Result<FixedWorkMeasurement, String> {
    let markers = markers.collect::<Vec<_>>();
    let [start, complete] = markers.as_slice() else {
        return Err(
            "fixed work requires exactly one paired start and successful completion in the window"
                .to_owned(),
        );
    };
    let measurement = FixedWorkMeasurement {
        start: (*start).clone(),
        complete: (*complete).clone(),
    };
    measurement.validate()?;
    Ok(measurement)
}

#[cfg(test)]
pub(crate) mod tests;
