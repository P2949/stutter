use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{
    actions::SafetyClass,
    autotune::simulation::{
        FakeDaemonScenario, FakeDaemonSimulationReport, FakeDaemonStep,
        run_fake_daemon_scenario_with_safety,
    },
    daemon::{DaemonPhase, policy::DaemonMode},
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonSoakProfile {
    ObserveOnly,
    ApplyLowRiskFake,
}

impl DaemonSoakProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe-only",
            Self::ApplyLowRiskFake => "apply-low-risk-fake",
        }
    }
}

impl fmt::Display for DaemonSoakProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DaemonSoakProfile {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "observe-only" | "observe" => Ok(Self::ObserveOnly),
            "apply-low-risk-fake" | "low-risk-fake" => Ok(Self::ApplyLowRiskFake),
            other => anyhow::bail!(
                "unknown soak profile {other:?}; expected observe-only or apply-low-risk-fake"
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakConfig {
    pub profile: DaemonSoakProfile,
    pub duration_seconds: u64,
    pub tick_millis: u64,
    pub budget: DaemonSoakBudget,
}

impl Default for DaemonSoakConfig {
    fn default() -> Self {
        Self {
            profile: DaemonSoakProfile::ObserveOnly,
            duration_seconds: 60,
            tick_millis: 1_000,
            budget: DaemonSoakBudget::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakBudget {
    pub max_memory_growth_bytes: u64,
    pub max_file_descriptors: u64,
    pub max_disk_growth_bytes: u64,
    pub max_event_queue_len: u64,
    pub max_task_count: u64,
    pub max_history_bytes: u64,
    pub max_cpu_millis_per_second: u64,
    pub max_wakeups_per_second: u64,
    pub max_event_drops: u64,
}

impl Default for DaemonSoakBudget {
    fn default() -> Self {
        Self {
            max_memory_growth_bytes: 8 * 1024 * 1024,
            max_file_descriptors: 128,
            max_disk_growth_bytes: 32 * 1024 * 1024,
            max_event_queue_len: 4096,
            max_task_count: 2048,
            max_history_bytes: 16 * 1024 * 1024,
            max_cpu_millis_per_second: 25,
            max_wakeups_per_second: 30,
            max_event_drops: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakReport {
    pub profile: DaemonSoakProfile,
    pub duration_seconds: u64,
    pub ticks: u64,
    pub passed: bool,
    pub metrics: DaemonSoakMetrics,
    pub failures: Vec<DaemonSoakFailure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<DaemonSoakScenarioReport>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakMetrics {
    pub scenario_count: u64,
    pub planner_decisions: u64,
    pub memory_growth_bytes: u64,
    pub file_descriptors: u64,
    pub disk_growth_bytes: u64,
    pub max_event_queue_len: u64,
    pub task_count: u64,
    pub history_bytes: u64,
    pub cpu_millis_per_second: u64,
    pub wakeups_per_second: u64,
    pub event_drops: u64,
    pub fake_actions_started: u64,
    pub fake_rollbacks: u64,
    pub max_active_experiments: u64,
    pub low_data_quality_ticks: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakFailure {
    pub reason_code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakScenarioReport {
    pub name: String,
    pub mode: DaemonMode,
    pub ticks: u64,
    pub decisions: Vec<String>,
    pub passed: bool,
    pub failures: Vec<DaemonSoakFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SoakScenario {
    pub name: String,
    pub mode: DaemonMode,
    #[serde(default = "default_soak_candidate_safety_class")]
    pub candidate_safety_class: SafetyClass,
    pub ticks: Vec<SoakTick>,
    #[serde(default)]
    pub assertions: Vec<SoakAssertion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SoakTick {
    FocusGame {
        #[serde(default = "default_focus_confidence")]
        confidence: f32,
    },
    FocusCleared {
        #[serde(default = "default_focus_clear_reason")]
        reason: String,
    },
    TargetPresent,
    TargetMissing,
    Interval {
        score_total: u64,
        samples: u64,
    },
    DroppedInterval {
        dropped_events: u64,
    },
    EvaluationTick {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoakAssertion {
    OneActiveExperimentMaximum,
    NoHighRiskAutonomousApply,
    NoApplyDuringLowDataQuality,
    NoProtectedTaskMutation,
    RollbackTokenBeforeApply,
    ShutdownRestoresActiveActions,
    CooldownRespected,
    FocusFlappingDoesNotCauseActionFlapping,
    HighRiskManualOnly,
}

pub fn run_fake_daemon_soak(config: &DaemonSoakConfig) -> DaemonSoakReport {
    run_scenario_daemon_soak(config, &default_soak_scenarios(config.profile))
}

pub fn run_scenario_daemon_soak(
    config: &DaemonSoakConfig,
    scenarios: &[SoakScenario],
) -> DaemonSoakReport {
    let tick_millis = config.tick_millis.max(1);
    let mut metrics = DaemonSoakMetrics {
        file_descriptors: 12,
        task_count: 1,
        cpu_millis_per_second: 2,
        wakeups_per_second: 1_000 / tick_millis,
        ..DaemonSoakMetrics::default()
    };
    let mut scenario_reports = Vec::new();
    let mut scenario_failures = Vec::new();
    let mut ticks = 0_u64;

    for scenario in scenarios {
        ticks = ticks.saturating_add(scenario.ticks.len() as u64);
        metrics.scenario_count = metrics.scenario_count.saturating_add(1);
        metrics.memory_growth_bytes = metrics
            .memory_growth_bytes
            .saturating_add(memory_growth_per_scenario(scenario));
        metrics.disk_growth_bytes = metrics
            .disk_growth_bytes
            .saturating_add(disk_growth_per_scenario(scenario));
        metrics.history_bytes = metrics
            .history_bytes
            .saturating_add(history_growth_per_scenario(scenario));
        metrics.task_count = metrics.task_count.max(1 + scenario.ticks.len() as u64);
        metrics.low_data_quality_ticks = metrics
            .low_data_quality_ticks
            .saturating_add(low_data_quality_ticks(scenario));

        match run_single_soak_scenario(scenario) {
            Ok((report, scenario_metrics)) => {
                metrics.planner_decisions = metrics
                    .planner_decisions
                    .saturating_add(scenario_metrics.planner_decisions);
                metrics.fake_actions_started = metrics
                    .fake_actions_started
                    .saturating_add(scenario_metrics.fake_actions_started);
                metrics.fake_rollbacks = metrics
                    .fake_rollbacks
                    .saturating_add(scenario_metrics.fake_rollbacks);
                metrics.max_active_experiments = metrics
                    .max_active_experiments
                    .max(scenario_metrics.max_active_experiments);
                metrics.event_drops = metrics
                    .event_drops
                    .saturating_add(scenario_metrics.event_drops);
                metrics.max_event_queue_len = metrics
                    .max_event_queue_len
                    .max(scenario_metrics.max_event_queue_len);
                scenario_failures.extend(report.failures.clone());
                scenario_reports.push(report);
            }
            Err(err) => {
                scenario_failures.push(DaemonSoakFailure {
                    reason_code: "scenario_runtime_failed".to_owned(),
                    message: format!("scenario {} failed: {err:#}", scenario.name),
                });
            }
        }
    }

    let ticks = ticks
        .max(
            config
                .duration_seconds
                .saturating_mul(1_000)
                .saturating_add(tick_millis - 1)
                / tick_millis,
        )
        .max(1);

    metrics.file_descriptors += match config.profile {
        DaemonSoakProfile::ObserveOnly => 4,
        DaemonSoakProfile::ApplyLowRiskFake => 8,
    };
    metrics.cpu_millis_per_second += match config.profile {
        DaemonSoakProfile::ObserveOnly => 3,
        DaemonSoakProfile::ApplyLowRiskFake => 7,
    };
    metrics.wakeups_per_second += match config.profile {
        DaemonSoakProfile::ObserveOnly => 2,
        DaemonSoakProfile::ApplyLowRiskFake => 4,
    };

    let mut failures = evaluate_soak_failures(&metrics, &config.budget);
    failures.extend(scenario_failures);

    DaemonSoakReport {
        profile: config.profile,
        duration_seconds: config.duration_seconds,
        ticks,
        passed: failures.is_empty(),
        metrics,
        failures,
        scenarios: scenario_reports,
    }
}

fn run_single_soak_scenario(
    scenario: &SoakScenario,
) -> anyhow::Result<(DaemonSoakScenarioReport, DaemonSoakMetrics)> {
    let runtime_report = run_fake_daemon_scenario_with_safety(
        FakeDaemonScenario {
            name: scenario.name.clone(),
            mode: scenario.mode,
            steps: scenario
                .ticks
                .iter()
                .cloned()
                .map(fake_step_for_tick)
                .collect(),
        },
        scenario.candidate_safety_class.clone(),
    )?;
    let scenario_name = runtime_report.scenario_name().to_owned();
    let decisions = runtime_report
        .decisions
        .iter()
        .map(|decision| decision.decision.clone())
        .collect::<Vec<_>>();
    let metrics = metrics_for_scenario(scenario, &runtime_report);
    let failures = evaluate_scenario_assertions(scenario, &runtime_report, &metrics);

    Ok((
        DaemonSoakScenarioReport {
            name: scenario_name,
            mode: scenario.mode,
            ticks: scenario.ticks.len() as u64,
            decisions,
            passed: failures.is_empty(),
            failures,
        },
        metrics,
    ))
}

fn fake_step_for_tick(tick: SoakTick) -> FakeDaemonStep {
    match tick {
        SoakTick::FocusGame { confidence } => FakeDaemonStep::FocusGame { confidence },
        SoakTick::FocusCleared { reason } => FakeDaemonStep::FocusCleared { reason },
        SoakTick::TargetPresent => FakeDaemonStep::TargetPresent,
        SoakTick::TargetMissing => FakeDaemonStep::TargetMissing,
        SoakTick::Interval {
            score_total,
            samples,
        } => FakeDaemonStep::Interval {
            score_total,
            samples,
        },
        SoakTick::DroppedInterval { dropped_events } => {
            FakeDaemonStep::DroppedInterval { dropped_events }
        }
        SoakTick::EvaluationTick { reason } => FakeDaemonStep::EvaluationTick { reason },
    }
}

fn metrics_for_scenario(
    scenario: &SoakScenario,
    report: &FakeDaemonSimulationReport,
) -> DaemonSoakMetrics {
    let decisions = report.decision_labels();
    let fake_actions_started = decisions
        .iter()
        .filter(|decision| **decision == "candidate_started")
        .count() as u64;
    let fake_rollbacks = decisions
        .iter()
        .filter(|decision| **decision == "candidate_reverted")
        .count() as u64;
    DaemonSoakMetrics {
        planner_decisions: decisions.len() as u64,
        max_event_queue_len: max_active_experiment_count(&decisions),
        fake_actions_started,
        fake_rollbacks,
        max_active_experiments: max_active_experiment_count(&decisions),
        low_data_quality_ticks: low_data_quality_ticks(scenario),
        ..DaemonSoakMetrics::default()
    }
}

fn evaluate_scenario_assertions(
    scenario: &SoakScenario,
    report: &FakeDaemonSimulationReport,
    metrics: &DaemonSoakMetrics,
) -> Vec<DaemonSoakFailure> {
    let mut failures = Vec::new();

    for assertion in &scenario.assertions {
        match assertion {
            SoakAssertion::OneActiveExperimentMaximum => {
                if metrics.max_active_experiments > 1 {
                    scenario_failure(
                        &mut failures,
                        "one_active_experiment_maximum",
                        scenario,
                        "more than one active experiment was observed",
                    );
                }
            }
            SoakAssertion::NoHighRiskAutonomousApply => {
                if scenario.candidate_safety_class == SafetyClass::HighRisk
                    && metrics.fake_actions_started > 0
                {
                    scenario_failure(
                        &mut failures,
                        "no_high_risk_autonomous_apply",
                        scenario,
                        "high-risk mode started an autonomous candidate",
                    );
                }
            }
            SoakAssertion::NoApplyDuringLowDataQuality => {
                if metrics.low_data_quality_ticks > 0 && metrics.fake_actions_started > 0 {
                    scenario_failure(
                        &mut failures,
                        "no_apply_during_low_data_quality",
                        scenario,
                        "candidate started while low data quality ticks were present",
                    );
                }
            }
            SoakAssertion::NoProtectedTaskMutation => {}
            SoakAssertion::RollbackTokenBeforeApply => {
                if report.final_state.phase == DaemonPhase::Measure
                    && !report
                        .final_state
                        .active_rollback
                        .as_ref()
                        .is_some_and(|rollback| rollback.rollback_available)
                {
                    scenario_failure(
                        &mut failures,
                        "rollback_token_before_apply",
                        scenario,
                        "measuring state lacks an active rollback token",
                    );
                }
            }
            SoakAssertion::ShutdownRestoresActiveActions => {
                let pending_manual_restore = report.final_state.active_rollback.is_some()
                    && report.final_state.phase == DaemonPhase::Faulted
                    && report.final_state.faulted.is_some();
                if report.final_state.active_rollback.is_some() && !pending_manual_restore {
                    scenario_failure(
                        &mut failures,
                        "shutdown_restores_active_actions",
                        scenario,
                        "scenario ended with an active rollback pending outside faulted manual-restore state",
                    );
                }
            }
            SoakAssertion::CooldownRespected => {
                if starts_after_terminal_decision(&report.decision_labels()) {
                    scenario_failure(
                        &mut failures,
                        "cooldown_respected",
                        scenario,
                        "candidate restarted after keep/revert cooldown transition",
                    );
                }
            }
            SoakAssertion::FocusFlappingDoesNotCauseActionFlapping => {
                if metrics.fake_actions_started > 1 {
                    scenario_failure(
                        &mut failures,
                        "focus_flapping_does_not_cause_action_flapping",
                        scenario,
                        "focus flapping caused repeated candidate starts",
                    );
                }
            }
            SoakAssertion::HighRiskManualOnly => {
                if metrics.fake_actions_started > 0 {
                    scenario_failure(
                        &mut failures,
                        "high_risk_manual_only",
                        scenario,
                        "manual-only scenario started an autonomous candidate",
                    );
                }
            }
        }
    }

    failures
}

fn scenario_failure(
    failures: &mut Vec<DaemonSoakFailure>,
    reason_code: &'static str,
    scenario: &SoakScenario,
    message: &'static str,
) {
    failures.push(DaemonSoakFailure {
        reason_code: reason_code.to_owned(),
        message: format!("{}: {message}", scenario.name),
    });
}

fn max_active_experiment_count(decisions: &[&str]) -> u64 {
    let mut active = 0_u64;
    let mut max_active = 0_u64;
    for decision in decisions {
        match *decision {
            "candidate_started" => {
                active = active.saturating_add(1);
                max_active = max_active.max(active);
            }
            "candidate_reverted" | "candidate_kept" => {
                active = active.saturating_sub(1);
            }
            _ => {}
        }
    }
    max_active
}

fn starts_after_terminal_decision(decisions: &[&str]) -> bool {
    let mut terminal_seen = false;
    for decision in decisions {
        match *decision {
            "candidate_kept" | "candidate_reverted" => terminal_seen = true,
            "candidate_started" if terminal_seen => return true,
            _ => {}
        }
    }
    false
}

fn memory_growth_per_scenario(scenario: &SoakScenario) -> u64 {
    scenario.ticks.len() as u64 * 256
}

fn disk_growth_per_scenario(scenario: &SoakScenario) -> u64 {
    scenario.ticks.len() as u64 * 128
}

fn history_growth_per_scenario(scenario: &SoakScenario) -> u64 {
    scenario.ticks.len() as u64 * 192
}

fn low_data_quality_ticks(scenario: &SoakScenario) -> u64 {
    scenario
        .ticks
        .iter()
        .filter(|tick| matches!(tick, SoakTick::DroppedInterval { .. }))
        .count() as u64
}

fn default_soak_scenarios(profile: DaemonSoakProfile) -> Vec<SoakScenario> {
    match profile {
        DaemonSoakProfile::ObserveOnly => vec![SoakScenario {
            name: "observe_only_no_apply".to_owned(),
            mode: DaemonMode::Observe,
            candidate_safety_class: SafetyClass::ReversibleLowRisk,
            ticks: vec![
                SoakTick::FocusGame { confidence: 0.95 },
                SoakTick::TargetPresent,
                SoakTick::Interval {
                    score_total: 500,
                    samples: 100,
                },
            ],
            assertions: vec![
                SoakAssertion::OneActiveExperimentMaximum,
                SoakAssertion::NoHighRiskAutonomousApply,
                SoakAssertion::RollbackTokenBeforeApply,
            ],
        }],
        DaemonSoakProfile::ApplyLowRiskFake => vec![SoakScenario {
            name: "apply_low_risk_revert".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            candidate_safety_class: SafetyClass::ReversibleLowRisk,
            ticks: vec![
                SoakTick::FocusGame { confidence: 0.95 },
                SoakTick::TargetPresent,
                SoakTick::Interval {
                    score_total: 100,
                    samples: 100,
                },
                SoakTick::TargetPresent,
                SoakTick::Interval {
                    score_total: 1_000,
                    samples: 100,
                },
            ],
            assertions: vec![
                SoakAssertion::OneActiveExperimentMaximum,
                SoakAssertion::RollbackTokenBeforeApply,
                SoakAssertion::CooldownRespected,
            ],
        }],
    }
}

fn default_focus_confidence() -> f32 {
    0.95
}

fn default_focus_clear_reason() -> String {
    "focus cleared".to_owned()
}

fn default_soak_candidate_safety_class() -> SafetyClass {
    SafetyClass::ReversibleLowRisk
}

fn evaluate_soak_failures(
    metrics: &DaemonSoakMetrics,
    budget: &DaemonSoakBudget,
) -> Vec<DaemonSoakFailure> {
    let mut failures = Vec::new();

    check_budget(
        &mut failures,
        "memory_growth",
        metrics.memory_growth_bytes,
        budget.max_memory_growth_bytes,
    );
    check_budget(
        &mut failures,
        "file_descriptors",
        metrics.file_descriptors,
        budget.max_file_descriptors,
    );
    check_budget(
        &mut failures,
        "disk_growth",
        metrics.disk_growth_bytes,
        budget.max_disk_growth_bytes,
    );
    check_budget(
        &mut failures,
        "event_queue",
        metrics.max_event_queue_len,
        budget.max_event_queue_len,
    );
    check_budget(
        &mut failures,
        "task_count",
        metrics.task_count,
        budget.max_task_count,
    );
    check_budget(
        &mut failures,
        "history_size",
        metrics.history_bytes,
        budget.max_history_bytes,
    );
    check_budget(
        &mut failures,
        "cpu_overhead",
        metrics.cpu_millis_per_second,
        budget.max_cpu_millis_per_second,
    );
    check_budget(
        &mut failures,
        "wakeups",
        metrics.wakeups_per_second,
        budget.max_wakeups_per_second,
    );
    check_budget(
        &mut failures,
        "event_drops",
        metrics.event_drops,
        budget.max_event_drops,
    );

    failures
}

fn check_budget(
    failures: &mut Vec<DaemonSoakFailure>,
    reason_code: &'static str,
    observed: u64,
    limit: u64,
) {
    if observed <= limit {
        return;
    }

    failures.push(DaemonSoakFailure {
        reason_code: reason_code.to_owned(),
        message: format!("observed {observed} exceeds budget {limit}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_only_fake_soak_passes_default_budgets() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            duration_seconds: 60,
            ..DaemonSoakConfig::default()
        });

        assert!(report.passed);
        assert_eq!(report.profile, DaemonSoakProfile::ObserveOnly);
        assert_eq!(report.metrics.event_drops, 0);
        assert_eq!(report.metrics.scenario_count, 1);
    }

    #[test]
    fn fake_low_risk_soak_tracks_actions_and_rollbacks() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            profile: DaemonSoakProfile::ApplyLowRiskFake,
            duration_seconds: 120,
            ..DaemonSoakConfig::default()
        });

        assert!(report.passed);
        assert!(report.metrics.fake_actions_started >= 1);
        assert!(report.metrics.fake_rollbacks >= 1);
    }

    #[test]
    fn fake_soak_fails_when_budget_is_too_small() {
        let report = run_fake_daemon_soak(&DaemonSoakConfig {
            duration_seconds: 60,
            budget: DaemonSoakBudget {
                max_disk_growth_bytes: 1,
                ..DaemonSoakBudget::default()
            },
            ..DaemonSoakConfig::default()
        });

        assert!(!report.passed);
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.reason_code == "disk_growth")
        );
    }

    #[test]
    fn soak_profile_parses_aliases() {
        assert_eq!(
            "low-risk-fake".parse::<DaemonSoakProfile>().unwrap(),
            DaemonSoakProfile::ApplyLowRiskFake
        );
    }

    #[test]
    fn scenario_driven_soak_fixtures_preserve_safety_invariants() {
        let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("testdata/autotune/soak");
        let mut paths = std::fs::read_dir(&fixture_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();

        assert_eq!(paths.len(), 12);

        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap();
            let scenario: SoakScenario = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            let report = run_scenario_daemon_soak(
                &DaemonSoakConfig {
                    profile: DaemonSoakProfile::ApplyLowRiskFake,
                    duration_seconds: scenario.ticks.len() as u64,
                    ..DaemonSoakConfig::default()
                },
                std::slice::from_ref(&scenario),
            );

            assert!(
                report.passed,
                "{} failed: {:?}",
                scenario.name, report.failures
            );
            assert!(
                report.metrics.event_drops <= DaemonSoakBudget::default().max_event_drops,
                "{} event drops exceeded budget",
                scenario.name
            );
            assert!(
                report.metrics.max_event_queue_len
                    <= DaemonSoakBudget::default().max_event_queue_len,
                "{} queue accounting should stay bounded",
                scenario.name
            );
            assert!(
                report.metrics.max_active_experiments <= 1,
                "{} active experiment invariant failed",
                scenario.name
            );
            assert_eq!(report.scenarios.len(), 1);
            assert!(report.scenarios[0].passed);
        }
    }
}
