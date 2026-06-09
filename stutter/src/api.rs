//! Public façade for embedding, integration, report tooling, and stable data contracts.
//!
//! Root subsystem modules are kept crate-private. Public consumers should use
//! this module, plus the root `run_cli` entry point and root `StutterError`
//! re-export, instead of depending on internal module layout.

pub mod error {
    //! Public error and warning types returned by stable crate entry points.

    pub use crate::error::{
        ArtifactError, ConfigError, DataQualityWarning, EbpfError, OutputWarning, ProbeError,
        ProbeWarning, RecordingError, RemoteError, ReportError, StutterError, TargetError,
    };
}

pub mod actions {
    //! Public action descriptors, safety classes, outcomes, and rollback contracts.

    pub use stutter_core::ids::ActionId;

    pub use crate::actions::{
        ActionError, ActionFailure, ActionOutcome, ActionPhase, ActionResult, ActionState,
        ActionTimeout, ActionWarning, CgroupCpusetRestoreRecord, CgroupRestoreRecord,
        CpuPowerAction, CpuPowerPolicy, CpuPowerRestoreRecord, GpuPowerAction, GpuPowerMode,
        GpuPowerPolicy, GpuPowerRestoreRecord, IoPrioRestoreRecord, IrqAffinityAction,
        IrqAffinityEvidence, IrqAffinityPolicy, IrqAffinityRestoreRecord, IrqAffinityRisk,
        NiceAction, NicePolicy, NiceRestoreRecord, PhaseFailure, RestoreAllInput,
        RestoreAllSummary, RollbackCandidate, RollbackHandler, RollbackOutcome, RollbackPreview,
        RollbackRegistry, RollbackRegistryError, RollbackResult, RollbackToken,
        RollbackTokenKindError, SafetyClass, ScopeLimitExceeded, TaskIdentity, TuningAction,
        UclampRestoreRecord, VmKnobAction, VmKnobChange, VmKnobMode, VmKnobPolicy,
        VmKnobRestoreRecord,
    };
}

pub mod agent {
    //! Public agent embedding and remote-control entry points.

    pub use crate::{
        agent::{
            AgentAuth, AgentConfig, AgentLimits, AgentState, AutotuneControllerHandle,
            DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS, DEFAULT_AGENT_MAX_DURATION_SECONDS,
            DEFAULT_AGENT_MAX_REQUEST_BYTES, DEFAULT_AGENT_MAX_TARGETS,
            DEFAULT_AGENT_RATE_LIMIT_REQUESTS, DEFAULT_AGENT_RATE_LIMIT_WINDOW, RunHandle,
            default_agent_unix_socket_path, default_runs_dir, load_bearer_token, run_agent,
        },
        remote::AgentAutotuneLimits,
    };
}

pub mod alert {
    //! Public alert payload and alert sender contracts.

    pub use crate::alert::{AlertPayload, send_desktop_alert, send_webhook_alert_with_client};
}

pub mod artifacts {
    //! Public artifact kind, selection, path, and stream metadata contracts.

    pub use crate::artifacts::{
        ARTIFACT_SPECS, ArtifactCounter, ArtifactEncoding, ArtifactKind, ArtifactPath,
        ArtifactSelection, ArtifactSpec, ArtifactStreamRegistry, artifact_alias_paths,
        artifact_counter_label, artifact_file_name, artifact_is_ndjson_stream, artifact_kinds,
        artifact_path, artifact_primary_and_alias_paths, artifact_spec, optional_artifact_kinds,
        push_artifact_event,
    };
}

pub mod autotune {
    //! Public autotune command, planning, status, and data contract façade.

    pub use crate::autotune::{
        DEFAULT_MIN_FOCUS_CONFIDENCE,
        commands::live::{AutotuneCommandInput, autotune_command},
        objective::ObjectiveKind,
    };

    pub mod candidate {
        //! Public candidate plan and suggestion contracts.

