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
    pub real_machine_validation: bool,
    pub real_validation_matrix: bool,
    pub false_negative_catalogue: bool,
    pub multi_machine_validation: bool,
    pub local_install_smoke_tests: bool,
    pub service_doctor_smoke_tests: bool,
    pub emergency_restore_smoke_tests: bool,
    pub unprivileged_report_smoke_tests: bool,
    pub packaged_artifact_layout_tests: bool,
    pub service_start_stop_smoke_tests: bool,
    pub rollback_drill: bool,

    /// Whether distro packages may be described as production-ready.
    ///
    /// This is intentionally separate from service unit packaging. The current
    /// tree ships local install helpers and service templates, but distro
    /// package recipes remain skeletons until the eBPF artifact and install
    /// flow are reproducible in package-manager builds.
    pub production_distro_packaging: bool,

    /// Whether the eBPF object has a reproducible package-manager build or a
    /// documented prebuilt release artifact path suitable for distro packages.
    pub reproducible_packaged_ebpf_object: bool,

    /// Whether install/package tests cover the ebuild/PKGBUILD/tarball layout.
    pub packaging_install_tests: bool,

    /// Whether packaged service units have start/stop smoke evidence.
    pub packaging_service_smoke_tests: bool,

    /// Whether tagged release tarballs/artifacts are available for packagers.
    pub versioned_release_tarball: bool,
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
            real_machine_validation: false,
            real_validation_matrix: false,
            false_negative_catalogue: false,
            multi_machine_validation: false,
            local_install_smoke_tests: false,
            service_doctor_smoke_tests: false,
            emergency_restore_smoke_tests: false,
            unprivileged_report_smoke_tests: false,
            packaged_artifact_layout_tests: false,
            service_start_stop_smoke_tests: false,
            rollback_drill: false,

            production_distro_packaging: false,
            reproducible_packaged_ebpf_object: false,
            packaging_install_tests: false,
            packaging_service_smoke_tests: false,
            versioned_release_tarball: false,
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
    let mut gates = match channel {
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
                "observe-stable requires stable service unit/install-script behavior, not production distro packaging",
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
            gate(
                "local_install_smoke_tests",
                true,
                inputs.local_install_smoke_tests,
                "observe-stable requires local install smoke evidence",
            ),
            gate(
                "service_doctor_smoke_tests",
                true,
                inputs.service_doctor_smoke_tests,
                "observe-stable requires service doctor dry-run smoke evidence",
            ),
            gate(
                "unprivileged_report_smoke_tests",
                true,
                inputs.unprivileged_report_smoke_tests,
                "observe-stable requires unprivileged report/recommend smoke evidence",
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
                "real_machine_validation",
                true,
                inputs.real_machine_validation,
                "low-risk-stable requires real-machine validation",
            ),
            gate(
                "real_validation_matrix",
                true,
                inputs.real_validation_matrix,
                "low-risk-stable requires the real validation matrix gate",
            ),
            gate(
                "false_negative_catalogue",
                true,
                inputs.false_negative_catalogue,
                "low-risk-stable requires tracked false-negative catalogue support",
            ),
            gate(
                "service_packaging",
                true,
                inputs.service_packaging,
                "low-risk-stable requires service unit templates and local install support, not production distro packaging",
            ),
            gate(
                "local_install_smoke_tests",
                true,
                inputs.local_install_smoke_tests,
                "low-risk-stable requires local install smoke evidence",
            ),
            gate(
                "emergency_restore_smoke_tests",
                true,
                inputs.emergency_restore_smoke_tests,
                "low-risk-stable requires emergency restore smoke evidence",
            ),
            gate(
                "service_start_stop_smoke_tests",
                true,
                inputs.service_start_stop_smoke_tests,
                "low-risk-stable requires service start/stop smoke evidence",
            ),
            gate(
                "rollback_drill",
                true,
                inputs.rollback_drill,
                "low-risk-stable requires rollback drill evidence",
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
            gate(
                "multi_machine_validation",
                true,
                inputs.multi_machine_validation,
                "medium-risk requires multi-machine validation evidence",
            ),
        ],
    };

    gates.extend(packaging_roadmap_gates(inputs));

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

