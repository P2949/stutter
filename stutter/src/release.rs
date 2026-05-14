use std::{fmt, str::FromStr};

use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseChannel {
    Experimental,
    ObserveStable,
    LowRiskStable,
    MediumRisk,
}

impl ReleaseChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Experimental => "experimental",
            Self::ObserveStable => "observe-stable",
            Self::LowRiskStable => "low-risk-stable",
            Self::MediumRisk => "medium-risk",
        }
    }
}

impl fmt::Display for ReleaseChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ReleaseChannel {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "experimental" => Ok(Self::Experimental),
            "observe-stable" | "observe" => Ok(Self::ObserveStable),
            "low-risk-stable" | "low-risk" => Ok(Self::LowRiskStable),
            "medium-risk" => Ok(Self::MediumRisk),
            other => anyhow::bail!(
                "unknown release channel {other:?}; expected experimental, observe-stable, low-risk-stable, or medium-risk"
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReleaseReadinessInputs {
    pub apply_actions_enabled: bool,
    pub stable_service_packaging: bool,
    pub retention_controls: bool,
    pub health_and_status: bool,
    pub action_runner_mandatory: bool,
    pub universal_rollback_for_enabled_families: bool,
    pub crash_recovery: bool,
    pub soak_tests: bool,
    pub service_packaging: bool,
    pub docs: bool,
    pub per_action_opt_in: bool,
    pub stronger_tests: bool,
    pub manual_confirmation_or_explicit_config: bool,
}

impl Default for ReleaseReadinessInputs {
    fn default() -> Self {
        Self {
            apply_actions_enabled: false,
            stable_service_packaging: true,
            retention_controls: true,
            health_and_status: true,
            action_runner_mandatory: true,
            universal_rollback_for_enabled_families: true,
            crash_recovery: true,
            soak_tests: false,
            service_packaging: true,
            docs: true,
            per_action_opt_in: true,
            stronger_tests: false,
            manual_confirmation_or_explicit_config: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReleaseReadinessReport {
    pub channel: ReleaseChannel,
    pub passed: bool,
    pub gates: Vec<ReleaseGate>,
    pub changelog_categories: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ReleaseGate {
    pub code: &'static str,
    pub required: bool,
    pub passed: bool,
    pub description: &'static str,
}

pub fn evaluate_release_readiness(
    channel: ReleaseChannel,
    inputs: &ReleaseReadinessInputs,
) -> ReleaseReadinessReport {
    let gates = match channel {
        ReleaseChannel::Experimental => vec![gate(
            "experimental_channel_declared",
            true,
            true,
            "release is explicitly marked experimental",
        )],
        ReleaseChannel::ObserveStable => vec![
            gate(
                "no_apply_actions",
                true,
                !inputs.apply_actions_enabled,
                "observe-stable must not enable apply actions",
            ),
            gate(
                "stable_service",
                true,
                inputs.stable_service_packaging,
                "observe-stable requires a stable service path",
            ),
            gate(
                "retention_controls",
                true,
                inputs.retention_controls,
                "observe-stable requires disk and retention controls",
            ),
            gate(
                "health_status",
                true,
                inputs.health_and_status,
                "observe-stable requires health and status surfaces",
            ),
        ],
        ReleaseChannel::LowRiskStable => vec![
            gate(
                "action_runner_mandatory",
                true,
                inputs.action_runner_mandatory,
                "low-risk-stable requires all apply paths to use ActionRunner",
            ),
            gate(
                "universal_rollback",
                true,
                inputs.universal_rollback_for_enabled_families,
                "low-risk-stable requires rollback for every enabled action family",
            ),
            gate(
                "crash_recovery",
                true,
                inputs.crash_recovery,
                "low-risk-stable requires startup crash recovery",
            ),
            gate(
                "soak_tests",
                true,
                inputs.soak_tests,
                "low-risk-stable requires long-running soak evidence",
            ),
            gate(
                "service_packaging",
                true,
                inputs.service_packaging,
                "low-risk-stable requires service packaging",
            ),
            gate("docs", true, inputs.docs, "low-risk-stable requires docs"),
        ],
        ReleaseChannel::MediumRisk => vec![
            gate(
                "per_action_opt_in",
                true,
                inputs.per_action_opt_in,
                "medium-risk requires per-action opt-in",
            ),
            gate(
                "stronger_tests",
                true,
                inputs.stronger_tests,
                "medium-risk requires stronger safety tests",
            ),
            gate(
                "manual_confirmation_or_explicit_config",
                true,
                inputs.manual_confirmation_or_explicit_config,
                "medium-risk requires manual confirmation or explicit config",
            ),
        ],
    };

    ReleaseReadinessReport {
        channel,
        passed: gates
            .iter()
            .filter(|gate| gate.required)
            .all(|gate| gate.passed),
        gates,
        changelog_categories: vec!["safety", "tuning behavior", "rollback", "config migration"],
    }
}

fn gate(
    code: &'static str,
    required: bool,
    passed: bool,
    description: &'static str,
) -> ReleaseGate {
    ReleaseGate {
        code,
        required,
        passed,
        description,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_stable_passes_only_without_apply_actions() {
        let mut inputs = ReleaseReadinessInputs {
            apply_actions_enabled: false,
            ..ReleaseReadinessInputs::default()
        };
        let report = evaluate_release_readiness(ReleaseChannel::ObserveStable, &inputs);
        assert!(report.passed);

        inputs.apply_actions_enabled = true;
        let report = evaluate_release_readiness(ReleaseChannel::ObserveStable, &inputs);
        assert!(!report.passed);
        assert_eq!(report.gates[0].code, "no_apply_actions");
    }

    #[test]
    fn low_risk_stable_is_blocked_without_soak_tests() {
        let inputs = ReleaseReadinessInputs::default();

        let report = evaluate_release_readiness(ReleaseChannel::LowRiskStable, &inputs);

        assert!(!report.passed);
        assert!(
            report
                .gates
                .iter()
                .any(|gate| gate.code == "soak_tests" && !gate.passed)
        );
    }

    #[test]
    fn medium_risk_requires_explicit_stronger_tests() {
        let inputs = ReleaseReadinessInputs::default();

        let report = evaluate_release_readiness(ReleaseChannel::MediumRisk, &inputs);

        assert!(!report.passed);
        assert!(
            report
                .gates
                .iter()
                .any(|gate| gate.code == "stronger_tests" && !gate.passed)
        );
    }

    #[test]
    fn release_channel_parses_aliases() {
        assert_eq!(
            "low-risk".parse::<ReleaseChannel>().unwrap(),
            ReleaseChannel::LowRiskStable
        );
    }
}