        pub use crate::autotune::planning::{
            candidate::{CandidateAction, CandidateEvidence, CandidatePlan},
            dry_run::{
                CandidateDryRunRecord, CandidateDryRunner, RealCandidateDryRunner,
                dry_run_candidate, dry_run_candidates, dry_run_candidates_with_runner,
                dry_run_record_from_action_state,
            },
            executable_plan::{
                CandidateExecutablePlan, CgroupPlacementActionPlan, CpuAffinityProfilePlan,
                CpuPowerActionPlan, FakeCandidatePlan, GpuPowerActionPlan, IoPrioActionPlan,
                IrqAffinityActionPlan, NiceActionPlan, UclampActionPlan, VmKnobActionPlan,
            },
            plan_io::{
                CandidatePlanFile, CandidatePlanSummary, apply_candidate_plan_file,
                candidate_plan_path, default_candidate_plan_dir, write_candidate_plan_file,
            },
            profile_candidates::{
                CandidateProfileStatus, GeneratedCpuSetPolicy, GeneratedProfileCandidatePlan,
                GeneratedTopologyProfilePlan, RejectedCandidateProfile,
                generate_profile_candidate_plan, generate_profile_candidate_plan_for_observation,
                generate_profile_candidate_plan_with_history, generate_profile_candidates,
                generate_profile_candidates_for_observation,
                generate_topology_aware_profile_candidate_plan,
                generate_topology_aware_profile_candidates,
                generate_topology_aware_profile_candidates_with_policy,
                generate_topology_aware_profile_plan, generate_topology_aware_profiles,
                generate_topology_aware_profiles_with_policy,
            },
            suggestion::{
                CandidateManualCommands, CandidateSuggestion, print_candidate_suggestions,
                render_candidate_suggestion, render_candidate_suggestions,
                suggestion_from_candidate_dry_run_record, suggestion_from_dry_run_record,
                suggestions_from_candidates_and_dry_run_records, suggestions_from_dry_run_records,
            },
        };
    }

    pub mod activity {
        //! Public autotune activity classification contracts.

        pub use crate::autotune::activity::{ActivityClassifier, ActivityLevel};
    }

    pub mod observation {
        //! Public autotune observation contracts.

        pub use crate::autotune::{
            observation::{AutotuneObservation, WorkloadIdentity},
            quality::OnlineDataQuality,
        };
    }

    pub mod providers {
        //! Public autotune provider extension contracts.

