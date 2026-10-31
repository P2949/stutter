//! Preregistered service scope and coverage floor; never loosen a running campaign's contract.
use std::path::Path;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
pub const FILE: &str = "docs/validation/service-graduation.toml";
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub manager: String,
    pub focus_source: String,
    pub controller_mode: String,
    pub minimum_focus_confidence: f32,
    pub coverage: Coverage,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    pub minimum_dwell_seconds: u64,
    pub maximum_dwell_seconds: u64,
    pub minimum_clean_cycles: u32,
    pub minimum_dangerous_event_repetitions: u32,
    pub minimum_real_reboots: u32,
    pub minimum_keeps_per_operation: u32,
    pub minimum_reverts_per_operation: u32,
}
impl Manifest {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(text)?;
        ensure!(
            manifest.schema_version == 1,
            "unsupported service campaign schema"
        );
        ensure!(
            manifest.manager == "systemd"
                && manifest.focus_source == "heuristic"
                && manifest.controller_mode == "apply-medium-risk",
            "service campaign scope differs from the bounded product templates"
        );
        ensure!(
            manifest.minimum_focus_confidence >= 0.85 && manifest.minimum_focus_confidence <= 1.0,
            "service confidence floor cannot be lowered"
        );
        let c = &manifest.coverage;
        ensure!(
            c.minimum_dwell_seconds >= 8 * 3600
                && c.maximum_dwell_seconds >= c.minimum_dwell_seconds
                && c.maximum_dwell_seconds <= 10 * 3600,
            "service soak needs >=8h dwell and a <=10h bounded extension"
        );
        ensure!(
            c.minimum_clean_cycles >= 6 && c.minimum_dangerous_event_repetitions >= 6,
            "service soak must meet or exceed M9 coverage"
        );
        ensure!(
            c.minimum_real_reboots >= 2 && c.minimum_real_reboots <= 4,
            "real reboot coverage must be repeated and bounded"
        );
        ensure!(
            c.minimum_keeps_per_operation >= 3 && c.minimum_reverts_per_operation >= 3,
            "each operation must repeatedly keep and revert"
        );
        Ok(manifest)
    }
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        Self::parse(
            &std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = include_str!("../../../docs/validation/service-graduation.toml");
    #[test]
    fn the_manifest_enforces_the_plan_instead_of_turning_short_trials_into_qualification() {
        Manifest::parse(SOURCE).unwrap();
        for (old, new) in [
            ("28800", "3600"),
            ("minimum_clean_cycles = 6", "minimum_clean_cycles = 1"),
            ("minimum_real_reboots = 2", "minimum_real_reboots = 0"),
            ("0.85", "0.6"),
        ] {
            assert!(Manifest::parse(&SOURCE.replace(old, new)).is_err());
        }
    }
}
