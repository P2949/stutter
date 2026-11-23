use super::*;

#[test]
fn top_denied_reason_for_plan_prefers_deny_reason_enum() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-noop".to_owned()),
        SafetyClass::ObserveOnly,
    );
    let descriptor = candidate.descriptor();
    let evaluation = crate::autotune::planner::CandidateEvaluation {
        candidate_name: "fake-noop".to_owned(),
        action_kind: "fake".to_owned(),
        descriptor,
        provider: "test".to_owned(),
        confidence: 1.0,
        eligible: false,
        deny_reasons: vec![crate::autotune::planner::CandidateDenyReason::NoEffectiveChange],
        deny_messages: vec!["candidate would not change active configuration".to_owned()],
        evidence: Vec::new(),
        objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
        rank: Some(1),
        dry_run: None,
        candidate,
    };
    let plan = PlanResult {
        selected: None,
        evaluations: vec![evaluation],
        no_action_reason: None,
    };

    assert_eq!(
        top_denied_reason_for_plan(&plan).as_deref(),
        Some("NoEffectiveChange")
    );
}

#[test]
fn admitted_research_reserves_the_action_slot_but_retains_candidate_diagnostics() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("research-slot-candidate".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_candidates(vec![candidate]);
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    assert!(
        runtime
            .select_candidate_for_normal_control(&observation)
            .unwrap()
            .is_some()
    );
    runtime.research_campaign_active = true;
    let selected = runtime
        .select_candidate_for_normal_control(&observation)
        .unwrap();
    assert!(selected.is_none());
    assert!(
        runtime
            .last_plan_result
            .as_ref()
            .unwrap()
            .selected
            .is_some()
    );
    let decision = decide_autotune_transition(
        &runtime.controller.policy,
        &runtime.controller.state,
        &observation,
        selected,
    );
    assert!(!matches!(
        decision,
        AutotuneDecision::StartExperiment { .. }
    ));
    runtime
        .apply_decision_side_effects(&observation, &decision, "research owns action slot")
        .unwrap();
    assert!(!runtime.has_active_experiment());
    runtime.research_campaign_active = false;
    assert!(
        runtime
            .select_candidate_for_normal_control(&observation)
            .unwrap()
            .is_some()
    );
}

mod research_action_gate {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{
        affinity::CpuMask,
        autotune::research::{
            ResearchAction,
            action_child::ActionChildScope,
            action_gate::{ActionGate, process_identity},
        },
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
    };

