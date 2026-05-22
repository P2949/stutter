use super::*;

pub(super) fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-autotune-status-test-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn score(total: u64) -> crate::autotune::experiment::WindowScore {
    crate::autotune::experiment::WindowScore {
        started_unix_nanos: 100,
        finished_unix_nanos: 200,
        interval_count: 10,
        scored_samples: 100,
        scored_task_count: 2,
        score: StutterScore {
            total,
            ..StutterScore::default()
        },
    }
}

pub(super) fn observation() -> ObservationSummary {
    ObservationSummary {
        target_present: true,
        active_target_count: 31,
        scored_task_count: 2,
        interval_count: 10,
        scored_samples: 100,
        score_total: 818,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: 0,
        frame_p99_ms: 12.0,
        frame_max_ms: 20.0,
        drop_counter_total: 0,
        data_quality: "High".to_owned(),
    }
}

pub(super) fn target() -> TargetIdentity {
    TargetIdentity {
        root_pid: 1234,
        process_comm: "KingdomCome.exe".to_owned(),
        process_starttime_ticks: Some(99),
        exe_dev: Some(1),
        exe_ino: Some(2),
        active_task_count: 31,
    }
}

pub(super) fn kept_event() -> AutotuneHistoryEvent {
    AutotuneHistoryEvent {
        schema_version: 1,
        unix_nanos: 1,
        controller_id: "controller-1".to_owned(),
        phase: ControllerPhase::Cooldown,
        mode: AutotuneMode::ApplyLowRisk,
        target: Some(target()),
        situation: SituationKind::GameCpuSchedulerPressure,
        observation_summary: observation(),
        decision: AutotuneDecisionSummary {
            decision: "KeepCurrent".to_owned(),
            candidate_name: Some("game-main-suggested".to_owned()),
            action_kind: Some("cpu_affinity_profile".to_owned()),
            safety_class: Some(SafetyClass::ReversibleLowRisk),
            eligible: true,
            rollback_policy: "rollback-on-exit".to_owned(),
        },
        experiment_id: Some("experiment-1".to_owned()),
        action_id: Some("cpu-affinity-profile:game-main-suggested".to_owned()),
        score_before: Some(score(1_000)),
        score_after: Some(score(818)),
        planner: None,
        rollback_performed: false,
        reason: "candidate improved by 18.20%; kept as current active profile".to_owned(),
    }
}