        pub use crate::autotune::providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput,
            CandidateProviderMetadata, CandidateProviderRegistry, vm_knob::VmKnobProvider,
        };
    }

    pub mod system_context {
        //! Public autotune system-context snapshot contracts.

        pub use crate::autotune::system_context::SystemContextSnapshot;
    }

    pub mod controller {
        //! Public controller policy and transition contracts.

        pub use crate::autotune::controller::{
            ActiveExperiment, ControllerPolicy, ControllerRuntimeState, decide_autotune_transition,
        };
    }

    pub mod controller_journal {
        //! Public controller journal persistence contracts.

        pub use crate::autotune::controller_journal::{
            CONTROLLER_JOURNAL_SCHEMA_VERSION, ControllerJournalActionMetadata,
            ControllerJournalRecord, ControllerJournalState, default_controller_journal_path,
            journal_process_identity, read_controller_journal, write_controller_journal_applied,
            write_controller_journal_applied_with_metadata, write_controller_journal_applying,
            write_controller_journal_applying_with_metadata, write_controller_journal_clean,
            write_controller_journal_phase_with_metadata, write_controller_journal_record,
            write_default_controller_journal_applied, write_default_controller_journal_applying,
            write_default_controller_journal_clean,
        };
    }

    pub mod decision {
        //! Public autotune decision contracts.

        pub use crate::autotune::decision::AutotuneDecision;
    }

    pub mod emergency_restore {
        //! Public emergency restore command contracts.

        pub use crate::autotune::emergency_restore::{
            AutotuneRestoreCommandInput, AutotuneRestoreOutcome, AutotuneRestoreStatus,
            RollbackRestoreSummary, autotune_restore_command, manual_restore_command_for_token,
            restore_known_autotune_actions, restore_rollback_token,
        };
    }

    pub mod experiment {
        //! Public experiment identity and scoring contracts.

        pub use crate::autotune::experiment::{
            ActiveExperiment, ExperimentId, ExperimentPhase, WindowScore,
        };
    }

    pub mod history {
        //! Public autotune history contracts.

        pub use crate::autotune::history::{
            AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode, ControllerPhase,
            ObservationSummary, TargetIdentity, append_autotune_history_event,
            append_autotune_history_event_to_default_path, default_autotune_history_path,
            observation_summary_from_window_score, read_autotune_history_events,
        };
    }

    pub mod planner {
        //! Public candidate planning contracts.

        pub use crate::autotune::planner::{
            CandidateDenyReason, CandidateEvaluation, CandidatePlanner, PlanResult,
            PlannerDenySummary, PlannerEvaluationSummary, PlannerInput, PlannerNoActionSummary,
            PlannerSelectedSummary, PlannerSummary,
        };
    }

    pub mod replay {
        //! Public autotune replay contracts.

        pub use crate::autotune::replay::{
            AutotuneReplayDecision, AutotuneReplayInput, AutotuneReplayReport,
            ObserveOnlyReplayPolicy, ReplayPolicyEngine, replay_autotune_events, replay_command,
        };
    }

    pub mod report_overlay {
        //! Public report overlay contracts for autotune events.

        pub use crate::autotune::report_overlay::{
            AutotuneReportEvent, AutotuneReportOverlay, append_autotune_overlay_to_legacy_text,
            build_autotune_report_overlay, render_autotune_events_text,
        };
    }

    pub mod runtime {
        //! Public online autotune runtime contracts.

        pub use crate::autotune::runtime::{
            AutotuneControllerExit, AutotuneDecisionStreamEntry, AutotuneRuntime,
            AutotuneRuntimeConfig, DEFAULT_RECENT_DIAGNOSIS_LIMIT, DEFAULT_RUNTIME_WINDOW_SECONDS,
            OnlineAutotuneController, RuntimeTargetState, daemon_config_for_runtime_mode,
            daemon_phase_from_controller_phase, run_autotune_controller_session,
        };
    }

    pub mod state {
        //! Public autotune mode, controller phase, and situation labels.

        pub use crate::autotune::state::{AutotuneMode, ControllerPhase, SituationKind};
    }

    pub mod status {
        //! Public autotune status command and model contracts.

        pub use crate::autotune::status::{
            AutotuneStatus, AutotuneStatusCommandInput, StatusKeptAction, StatusTarget,
            autotune_status_command, load_autotune_status, render_autotune_status_text,
            status_from_daemon_state, status_from_history_events,
        };
    }

    pub mod workload_policy {
        //! Public workload policy configuration contracts.

        pub use crate::autotune::workload_policy::{
            DaemonWorkloadPolicyConfig, DaemonWorkloadPolicyConfigFile, WorkloadPolicyMatrix,
            WorkloadPolicyRule, WorkloadPolicyRuleConfigFile, known_action_families,
            parse_objective_kind, parse_situation_kind, parse_workload_policy_rule_configs,
            validate_action_family_name, validate_workload_policy_rule,
            workload_policy_for_situation,
        };
    }
}

pub mod config {
    //! Public configuration model, source, and merge contracts.

    pub use crate::config::{CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX};

    pub mod effective {
        //! Public resolved/effective monitor configuration contracts.

        pub use stutter_config::effective::{
            EffectiveMonitorConfig, ResolvedMonitorConfig, apply_layer,
        };

        pub use crate::config::merge::resolve_monitor_config_sources;
    }

    pub mod layer {
        //! Public partial configuration layer contract.

        pub use stutter_config::monitor_layer::MonitorConfigLayer;
    }

    pub mod merge {
        //! Public configuration merge input and output contracts.

        pub use crate::config::merge::{
            ApiOverrides, CliOverrides, ConfigSources, DefaultConfig, PresetConfig,
            RuntimeOverrides, merge_config_sources_checked, merge_config_sources_effective_checked,
        };
    }

    pub mod model {
        //! Public monitor configuration model contracts.

        pub use crate::config::model::{
            AlertConfig, CpuPerfConfig, EbpfSizingConfig, FocusConfig, HwmonConfig, MangoHudConfig,
            MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, RecordingRetentionConfig,
            RemoteConfig, RuntimeSlicesConfig, SafetyConfig, StreamConfig, TargetConfig,
            TimingConfig, UiConfig, WatchConfig,
        };
    }

