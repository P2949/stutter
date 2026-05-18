#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        actions::{ActionId, SafetyClass},
        autotune::{
            candidate::CandidateAction,
            controller_journal::read_controller_journal,
            emergency_restore::{
                AutotuneRestoreCommandInput, AutotuneRestoreStatus, restore_known_autotune_actions,
            },
            runtime::{AutotuneRuntime, AutotuneRuntimeConfig},
            state::{ControllerPhase, SituationKind},
        },
        focus::FocusGroupKind,
        process_tree::{TaskClass, TaskInfo},
        session_events::{DropCountersSnapshot, IntervalRecord, MonitorEvent},
    };

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "stutter-autotune-lifecycle-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("failed to create lifecycle temp dir");
        path
    }

    fn game_task(tid: u32) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid: 1234,
            process_ppid: 1,
            comm: "lifecycle-game".to_owned(),
            process_comm: "lifecycle-game".into(),
            process_starttime_ticks: Some(10_000),
            task_starttime_ticks: Some(10_000 + u64::from(tid)),
            exe_dev: Some(1),
            exe_ino: Some(1234),
            class: TaskClass::Game,
            sched_policy: Some(0),
            from_cgroup: false,
        }
    }

    fn record(
        elapsed_ms: u64,
        samples: u64,
        over_1ms: u64,
        over_2ms: u64,
        over_5ms: u64,
        max_ns: u64,
    ) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms,
            task: 1234,
            active: true,
            class: TaskClass::Game,
            comm: "lifecycle-game".to_owned(),
            process_pid: Some(1234),
            process_comm: "lifecycle-game".into(),
            samples,
            stored_samples: samples,
            max_ns,
            over_1ms,
            over_2ms,
            over_5ms,
            percentile_scope: "task".to_owned(),
            ..IntervalRecord::default()
        }
    }

    fn records(
        start_elapsed_ms: u64,
        count: usize,
        samples: u64,
        over_1ms: u64,
        over_2ms: u64,
        over_5ms: u64,
        max_ns: u64,
    ) -> Vec<IntervalRecord> {
        (0..count)
            .map(|offset| {
                record(
                    start_elapsed_ms + (offset as u64 * 1_000),
                    samples,
                    over_1ms,
                    over_2ms,
                    over_5ms,
                    max_ns,
                )
            })
            .collect()
    }

    fn interval_event(records: Vec<IntervalRecord>) -> MonitorEvent {
        let elapsed_ms = records.last().map(|record| record.elapsed_ms).unwrap_or(0);
        MonitorEvent::Interval {
            elapsed_ms,
            records,
            drop_counters: DropCountersSnapshot::default(),
        }
    }

    #[tokio::test]
    async fn apply_low_risk_fake_candidate_lifecycle_keeps_and_cleans_journal() -> anyhow::Result<()>
    {
        let dir = temp_dir();
        let history_path = dir.join("autotune-history.jsonl");
        let audit_path = dir.join("audit.jsonl");
        let journal_path = dir.join("controller-journal.json");

        let candidate = CandidateAction::fake(
            ActionId("test-fake".to_owned()),
            SafetyClass::ReversibleLowRisk,
        );
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
            .with_simulated_candidates(vec![candidate])
            .with_simulated_action_effects()
            .with_candidate_window_seconds(1)
            .with_washout(0, 1);
        config.history_log = Some(history_path.clone());
        config.controller_journal_path = Some(journal_path.clone());

        let mut runtime = AutotuneRuntime::new(config);
        let mut active_targets = BTreeMap::new();
        active_targets.insert(1234, game_task(1234));

        runtime.on_event(MonitorEvent::TargetSnapshot {
            elapsed_ms: 0,
            active_targets,
            removed_targets: Vec::new(),
        })?;
        runtime.on_event(MonitorEvent::FocusChanged {
            elapsed_ms: 0,
            old_kind: None,
            new_kind: FocusGroupKind::Game,
            root_pids: vec![1234],
            member_pids: vec![1234],
            confidence: 0.95,
            score: 1.0,
            situation: SituationKind::GameCpuSchedulerPressure,
            reasons: vec!["lifecycle test game focus".to_owned()],
        })?;

        runtime.on_event(interval_event(records(1_000, 4, 25, 5, 5, 2, 8_000_000)))?;
        assert_eq!(runtime.controller_state().phase, ControllerPhase::Observing);

        let started = runtime
            .on_event(interval_event(records(5_000, 1, 25, 5, 5, 2, 8_000_000)))?
            .expect("baseline window should start a fake experiment");
        assert_eq!(started.decision, "candidate_started");
        assert_eq!(runtime.controller_state().phase, ControllerPhase::Measuring);
        assert!(runtime.has_active_experiment());

        let kept = runtime
            .on_event(interval_event(records(6_000, 5, 25, 0, 0, 0, 500_000)))?
            .expect("candidate measurement should keep the improved fake action");
        assert_eq!(kept.decision, "candidate_kept");
        assert_eq!(runtime.controller_state().phase, ControllerPhase::Cooldown);
        assert_eq!(runtime.active_profile_state().kept_action_count(), 1);

        for tick in 0..53 {
            runtime.on_event(interval_event(records(
                12_000 + (tick * 1_000),
                1,
                25,
                0,
                0,
                0,
                500_000,
            )))?;
        }

        assert_ne!(runtime.controller_state().phase, ControllerPhase::Faulted);
        assert!(history_path.exists());
        assert!(!fs::read_to_string(&history_path)?.trim().is_empty());

        let restore = restore_known_autotune_actions(AutotuneRestoreCommandInput {
            journal_path: Some(journal_path.clone()),
            audit_path: Some(audit_path),
            history_path: Some(history_path),
            dry_run: false,
        })?;
        assert_eq!(restore.status, AutotuneRestoreStatus::Restored);
        assert!(read_controller_journal(&journal_path)?.is_clean());

        Ok(())
    }
}
