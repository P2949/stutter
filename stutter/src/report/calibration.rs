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
    /// Separate from `confidence`: whether the cause is established as the cause of visible
    /// stutter (`CausalConfidence`).
    pub causal_confidence: String,
    pub score: Option<f32>,
}

/// Diagnosis families whose single significance threshold can be swept.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalibrationFamily {
    /// Scheduler delay; the value is nanoseconds.
    Scheduler,
    /// Blocking block-I/O significance; nanoseconds.
    BlockIo,
    /// IRQ handler significance; nanoseconds.
    Irq,
    /// GPU-busy bound; percent.
    Gpu,
}

impl CalibrationFamily {
    fn config(self, value: f64) -> DiagnosisConfig {
        let mut config = DiagnosisConfig::default();
        match self {
            Self::Scheduler => {
                config.sched_delay_significant_ns = value as u64;
                config.min_scheduler_latency_for_primary_ns = value as u64;
            }
            Self::BlockIo => config.block_io_significant_ns = value as u64,
            Self::Irq => config.irq_significant_ns = value as u64,
            Self::Gpu => config.gpu_busy_bound_percent = value as u32,
        }
        config
    }
}

/// Diagnoses `run` with one family's threshold set to `value`; every other threshold is the
/// default. Uses the production clustering, artifact filtering and diagnosis.
pub fn family_calibration_outcome(
    run: &Path,
    family: CalibrationFamily,
    value: f64,
) -> anyhow::Result<Option<CalibrationOutcome>> {
    let result = build_report_analysis_from_input_with_config(
        load_report_input(run)?,
        10,
        5,
        None,
        family.config(value),
    )?;
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
            causal_confidence: format!("{:?}", diagnosis.causal_confidence),
            score: diagnosis.primary.as_ref().map(|primary| primary.score),
        }))
}

/// Scheduler-delay sweep point (nanoseconds).
pub fn scheduler_calibration_outcome(
    run: &Path,
    sched_delay_ns: u64,
) -> anyhow::Result<Option<CalibrationOutcome>> {
    family_calibration_outcome(run, CalibrationFamily::Scheduler, sched_delay_ns as f64)
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

    #[test]
    fn default_fixtures_report_causal_confidence_within_observation_confidence() {
        let rank = |word: &str| {
            ["NotEstablished", "Low", "Medium", "High"]
                .iter()
                .position(|w| *w == word)
                .unwrap()
        };
        for (family, fixture, value) in [
            (
                CalibrationFamily::Scheduler,
                "game_thread_scheduler_delay",
                2_000_000.0,
            ),
            (CalibrationFamily::BlockIo, "block_io_stall", 1_000_000.0),
            (CalibrationFamily::Irq, "irq_heavy", 250_000.0),
            (CalibrationFamily::Gpu, "gpu_bound_clean_cpu", 95.0),
        ] {
            let run = crate::test_fixture_builder::fixture_path(fixture);
            let outcome = family_calibration_outcome(&run, family, value)
                .unwrap()
                .unwrap();
            assert!(
                rank(&outcome.causal_confidence) <= rank(&outcome.confidence),
                "{fixture}: causal {} exceeds observation {}",
                outcome.causal_confidence,
                outcome.confidence
            );
            // Scheduler overlap is temporal coincidence: High needs an explicit attribution chain.
            if fixture == "game_thread_scheduler_delay" {
                assert_ne!(outcome.causal_confidence, "High", "{fixture}");
            }
        }
    }
}