    pub mod schema {
        //! Public user configuration schema contracts.

        pub use stutter_config::schema::{ConfigDiagnostic, ConfigDiagnosticLevel};

        pub use crate::config::schema::{
            CURRENT_CONFIG_VERSION, ParsedUserConfigFile, RawConfigFile,
        };
    }

    pub mod source {
        //! Public configuration provenance contracts.

        pub use stutter_config::source::{
            ConfigMergeTrace, ConfigSource, FieldProvenance, MergeReason,
        };
    }

    pub mod types {
        //! Public common configuration enum contracts.

        pub use crate::config::types::{
            CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX,
        };
    }
}

pub mod daemon {
    //! Public daemon policy, state, health, lifecycle, and runtime contracts.

    pub use crate::daemon::{
        DaemonPolicyVerdict, DaemonRuntime, DaemonRuntimeConfig, DaemonRuntimeEvent,
        DaemonTransition,
        autotune::{AutotuneSubsystem, AutotuneSubsystemEvent},
        capabilities::{CapabilityProbe, CapabilityProbeRoot, DaemonCapabilities},
        config::{
            CgroupTargetRole, DaemonAutotuneConfig, DaemonCandidateConfidenceConfig,
            DaemonCgroupTargetsConfig, DaemonConfig, DaemonHealthConfig, DaemonPreset,
            DaemonRemoteConfig, DaemonRetentionConfig, DaemonSafetyConfig, DaemonTargetConfig,
            normalize_cgroup_target_path,
        },
        explain::{
            DaemonPolicyExplanation, DaemonStatusExplanation, PolicyDecisionKind,
            PolicyExplainLine, PolicyExplanation, PolicyRuleEvaluation,
            policy_context_from_daemon_status, policy_context_from_daemon_status_at,
        },
        health::{
            SystemHealthInputs, SystemHealthIssue, SystemHealthMonitor, SystemHealthProbeRoot,
            SystemHealthSnapshot, SystemHealthState, SystemHealthThresholds,
            evaluate_system_health,
        },
        lifecycle::{
            DaemonLifecycleAction, DaemonLifecycleEvent, DaemonLifecycleInputs,
            DaemonLifecyclePolicy, DaemonLifecycleTransition, SuspendResumeDetector,
            evaluate_daemon_lifecycle_event,
        },
        monitor::{MonitorShutdownSummary, MonitorSubsystem, MonitorSubsystemConfig},
        overhead::{
            DaemonOverheadBudget, DaemonOverheadIssue, DaemonOverheadMonitor, DaemonOverheadReport,
            DaemonOverheadSnapshot, evaluate_daemon_overhead,
        },
        policy::{
            ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
            DaemonPolicyBuildInput, DaemonPolicyContext, PolicyIntent, PolicyRejection,
            RemoteApplyPolicy, RemotePolicyContext, RollbackRequirement, build_daemon_policy,
        },
        privilege::{
            CandidateApplyRequest, CandidatePlanRequest, InProcessPrivilegedActionService,
            PrivilegeCommandAllowlist, PrivilegeCommandRequest, PrivilegeDecision,
            PrivilegeProcessRole, PrivilegeTransport, PrivilegedActionService, PrivilegedOperation,
            PrivilegedWorkerHandle, PrivilegedWorkerRequest, PrivilegedWorkerResponse,
            RollbackRequest, UnixSocketPrivilegedActionService, privileged_operation_audit_event,
            run_privileged_worker_with_service,
        },
        state::{
            DAEMON_STATE_SCHEMA_VERSION, DaemonDecisionState, DaemonDegradedStatus,
            DaemonExperimentState, DaemonFaultState, DaemonPhase, DaemonProfileEnvironment,
            DaemonProfileMemory, DaemonProfilePartition, DaemonProfileValidation,
            DaemonRollbackState, DaemonState, DaemonStateSnapshotWriter, DaemonTargetState,
            DaemonWorkloadProfile, default_daemon_state_snapshot_path, load_daemon_state,
        },
        state_builders::{
            StartupRecoveryDaemonStateInput, daemon_decision_state, daemon_state_for_agent_fault,
            daemon_state_for_record_start, daemon_state_for_startup_recovery_snapshot,
            daemon_state_from_startup_recovery, safety_class_for_rollback_token,
        },
        store::DaemonStateStore,
        watchdog::{
            DaemonSelfHealingAction, DaemonWatchdogConfig, DaemonWatchdogInputs,
            DaemonWatchdogIssue, DaemonWatchdogReport, evaluate_daemon_watchdog,
        },
    };
}