    fn profile() -> Profile {
        Profile {
            name: "research-gate".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn runtime_for(pid: u32, candidates: Vec<CandidateAction>) -> AutotuneRuntime {
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(pid), None)
            .with_simulated_candidates(candidates);
        config.history_log = None;
        let mut runtime = AutotuneRuntime::new(config);
        runtime.research_campaign_active = true;
        runtime
    }

    fn gate(actions: &[ResearchAction], pid: u32) -> Arc<Mutex<ActionGate>> {
        let target = process_identity(pid).expect("a live process");
        let until = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 600;
        Arc::new(Mutex::new(ActionGate::new(
            ActionChildScope::new(actions.iter().copied(), target, until),
            Box::new(|| Some(0)),
        )))
    }

    #[test]
    fn an_installed_gate_lets_an_authorized_game_action_through_and_nothing_else() {
        let pid = std::process::id();
        let cpu = CandidateAction::cpu_affinity_profile(profile(), pid);
        let fake = CandidateAction::fake(
            crate::actions::ActionId::new("not-a-game-action".to_owned()),
            SafetyClass::ReversibleLowRisk,
        );

        // No gate: nothing is ever permitted, the blanket suppression it always was.
        let mut blanket = runtime_for(pid, vec![cpu.clone()]);
        assert!(!blanket.research_gate_permits(&cpu));

        // A gate that authorizes this action on this process permits the candidate, once.
        let mut gated = runtime_for(pid, vec![cpu.clone()]);
        let shared = gate(&[ResearchAction::CpuAffinityProfile], pid);
        gated.set_research_action_gate(Some(shared.clone()));
        assert!(gated.research_gate_permits(&cpu));
        // One at a time: while it is outstanding, the same or another candidate is refused.
        assert!(!gated.research_gate_permits(&cpu));
        // The event ended with no experiment applied: the gate settles with nothing to restore and
        // the refused action is not retried.
        gated.observe_research_gate();
        let evidence = shared.lock().unwrap().evidence();
        assert!(evidence.clean && evidence.settled.is_empty());
        assert!(!gated.research_gate_permits(&cpu));

        // An action the campaign did not authorize, and a non-game candidate, stay suppressed.
        let mut other = runtime_for(pid, vec![cpu.clone()]);
        other.set_research_action_gate(Some(gate(&[ResearchAction::CgroupPlacement], pid)));
        assert!(!other.research_gate_permits(&cpu));
        let mut not_game = runtime_for(pid, vec![fake.clone()]);
        not_game.set_research_action_gate(Some(gate(&[ResearchAction::CpuAffinityProfile], pid)));
        assert!(!not_game.research_gate_permits(&fake));
    }

    #[test]
    fn the_runtime_reports_its_interval_and_gpu_events_into_the_resident_observer() {
        use crate::{
            autotune::research::{
                action_child::ChildTarget,
                resident_observer::{ResidentObserver, SharedResident},
            },
            metrics::{IntervalRecord, LatencyHistogram},
            process_tree::TaskClass,
            session_events::MonitorEvent,
        };
        let pid = std::process::id();
        let identity = process_identity(pid).expect("a live process");
        let mut runtime = runtime_for(pid, vec![]);
        let resident: SharedResident = Arc::new(Mutex::new(ResidentObserver::default()));
        runtime.set_research_resident(Some(resident.clone()));
        let game = |events: u64| {
            let mut histogram = LatencyHistogram::new();
            for _ in 0..events {
                histogram.record(500_000);
            }
            IntervalRecord {
                process_pid: Some(pid),
                class: TaskClass::Game,
                samples: events,
                max_ns: 500_000,
                histogram: histogram.snapshot(),
                ..Default::default()
            }
        };
        // Nothing attested yet: nothing is counted.
        runtime.observe_research_intervals(&[game(10)], 0);
        resident
            .lock()
            .unwrap()
            .arm(ChildTarget { ..identity }, Some(0), 100);
        runtime.observe_research_intervals(&[game(40), game(60)], 2);
        for _ in 0..3 {
            runtime
                .on_event(MonitorEvent::GpuSample {
                    sample: Box::default(),
                })
                .unwrap();
        }
        let windows = resident.lock().unwrap().freeze(200);
        assert_eq!(windows.len(), 1);
        assert_eq!(
            (
                windows[0].events(),
                windows[0].lost_events,
                windows[0].gpu_samples,
                windows[0].cpu_samples
            ),
            (100, 2, 3, 1)
        );
    }

    /// Every kind of event reaches the resident observer through the runtime's one entry point, `on_event`, exactly as the
    /// session delivers them: interval records, GPU samples, frames and CPU frequency events build one attempt's window.
    #[test]
    fn real_monitor_events_through_on_event_build_the_attempts_resident_window() {
        use crate::{
            autotune::research::{
                action_child::ChildTarget,
                resident_observer::{ResidentObserver, SharedResident},
            },
            metrics::LatencyHistogram,
            recorder::{CpuFreqRecord, FrameEvent},
            session_events::MonitorEvent,
        };
        let pid = std::process::id();
        let identity = process_identity(pid).expect("a live process");
        let mut runtime = runtime_for(pid, vec![]);
        let resident: SharedResident = Arc::new(Mutex::new(ResidentObserver::default()));
        runtime.set_research_resident(Some(resident.clone()));
        resident
            .lock()
            .unwrap()
            .arm(ChildTarget { ..identity }, Some(0), 100);
        let mut histogram = LatencyHistogram::new();
        for _ in 0..50 {
            histogram.record(400_000);
        }
        for second in 1..=3u64 {
            let game = IntervalRecord {
                elapsed_ms: second * 1_000,
                process_pid: Some(pid),
                class: TaskClass::Game,
                samples: 50,
                max_ns: 400_000,
                histogram: histogram.snapshot(),
                ..Default::default()
            };
            runtime
                .on_event(MonitorEvent::Interval {
                    elapsed_ms: second * 1_000,
                    records: vec![game],
                    drop_counters: DropCountersSnapshot::default(),
                })
                .unwrap();
            runtime
                .on_event(MonitorEvent::GpuSample {
                    sample: Box::default(),
                })
                .unwrap();
            runtime
                .on_event(MonitorEvent::Frame {
                    event: Box::new(FrameEvent {
                        elapsed_ms: second * 1_000,
                        frametime_ms: 8.0,
                    }),
                })
                .unwrap();
            runtime
                .on_event(MonitorEvent::CpuFreqSample {
                    event: Box::new(CpuFreqRecord {
                        elapsed_ms: second * 1_000,
                        cpu: 0,
                        freq_khz: 4_000_000,
                        timestamp_ns: second * 1_000_000_000,
                    }),
                })
                .unwrap();
        }
        let windows = resident.lock().unwrap().freeze(200);
        assert_eq!(windows.len(), 1);
        let window = &windows[0];
        assert_eq!(
            (
                window.events(),
                window.cpu_samples,
                window.gpu_samples,
                window.cpu_freq_samples,
                window.invalid_intervals,
                window.target_changed
            ),
            (150, 3, 3, 3, 0, false)
        );
    }

    #[test]
    fn the_runtime_reports_its_frames_and_frame_source_into_the_telemetry_observer() {
        use crate::{
            autotune::research::{
                action_child::ChildTarget,
                telemetry_observer::{SharedTelemetry, TelemetryObserver},
            },
            telemetry::{
                frame_source::{MeasurementStage, SourceClaim},
                live_source::FrameSourceStatus,
            },
        };
        let pid = std::process::id();
        let identity = process_identity(pid).expect("a live process");
        let mut runtime = runtime_for(pid, vec![]);
        let telemetry: SharedTelemetry = Arc::new(Mutex::new(TelemetryObserver::default()));
        runtime.set_research_telemetry(Some(telemetry.clone()));
        // Nothing attested yet: events are not counted.
        runtime.observe_research_telemetry(true);
        telemetry
            .lock()
            .unwrap()
            .arm(ChildTarget { ..identity }, Some(0));
        // The runtime's own frame source, bound to this very process.
        let mut claim = SourceClaim {
            stage: MeasurementStage::ApplicationFrame,
            producer_pid: Some(pid),
            producer_start_ticks: Some(identity.start_ticks),
            file_owner_uid: Some(identity.uid),
            launch_id_matched: true,
        };
        let mut status = FrameSourceStatus::new(claim.clone());
        status.last_frame_at = Some(std::time::Instant::now());
        runtime.frame_source = Some(Arc::new(Mutex::new(status)));
        for _ in 0..3 {
            runtime.observe_research_telemetry(true);
        }
        runtime.observe_research_telemetry(false); // not a frame: only the source is read
        let receipts = telemetry.lock().unwrap().take_receipts();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].frames, 3);
        assert!(receipts[0].source_bound_to_game && receipts[0].source_healthy);
        // Far too few frames to be eligible, and that is what the receipt says.
        assert_eq!(receipts[0].frame_verdict, "TooFewFrames");
        // A source bound to some other process is the wrong renderer.
        claim.producer_pid = Some(1);
        runtime.frame_source = Some(Arc::new(Mutex::new(FrameSourceStatus::new(claim))));
        telemetry
            .lock()
            .unwrap()
            .arm(ChildTarget { ..identity }, Some(0));
        runtime.observe_research_telemetry(false);
        let other = telemetry.lock().unwrap().take_receipts();
        assert!(!other[0].source_bound_to_game);
    }

    #[test]
    fn a_gated_action_runs_the_normal_decision_apply_and_stop_rollback_and_the_gate_records_it() {
        let pid = std::process::id();
        // GameRunnableLatency is the objective the action is judged by.
        let cpu = CandidateAction::cpu_affinity_profile_for(
            profile(),
            pid,
            crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
        );
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(pid), None)
            .with_simulated_candidates(vec![cpu.clone()])
            .with_simulated_action_effects();
        config.history_log = None;
        let mut runtime = AutotuneRuntime::new(config);
        runtime.research_campaign_active = true;
        let shared = gate(&[ResearchAction::CpuAffinityProfile], pid);
        runtime.set_research_action_gate(Some(shared.clone()));

        // The normal controller path: the gate permits, the decision starts an experiment, and the
        // runtime applies it; the gate then sees an action it must hold until it is restored.
        assert!(runtime.research_gate_permits(&cpu));
        let observation = high_quality_game_observation_with_focus_confidence(0.95);
        let decision = decide_autotune_transition(
            &runtime.controller.policy,
            &runtime.controller.state,
            &observation,
            Some(cpu),
        );
        assert!(
            matches!(decision, AutotuneDecision::StartExperiment { .. }),
            "{decision:?}"
        );
        runtime
            .apply_decision_side_effects(&observation, &decision, "gated action")
            .unwrap();
        assert!(runtime.has_active_experiment());
        runtime.observe_research_gate();
        assert!(shared.lock().unwrap().is_busy());
        assert!(
            !shared.lock().unwrap().evidence().clean,
            "outstanding is not clean"
        );

        // A stop rolls the experiment back; the gate settles and records the restored action. With no
        // kernel state probe installed here, the kernel readback is honestly not claimed.
        runtime.rollback_on_stop("research stop").unwrap();
        assert!(!runtime.has_active_experiment());
        runtime.observe_research_gate();
        let evidence = shared.lock().unwrap().evidence();
        assert!(evidence.clean, "{evidence:?}");
        assert_eq!(evidence.settled.len(), 1, "{evidence:?}");
        assert_eq!(evidence.settled[0].0, ResearchAction::CpuAffinityProfile);
        assert!(!evidence.kernel_state_proven);
    }

    #[test]
    fn an_action_judged_by_an_objective_that_has_not_graduated_is_refused_by_the_normal_controller()
    {
        let pid = std::process::id();
        // StutterScore is Experimental: only GameRunnableLatency may apply this autonomously.
        let cpu = CandidateAction::cpu_affinity_profile(profile(), pid);
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(pid), None)
            .with_simulated_candidates(vec![cpu.clone()])
            .with_simulated_action_effects();
        config.history_log = None;
        let mut runtime = AutotuneRuntime::new(config);
        let observation = high_quality_game_observation_with_focus_confidence(0.95);
        let decision = decide_autotune_transition(
            &runtime.controller.policy,
            &runtime.controller.state,
            &observation,
            Some(cpu),
        );
        let error = runtime
            .apply_decision_side_effects(&observation, &decision, "experimental objective")
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("objective_not_supported"),
            "{error:#}"
        );
        assert!(!runtime.has_active_experiment());
    }

    /// The real kernel path on a stand-in target (not the game): a process this test owns has its CPU
    /// affinity changed by the runtime's own apply and restored by the stop rollback, and the gate proves the
    /// restoration by comparing the target's /proc state before and after. Nothing here touches the game, the
    /// GPU or any other process.
    #[test]
    fn a_real_apply_and_restore_on_a_stand_in_process_is_proven_against_procfs() {
        let mut child = std::process::Command::new("sleep")
            .arg("120")
            .spawn()
            .unwrap();
        let pid = child.id();
        let allowed = |pid: u32| -> String {
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .unwrap()
                .lines()
                .find_map(|l| l.strip_prefix("Cpus_allowed_list:"))
                .unwrap()
                .trim()
                .to_owned()
        };
        // The stand-in is matched by its command name; it is not a game, so no class rule applies to it.
        let standin = Profile {
            name: "stand-in".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: Vec::new(),
                match_comm: vec![
                    crate::process_tree::CompiledPattern::new("sleep".to_owned()).unwrap(),
                ],
            }],
        };
        let original = allowed(pid);
        let cpu = CandidateAction::cpu_affinity_profile_for(
            standin,
            pid,
            crate::autotune::objective::ObjectiveKind::GameRunnableLatency,
        );
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(pid), None)
            .with_simulated_candidates(vec![cpu.clone()]);
        config.history_log = None;
        let mut runtime = AutotuneRuntime::new(config);
        runtime.research_campaign_active = true;
        let target = process_identity(pid).expect("the stand-in is alive");
        let shared: Arc<Mutex<ActionGate>> = Arc::new(Mutex::new(
            ActionGate::new(
                ActionChildScope::new(
                    [ResearchAction::CpuAffinityProfile],
                    target,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        + 600,
                ),
                Box::new(|| Some(0)),
            )
            .with_state_probe(Box::new(
                crate::autotune::research::action_gate::procfs_state,
            )),
        ));
        runtime.set_research_action_gate(Some(shared.clone()));
        assert!(runtime.research_gate_permits(&cpu));
        let observation = high_quality_game_observation_with_focus_confidence(0.95);
        let decision = decide_autotune_transition(
            &runtime.controller.policy,
            &runtime.controller.state,
            &observation,
            Some(cpu),
        );
        let applied = runtime.apply_decision_side_effects(&observation, &decision, "real apply");
        let during = allowed(pid);
        runtime.observe_research_gate();
        let stopped = runtime.rollback_on_stop("real restore");
        let after = allowed(pid);
        runtime.observe_research_gate();
        let evidence = shared.lock().unwrap().evidence();
        let _ = child.kill();
        let _ = child.wait();
        applied.unwrap();
        stopped.unwrap();
        assert_eq!(
            during, "0",
            "the runtime really changed the stand-in's affinity"
        );
        assert_eq!(after, original, "and really put it back");
        assert!(
            evidence.clean && evidence.kernel_state_proven,
            "{evidence:?}"
        );
        assert_eq!(evidence.settled.len(), 1);
    }

    #[test]
    fn a_gate_armed_for_another_process_never_permits_this_ones_candidate() {
        let pid = std::process::id();
        let cpu_again = CandidateAction::cpu_affinity_profile(profile(), pid);
        let mut runtime = runtime_for(pid, vec![cpu_again.clone()]);
        // The gate was frozen on a different process (pid 1 is never this test process).
        let stranger = process_identity(1).expect("init exists");
        let until = u64::MAX / 2;
        runtime.set_research_action_gate(Some(Arc::new(Mutex::new(ActionGate::new(
            ActionChildScope::new([ResearchAction::CpuAffinityProfile], stranger, until),
            Box::new(|| Some(0)),
        )))));
        assert!(!runtime.research_gate_permits(&cpu_again));
    }
}