fn packaging_roadmap_gates(inputs: &ReleaseReadinessInputs) -> Vec<ReleaseGate> {
    vec![
        gate(
            "production_distro_packaging",
            false,
            inputs.production_distro_packaging,
            "production distro packaging is separate from source readiness and is not claimed by default",
        ),
        gate(
            "reproducible_packaged_ebpf_object",
            false,
            inputs.reproducible_packaged_ebpf_object,
            "production distro packaging requires a reproducible packaged eBPF object build or release artifact path",
        ),
        gate(
            "packaging_install_tests",
            false,
            inputs.packaging_install_tests || inputs.packaged_artifact_layout_tests,
            "production distro packaging requires install tests for ebuild/PKGBUILD/tarball layout",
        ),
        gate(
            "packaging_service_smoke_tests",
            false,
            inputs.packaging_service_smoke_tests,
            "production distro packaging requires packaged service start/stop smoke tests",
        ),
        gate(
            "versioned_release_tarball",
            false,
            inputs.versioned_release_tarball,
            "production distro packaging requires versioned release tarballs/artifacts",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_stable_passes_only_without_apply_actions() {
        let mut inputs = ReleaseReadinessInputs {
            apply_actions_enabled: false,
            local_install_smoke_tests: true,
            service_doctor_smoke_tests: true,
            unprivileged_report_smoke_tests: true,
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
    fn low_risk_stable_requires_real_validation_matrix_and_false_negative_catalogue() {
        let inputs = ReleaseReadinessInputs {
            soak_tests: true,
            real_machine_validation: true,
            local_install_smoke_tests: true,
            emergency_restore_smoke_tests: true,
            service_start_stop_smoke_tests: true,
            rollback_drill: true,
            ..ReleaseReadinessInputs::default()
        };

        let report = evaluate_release_readiness(ReleaseChannel::LowRiskStable, &inputs);

        assert!(!report.passed);
        assert!(
            report
                .gates
                .iter()
                .any(|gate| gate.code == "real_validation_matrix" && !gate.passed)
        );
        assert!(
            report
                .gates
                .iter()
                .any(|gate| gate.code == "false_negative_catalogue" && !gate.passed)
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

    #[test]
    fn release_readiness_tracks_distro_packaging_as_advisory_by_default() {
        let report = evaluate_release_readiness(
            ReleaseChannel::Experimental,
            &ReleaseReadinessInputs::default(),
        );

        let gate = report
            .gates
            .iter()
            .find(|gate| gate.code == "production_distro_packaging")
            .expect("release report should track production distro packaging");

        assert!(!gate.required);
        assert!(!gate.passed);
        assert!(
            gate.description.contains("separate from source readiness"),
            "gate should clearly separate packaging from source readiness"
        );

        assert!(
            report.passed,
            "experimental source readiness should not fail because distro packaging is not claimed"
        );
    }

    #[test]
    fn release_readiness_lists_packaging_roadmap_gates() {
        let report = evaluate_release_readiness(
            ReleaseChannel::LowRiskStable,
            &ReleaseReadinessInputs::default(),
        );

        for code in [
            "production_distro_packaging",
            "reproducible_packaged_ebpf_object",
            "packaging_install_tests",
            "packaging_service_smoke_tests",
            "versioned_release_tarball",
        ] {
            let gate = report
                .gates
                .iter()
                .find(|gate| gate.code == code)
                .unwrap_or_else(|| panic!("missing packaging gate {code}"));

            assert!(
                !gate.required,
                "packaging roadmap gate {code} should be advisory"
            );
        }
    }

    #[test]
    fn release_readiness_can_mark_distro_packaging_gates_as_met() {
        let inputs = ReleaseReadinessInputs {
            production_distro_packaging: true,
            reproducible_packaged_ebpf_object: true,
            packaging_install_tests: true,
            packaging_service_smoke_tests: true,
            versioned_release_tarball: true,
            ..ReleaseReadinessInputs::default()
        };

        let report = evaluate_release_readiness(ReleaseChannel::Experimental, &inputs);

        for code in [
            "production_distro_packaging",
            "reproducible_packaged_ebpf_object",
            "packaging_install_tests",
            "packaging_service_smoke_tests",
            "versioned_release_tarball",
        ] {
            let gate = report
                .gates
                .iter()
                .find(|gate| gate.code == code)
                .unwrap_or_else(|| panic!("missing packaging gate {code}"));

            assert!(gate.passed, "packaging gate {code} should be marked met");
        }
    }
}