pub mod daemon_policy {
    //! Compatibility façade for daemon policy and explanation contracts.

    pub use crate::daemon::{
        explain::{
            DaemonPolicyExplanation, DaemonStatusExplanation, PolicyDecisionKind,
            PolicyExplainLine, PolicyExplanation, PolicyRuleEvaluation,
            policy_context_from_daemon_status, policy_context_from_daemon_status_at,
        },
        policy::{
            ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy,
            DaemonPolicyBuildInput, DaemonPolicyContext, DaemonPolicyVerdict,
            HIGH_RISK_APPLY_IMPLEMENTED, PolicyIntent, PolicyRejection, RemoteApplyPolicy,
            RemotePolicyContext, RollbackRequirement, build_daemon_policy,
        },
    };
}

pub mod events {
    //! Public event decoding and interpretation contracts.

    pub use crate::events::{
        EventRuntimeConfig, block_io_event_record, handle_block_io_record, handle_cpu_freq_event,
        handle_event_with_runtime_config, handle_exec_event, handle_irq_record,
        handle_migration_event, irq_event_record, log_irq_event, log_irq_record,
    };

    pub mod decode {
        //! Public eBPF event decoding contracts.

        pub use crate::events::decode::{
            DecodedEbpfEvent, decode_ebpf_event, read_event_unaligned,
        };
    }

    pub mod interpret {
        //! Public scheduler event interpretation contracts.

        pub use crate::events::interpret::{
            SchedulerSampleUpdate, SpikeConfig, interpret_scheduler_event,
        };
    }
}

pub mod focus {
    //! Public focus snapshot, classification, scoring, and resolution contracts.

    pub use crate::{
        focus::{
            Classification, FocusCache, FocusCounters, FocusDecision, FocusGroup, FocusGroupKind,
            FocusPolicy, FocusProcess, FocusResolver, FocusScoreBreakdown, FocusSnapshot,
            PriorityBand, ProcessIdentity, ResolvedFocus, SafetyWarning, ThreadIdentity,
            build_focus_snapshot_from_processes, classify_process, classify_thread,
            focus_snapshot_at, priority_band_for_class, safety_warnings_for_group,
            situation_for_group,
        },
        process_tree::TaskClass as SystemTaskClass,
    };
}

pub mod presets {
    //! Public preset names and default configuration contracts.

    pub use crate::presets::{Preset, PresetDefaults, VALID_PRESETS};
}

pub mod probe_activation {
    //! Public probe activation planning contracts.

    pub use crate::probe_activation::{
        ProbeActivationPlan, ProbeActivationWarning, ProbeDisabledReason, registry_spec_for_key,
    };
}

pub mod probe_registry {
    //! Public probe registry contracts.

    pub use crate::probe_registry::{
        DataQualityRule, EbpfProgramSpec, PROBE_REGISTRY, PerfEventSpec, ProbeCapability, ProbeKey,
        ProbeSpec, TracepointSpec, implemented_probe_specs, probe_spec,
    };
}

pub mod process_tree {
    //! Public process tree snapshots, classifiers, target diffs, and scan helpers.

    pub use crate::process_tree::{
        CachedProcInfo, CompiledPattern, DEFAULT_MAX_PROC_SCAN_MS, DEFAULT_MAX_THREADS_PER_PROCESS,
        ProcInfo, ProcessCache, ScanBudget, ScanBudgetReport, TargetDiffAction, TargetDiffRef,
        TargetSnapshot, TargetSnapshotInput, TaskClass, TaskFilters, TaskInfo, classify_task,
        classify_task_with_context, collect_cgroup_pids_at, descendants_of, diff_tasks_ref,
        expand_tasks_at, find_auto_target_pids, parse_proc_stat_policy, parse_proc_stat_starttime,
        process_starttime_at, render_tree, render_tree_at, same_logical_task, scan_processes_at,
        sched_policy_name, target_snapshot, task_comm_at, thread_ids_of_at,
        thread_ids_of_at_limited,
    };
}

