use std::{collections::BTreeMap, sync::Arc};

use crate::{
    actions::{ActionId, SafetyClass},
    autotune::{
        candidate::CandidateAction,
        quality::OnlineDataQualityPolicy,
        runtime::{
            AutotuneDecisionStreamEntry, AutotuneRuntime, AutotuneRuntimeConfig,
            daemon_config_for_runtime_mode,
        },
        state::SituationKind,
    },
    daemon::{
        policy::{ActionSource, DaemonMode},
        state::DaemonState,
    },
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::{TaskClass, TaskInfo},
    recorder::IntervalRecord,
    session_events::MonitorEvent,
};

#[derive(Clone, Debug)]
pub enum FakeDaemonStep {
    FocusGame { confidence: f32 },
    FocusCleared { reason: String },
    TargetPresent,
    TargetMissing,
    Interval { score_total: u64, samples: u64 },
    DroppedInterval { dropped_events: u64 },
    EvaluationTick { reason: String },
}

#[derive(Clone, Debug)]
pub struct FakeDaemonScenario {
    pub name: String,
    pub mode: DaemonMode,
    pub steps: Vec<FakeDaemonStep>,
}

#[derive(Clone, Debug)]
pub struct FakeDaemonSimulationReport {
    pub name: String,
    pub decisions: Vec<AutotuneDecisionStreamEntry>,
    pub final_state: DaemonState,
}

impl FakeDaemonSimulationReport {
    pub fn scenario_name(&self) -> &str {
        &self.name
    }

    pub fn decision_labels(&self) -> Vec<&str> {
        self.decisions
            .iter()
            .map(|decision| decision.decision.as_str())
            .collect()
    }

    pub fn contains_decision(&self, label: &str) -> bool {
        self.decisions
            .iter()
            .any(|decision| decision.decision == label)
    }
}

pub fn run_fake_daemon_scenario(
    scenario: FakeDaemonScenario,
) -> anyhow::Result<FakeDaemonSimulationReport> {
    run_fake_daemon_scenario_with_safety(scenario, SafetyClass::ReversibleLowRisk)
}

pub fn run_fake_daemon_scenario_with_safety(
    scenario: FakeDaemonScenario,
    candidate_safety_class: SafetyClass,
) -> anyhow::Result<FakeDaemonSimulationReport> {
    let mut runtime = AutotuneRuntime::new(simulation_runtime_config(
        scenario.mode,
        candidate_safety_class,
    ));
    let mut elapsed_ms = 0_u64;
    let mut decisions = Vec::new();

    for step in scenario.steps {
        elapsed_ms = elapsed_ms.saturating_add(1_000);
        if let Some(event) = event_for_step(step, elapsed_ms)
            && let Some(decision) = runtime.on_event(event)?
        {
            decisions.push(decision);
        }
    }

    Ok(FakeDaemonSimulationReport {
        name: scenario.name,
        decisions,
        final_state: runtime.daemon_state_snapshot(),
    })
}

#[cfg(test)]
pub fn assert_no_apply_without_rollback(report: &FakeDaemonSimulationReport) {
    if report.contains_decision("candidate_started") {
        assert!(
            report
                .final_state
                .active_rollback
                .as_ref()
                .is_some_and(|rollback| rollback.rollback_available),
            "scenario {} started a candidate without rollback availability",
            report.name
        );
    }
}

fn simulation_runtime_config(
    mode: DaemonMode,
    candidate_safety_class: SafetyClass,
) -> AutotuneRuntimeConfig {
    let mut base = AutotuneRuntimeConfig::from_daemon_config(
        daemon_config_for_runtime_mode(mode, ActionSource::Test, Some(1234), None),
        None,
    );
    base.history_log = None;

    base.with_online_data_quality_policy(OnlineDataQualityPolicy {
        min_scored_intervals: 1,
        min_scored_samples: 1,
        max_drop_counter_total: 0,
        ..OnlineDataQualityPolicy::default()
    })
    .with_min_focus_confidence(0.70)
    .with_simulated_candidates(vec![CandidateAction::fake(
        ActionId::new("cpu-affinity-profile:simulation-low-risk".to_owned()),
        candidate_safety_class,
    )])
    .with_simulated_action_effects()
}

