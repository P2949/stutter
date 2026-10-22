//! Calibration entry point: the production report pipeline with an explicit scheduler-delay
//! threshold, so threshold sweeps cannot drift from what users run.
use std::path::Path;

use super::{
    DiagnosisConfig, analysis::build_report_analysis_from_input_with_config,
    load::load_report_input,
};

/// Primary diagnosis of the first diagnosed cluster of a run (the corpus tests' convention).
#[derive(Clone, Debug, PartialEq)]
pub struct CalibrationOutcome {
    pub cause: String,
    pub confidence: String,
    pub score: Option<f32>,
}

/// Diagnoses `run` with `sched_delay_ns` as both the scheduler-delay significance and the
/// minimum scheduler latency for a primary claim; every other threshold is the default.
pub fn scheduler_calibration_outcome(
    run: &Path,
    sched_delay_ns: u64,
) -> anyhow::Result<Option<CalibrationOutcome>> {
    let config = DiagnosisConfig {
        sched_delay_significant_ns: sched_delay_ns,
        min_scheduler_latency_for_primary_ns: sched_delay_ns,
        ..DiagnosisConfig::default()
    };
    let result =
        build_report_analysis_from_input_with_config(load_report_input(run)?, 10, 5, None, config)?;
    Ok(result
        .analysis
        .cluster_analysis
        .clusters
        .iter()
        .filter_map(|cluster| cluster.diagnosis.as_ref())
        .next()
        .map(|diagnosis| CalibrationOutcome {
            cause: format!("{:?}", diagnosis.cause),
            confidence: format!("{:?}", diagnosis.confidence),
            score: diagnosis.primary.as_ref().map(|primary| primary.score),
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_threshold_reproduces_the_corpus_expectation_and_a_huge_one_suppresses_it() {
        let run = crate::test_fixture_builder::fixture_path("game_thread_scheduler_delay");
        let default = scheduler_calibration_outcome(&run, 2_000_000)
            .unwrap()
            .unwrap();
        assert_eq!(default.cause, "GameThreadSchedulerDelay");
        let strict = scheduler_calibration_outcome(&run, 10_000_000_000).unwrap();
        assert_ne!(
            strict.map(|o| o.cause).as_deref(),
            Some("GameThreadSchedulerDelay")
        );
    }
}
