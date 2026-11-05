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
/// The targeted acceptance manifest. It is a separate contract with its own evidence suite and
/// can never be mistaken for the endurance contract above.
pub const ACCEPTANCE_FILE: &str = "docs/validation/service-acceptance.toml";

impl Manifest {
    /// The manifest file a campaign was started with.
    pub fn file_for(acceptance: bool) -> &'static str {
        if acceptance { ACCEPTANCE_FILE } else { FILE }
    }

    pub fn load_for(path: &Path, acceptance: bool) -> anyhow::Result<Self> {
        if !acceptance {
            return Self::load(path);
        }
        Self::parse_acceptance(
            &std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?,
        )
    }

    /// Same scope and confidence floor as the endurance manifest; coverage is event-complete
    /// and bounded instead of time-based. Nothing here can be reached from `parse`.
    pub fn parse_acceptance(text: &str) -> anyhow::Result<Self> {
        let manifest: Self = toml::from_str(text)?;
        ensure!(
            manifest.schema_version == 1,
            "unsupported service acceptance schema"
        );
        ensure!(
            manifest.manager == "systemd"
                && manifest.focus_source == "heuristic"
                && manifest.controller_mode == "apply-medium-risk",
            "service acceptance scope differs from the bounded product templates"
        );
        ensure!(
            manifest.minimum_focus_confidence >= 0.85 && manifest.minimum_focus_confidence <= 1.0,
            "service confidence floor cannot be lowered"
        );
        let c = &manifest.coverage;
        ensure!(
            c.minimum_dwell_seconds == 0,
            "targeted acceptance has no dwell floor; a dwell requirement belongs to endurance"
        );
        ensure!(
            (1800..=6 * 3600).contains(&c.maximum_dwell_seconds),
            "the acceptance timeout must be between 30 minutes and 6 hours"
        );
        ensure!(
            c.minimum_clean_cycles >= 1 && c.minimum_dangerous_event_repetitions >= 1,
            "acceptance needs at least one clean cycle with every event"
        );
        ensure!(
            c.minimum_real_reboots == 0,
            "in-campaign reboots belong to endurance; production boot is checked separately"
        );
        ensure!(
            c.minimum_keeps_per_operation >= 1 && c.minimum_reverts_per_operation >= 1,
            "each operation must keep and revert at least once"
        );
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = include_str!("../../../docs/validation/service-graduation.toml");

    const ACCEPTANCE: &str = include_str!("../../../docs/validation/service-acceptance.toml");

    #[test]
    fn acceptance_is_a_separate_contract_that_cannot_pass_as_endurance() {
        Manifest::parse_acceptance(ACCEPTANCE).unwrap();
        // The endurance parser refuses it, so a short run can never be loaded as endurance.
        assert!(Manifest::parse(ACCEPTANCE).is_err());
        // And the acceptance parser keeps its own guard rails.
        for (old, new) in [
            ("minimum_dwell_seconds = 0", "minimum_dwell_seconds = 600"),
            ("minimum_real_reboots = 0", "minimum_real_reboots = 2"),
            ("minimum_clean_cycles = 1", "minimum_clean_cycles = 0"),
            (
                "maximum_dwell_seconds = 14400",
                "maximum_dwell_seconds = 100",
            ),
            ("0.85", "0.6"),
        ] {
            assert!(Manifest::parse_acceptance(&ACCEPTANCE.replace(old, new)).is_err());
        }
    }
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