fn event_for_step(step: FakeDaemonStep, elapsed_ms: u64) -> Option<MonitorEvent> {
    match step {
        FakeDaemonStep::FocusGame { confidence } => Some(MonitorEvent::FocusChanged {
            elapsed_ms,
            old_kind: None,
            new_kind: FocusGroupKind::Game,
            root_pids: vec![1234],
            member_pids: vec![1234],
            confidence,
            score: confidence,
            situation: SituationKind::GameCpuSchedulerPressure,
            reasons: vec!["simulation game focus".to_owned()],
        }),
        FakeDaemonStep::FocusCleared { reason } => Some(MonitorEvent::FocusCleared {
            elapsed_ms,
            old_kind: Some(FocusGroupKind::Game),
            reason,
        }),
        FakeDaemonStep::TargetPresent => Some(MonitorEvent::TargetSnapshot {
            elapsed_ms,
            active_targets: BTreeMap::from([(1234, task_info())]),
            removed_targets: Vec::new(),
        }),
        FakeDaemonStep::TargetMissing => Some(MonitorEvent::TargetSnapshot {
            elapsed_ms,
            active_targets: BTreeMap::new(),
            removed_targets: vec![1234],
        }),
        FakeDaemonStep::Interval {
            score_total,
            samples,
        } => Some(MonitorEvent::Interval {
            elapsed_ms,
            records: vec![interval_record(elapsed_ms, score_total, samples)],
            drop_counters: DropCountersSnapshot::default(),
        }),
        FakeDaemonStep::DroppedInterval { dropped_events } => Some(MonitorEvent::Interval {
            elapsed_ms,
            records: vec![interval_record_with_drops(
                elapsed_ms,
                100,
                100,
                dropped_events,
            )],
            drop_counters: DropCountersSnapshot {
                ringbuf_reserve_failed: dropped_events,
                ..DropCountersSnapshot::default()
            },
        }),
        FakeDaemonStep::EvaluationTick { reason } => {
            Some(MonitorEvent::DataQualityWarning { message: reason })
        }
    }
}

fn task_info() -> TaskInfo {
    TaskInfo {
        tid: 1234,
        process_pid: 1234,
        process_ppid: 1,
        comm: "simulation-game".to_owned(),
        process_comm: Arc::from("simulation-game"),
        process_starttime_ticks: Some(10),
        task_starttime_ticks: Some(10),
        exe_dev: Some(1),
        exe_ino: Some(2),
        class: TaskClass::Game,
        sched_policy: Some(0),
        from_cgroup: false,
    }
}

fn interval_record(elapsed_ms: u64, score_total: u64, samples: u64) -> IntervalRecord {
    interval_record_with_drops(elapsed_ms, score_total, samples, 0)
}

