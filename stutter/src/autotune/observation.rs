#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use super::state::SituationKind;
use crate::{diagnosis::LiveDiagnosisEntry, scorer::StutterScore};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnlineDataQuality {
    pub usable: bool,
    pub min_scored_samples_met: bool,
    pub min_scored_intervals_met: bool,
    pub drop_counters_ok: bool,
    pub target_identity_stable: bool,
    pub target_present: bool,
    pub warnings: Vec<String>,
}

impl Default for OnlineDataQuality {
    fn default() -> Self {
        Self {
            usable: false,
            min_scored_samples_met: false,
            min_scored_intervals_met: false,
            drop_counters_ok: true,
            target_identity_stable: true,
            target_present: false,
            warnings: Vec::new(),
        }
    }
}

impl OnlineDataQuality {
    pub fn good() -> Self {
        Self {
            usable: true,
            min_scored_samples_met: true,
            min_scored_intervals_met: true,
            drop_counters_ok: true,
            target_identity_stable: true,
            target_present: true,
            warnings: Vec::new(),
        }
    }

    pub fn blocks_action(&self) -> bool {
        !self.usable
            || !self.min_scored_samples_met
            || !self.min_scored_intervals_met
            || !self.drop_counters_ok
            || !self.target_identity_stable
            || !self.target_present
    }

    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutotuneObservation {
    pub now_unix_nanos: u128,
    pub elapsed_ms: u64,

    pub target_present: bool,
    pub target_root_pid: Option<u32>,
    pub active_target_count: usize,
    pub scored_task_count: usize,

    pub interval_count: usize,
    pub scored_samples: u64,

    pub score: StutterScore,
    pub data_quality: OnlineDataQuality,

    pub primary_situation: SituationKind,
    pub recent_diagnoses: Vec<LiveDiagnosisEntry>,

    pub frame_count: usize,
    pub frame_p99_ms: f64,
    pub frame_max_ms: f64,

    pub drop_counter_total: u64,
}

impl Default for AutotuneObservation {
    fn default() -> Self {
        Self {
            now_unix_nanos: 0,
            elapsed_ms: 0,
            target_present: false,
            target_root_pid: None,
            active_target_count: 0,
            scored_task_count: 0,
            interval_count: 0,
            scored_samples: 0,
            score: StutterScore::default(),
            data_quality: OnlineDataQuality::default(),
            primary_situation: SituationKind::Unknown,
            recent_diagnoses: Vec::new(),
            frame_count: 0,
            frame_p99_ms: 0.0,
            frame_max_ms: 0.0,
            drop_counter_total: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_observation_blocks_action() {
        let observation = AutotuneObservation::default();

        assert!(!observation.target_present);
        assert_eq!(observation.primary_situation, SituationKind::Unknown);
        assert!(observation.data_quality.blocks_action());
    }

    #[test]
    fn good_data_quality_does_not_block_action() {
        let quality = OnlineDataQuality::good();

        assert!(quality.usable);
        assert!(!quality.blocks_action());
    }
}