pub mod recorder {
    //! Public recording artifact schema, live recorder, retention, and writer contracts.

    pub use crate::recorder::{
        ArtifactSchemaVersion, BlockIoRecord, CpuFreqRecord, CpuPerfStatus, CsvOutput,
        ExporterState, FinalizeRecordingInput, FocusEvent, ForegroundEvent, FrameEvent, GpuSample,
        IntervalCsvWriter, IntervalRecord, IrqEventRecord, LiveBuffers, LiveRecorder,
        MAX_SPIKE_EVENTS, MetadataFile, MigrationEventRecord, NdjsonWriter, RecordedConfig,
        RecordedCpuSnapshot, RecordedLatency, RecordedSpike, RecordedTime, RecordingCounters,
        RecordingRetentionPolicy, RecordingRetentionSummary, RecordingRun, RecordingWarning,
        RecordingWarningKind, RuntimeSliceRecord, SESSION_SCHEMA_VERSION, ScxEvent, SessionFile,
        SessionMetadataCore, SessionSpike, SessionTask, SpikeDiagnosticContext, SpikeEvent,
        SpikeEventBuffer, SpikePushResult, StdoutJsonStream, SyncTracker, TreeEvent, WakerEntry,
        apply_recording_retention, ensure_min_free_space_for_path, finalize_recording,
        prepare_recording, print_recording_warnings, recorded_config, recorded_time,
        recording_warnings, write_ndjson_value,
    };
}

pub mod report {
    //! Public report loading, analysis, rendering, diffing, and regression contracts.

    pub use crate::report::{
        ArtifactsSummary, DataQualityLevel, DataQualitySummary, DisplayPathComponent,
        DisplayPathDiagnosisSummary, DmaBufPathSummary, EvidenceQuality, FocusReportSummary,
        ForegroundReportSummary, FrameOutlierView, FramePacingSummary, GpuEngineActivitySummary,
        HtmlChartArtifacts, HtmlReportModel, PressureKind, PressurePeakWindow,
        PressureTimelineCoverage, PressureTimelineSummary, PressureWindow, RegressionCheckSummary,
        RegressionMetric, RegressionViolation, ReportAnalysisJson, RuntimeSliceAnalysisSummary,
        RuntimeThreadSummary, SpikeClusterAnalysis, SpikeClusterSource, SpikeDensityBucket,
        TaskHtmlRow, build_report_analysis, check_regression, print_batch_report,
        print_diff_report, print_report, write_html_report,
    };
}

pub mod tune_recommendation {
    //! Public tune recommendation artifact refresh contracts.

    pub use crate::tune::{
        RankingConfidence, TuneCandidateSummary, TuneIterationOrder, TuneProfilePlanSummary,
        TuneProfileRulePlanSummary, TuneProfileStats, TuneSummary,
        comparability::TuneCoverageMetrics,
        recommendation::{
            TuneRecommendation, TuneRecommendationComparison, TuneRecommendationMetrics,
            TuneRecommendationProfilePlanSummary, TuneRecommendationVerdict,
            build_tune_recommendation, render_tune_recommendation_markdown,
        },
        recommendation_html::render_tune_recommendation_html,
    };
}

pub mod session {
    //! Public monitor session runtime entry points.

    pub use crate::session::{MonitorSession, configure_target_irqs, run_monitor};
}

pub mod session_events {
    //! Public monitor event stream data contracts.

    pub use crate::session_events::{
        DaemonEvent, DropCountersSnapshot, IntervalRecord, MonitorEvent, MonitorEventDeliveryClass,
    };
}

pub mod session_io {
    //! Public offline session artifact loading and validation contracts.

    pub use crate::session_io::{
        ArtifactLoader, CorrelationWindows, RunArtifacts, RunValidationReport, load_metadata,
        load_run_artifacts, load_session, validate_run_dir, validate_run_dir_shallow,
    };
}