fn interval_record_with_drops(
    elapsed_ms: u64,
    score_total: u64,
    samples: u64,
    dropped_events: u64,
) -> IntervalRecord {
    let over_5ms = score_total / 100;
    let remainder = score_total % 100;
    let over_2ms = remainder / 20;
    let over_1ms = remainder % 20;

    IntervalRecord {
        elapsed_ms,
        task: 1234,
        active: true,
        class: TaskClass::Game,
        comm: "simulation-game".to_owned(),
        process_pid: Some(1234),
        process_comm: Arc::from("simulation-game"),
        samples,
        stored_samples: samples,
        max_ns: if score_total > 0 { 6_000_000 } else { 500_000 },
        over_1ms,
        over_2ms,
        over_5ms,
        drop_counters: DropCountersSnapshot {
            ringbuf_reserve_failed: dropped_events,
            ..DropCountersSnapshot::default()
        },
        ..IntervalRecord::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::state::DaemonPhase;

    fn standard_prefix() -> Vec<FakeDaemonStep> {
        vec![
            FakeDaemonStep::FocusGame { confidence: 0.95 },
            FakeDaemonStep::TargetPresent,
        ]
    }

    #[test]
    fn simulation_noops_when_data_is_insufficient() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::EvaluationTick {
            reason: "simulation tick".to_owned(),
        });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "insufficient-data".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps,
        })
        .unwrap();

        assert!(!report.contains_decision("candidate_started"));
        assert!(report.contains_decision("observed"));
        assert_eq!(report.final_state.phase, DaemonPhase::Observe);
    }

    #[test]
    fn simulation_suggests_candidate_without_starting_experiment() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::Interval {
            score_total: 500,
            samples: 100,
        });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "suggest-stable-game".to_owned(),
            mode: DaemonMode::Suggest,
            steps,
        })
        .unwrap();

        assert!(report.contains_decision("suggested"));
        assert!(!report.contains_decision("candidate_started"));
        assert!(report.final_state.active_rollback.is_none());
    }

    #[test]
    fn simulation_starts_low_risk_candidate_with_rollback() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::Interval {
            score_total: 500,
            samples: 100,
        });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "start-low-risk".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps,
        })
        .unwrap();

        assert!(report.contains_decision("candidate_started"));
        assert_eq!(report.final_state.phase, DaemonPhase::Measure);
        assert_no_apply_without_rollback(&report);
    }

    #[test]
    fn simulation_keeps_improved_candidate() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::Interval {
            score_total: 1_000,
            samples: 100,
        });
        steps.push(FakeDaemonStep::TargetPresent);
        steps.push(FakeDaemonStep::Interval {
            score_total: 10,
            samples: 100,
        });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "keep-improved".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps,
        })
        .unwrap();

        assert!(report.contains_decision("candidate_started"));
        assert!(report.contains_decision("candidate_kept"));
        assert_eq!(report.final_state.phase, DaemonPhase::Cooldown);
        assert!(report.final_state.active_rollback.is_none());
    }

    #[test]
    fn simulation_reverts_regressed_candidate() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::Interval {
            score_total: 10,
            samples: 100,
        });
        steps.push(FakeDaemonStep::TargetPresent);
        steps.push(FakeDaemonStep::Interval {
            score_total: 1_000,
            samples: 100,
        });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "revert-regressed".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps,
        })
        .unwrap();

        assert!(report.contains_decision("candidate_started"));
        assert!(report.contains_decision("candidate_reverted"));
        assert_eq!(report.final_state.phase, DaemonPhase::Cooldown);
        assert!(report.final_state.active_rollback.is_none());
    }

    #[test]
    fn simulation_rolls_back_on_focus_switch() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::Interval {
            score_total: 500,
            samples: 100,
        });
        steps.push(FakeDaemonStep::FocusCleared {
            reason: "simulation focus switch".to_owned(),
        });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "rollback-focus-switch".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps,
        })
        .unwrap();

        assert!(report.contains_decision("candidate_started"));
        assert_eq!(report.final_state.phase, DaemonPhase::Cooldown);
        assert!(report.final_state.active_rollback.is_none());
    }

    #[test]
    fn simulation_rolls_back_when_target_disappears() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::Interval {
            score_total: 500,
            samples: 100,
        });
        steps.push(FakeDaemonStep::TargetMissing);
        steps.push(FakeDaemonStep::EvaluationTick {
            reason: "simulation target disappeared".to_owned(),
        });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "rollback-target-disappeared".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps,
        })
        .unwrap();

        assert!(report.contains_decision("candidate_started"));
        assert!(report.contains_decision("candidate_reverted"));
        assert_eq!(report.final_state.phase, DaemonPhase::Cooldown);
        assert!(report.final_state.active_rollback.is_none());
    }

    #[test]
    fn simulation_drop_counters_block_apply() {
        let mut steps = standard_prefix();
        steps.push(FakeDaemonStep::DroppedInterval { dropped_events: 1 });

        let report = run_fake_daemon_scenario(FakeDaemonScenario {
            name: "drop-counters".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            steps,
        })
        .unwrap();

        assert!(!report.contains_decision("candidate_started"));
        assert!(report.final_state.health.reason_code.as_deref().is_some());
    }

    #[test]
    fn simulation_property_corpus_never_leaves_stale_active_rollback() {
        for seed in 0..16_u64 {
            let mut steps = standard_prefix();
            steps.push(FakeDaemonStep::Interval {
                score_total: if seed % 2 == 0 { 500 } else { 50 },
                samples: 100,
            });

            match seed % 4 {
                0 => {
                    steps.push(FakeDaemonStep::TargetPresent);
                    steps.push(FakeDaemonStep::Interval {
                        score_total: 5,
                        samples: 100,
                    });
                }
                1 => {
                    steps.push(FakeDaemonStep::TargetPresent);
                    steps.push(FakeDaemonStep::Interval {
                        score_total: 1_500,
                        samples: 100,
                    });
                }
                2 => {
                    steps.push(FakeDaemonStep::TargetMissing);
                    steps.push(FakeDaemonStep::EvaluationTick {
                        reason: format!("target disappeared seed={seed}"),
                    });
                }
                _ => {
                    steps.push(FakeDaemonStep::DroppedInterval { dropped_events: 1 });
                }
            }

            let report = run_fake_daemon_scenario(FakeDaemonScenario {
                name: format!("property-seed-{seed}"),
                mode: DaemonMode::ApplyLowRisk,
                steps,
            })
            .unwrap();

            if report.contains_decision("candidate_kept") {
                assert!(report.contains_decision("candidate_started"));
                assert!(report.final_state.active_rollback.is_none());
            }

            if report.contains_decision("candidate_reverted") {
                assert_eq!(report.final_state.phase, DaemonPhase::Cooldown);
                assert!(report.final_state.active_rollback.is_none());
            }

            if report.final_state.phase == DaemonPhase::Measure {
                assert!(
                    report
                        .final_state
                        .active_rollback
                        .as_ref()
                        .is_some_and(|rollback| rollback.rollback_available),
                    "measuring scenario {} lacks rollback",
                    report.name
                );
            }
        }
    }
}
