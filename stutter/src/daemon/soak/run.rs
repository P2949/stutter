use super::model::*;
use crate::{
    actions::SafetyClass,
    autotune::simulation::{
        FakeDaemonScenario, FakeDaemonSimulationReport, FakeDaemonStep,
        run_fake_daemon_scenario_with_safety,
    },
    daemon::DaemonPhase,
};

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
            diagnostic_score_total,
            samples,
        } => FakeDaemonStep::Interval {
            diagnostic_raw_score_total: diagnostic_score_total,
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

pub fn low_data_quality_ticks(scenario: &SoakScenario) -> u64 {
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
            mode: crate::daemon::policy::DaemonMode::Observe,
            candidate_safety_class: SafetyClass::ReversibleLowRisk,
            ticks: vec![
                SoakTick::FocusGame { confidence: 0.95 },
                SoakTick::TargetPresent,
                SoakTick::Interval {
                    diagnostic_score_total: 500,
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
            mode: crate::daemon::policy::DaemonMode::ApplyLowRisk,
            candidate_safety_class: SafetyClass::ReversibleLowRisk,
            ticks: vec![
                SoakTick::FocusGame { confidence: 0.95 },
                SoakTick::TargetPresent,
                SoakTick::Interval {
                    diagnostic_score_total: 100,
                    samples: 100,
                },
                SoakTick::TargetPresent,
                SoakTick::Interval {
                    diagnostic_score_total: 1_000,
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
