# Architecture Boundaries

This document defines the target ownership and dependency boundaries for
`stutter` subsystems. It is intentionally stricter than the current source tree
where older code has not yet been split.

These boundaries preserve the watcher/autotune safety invariant:

- observation and planning must not mutate the machine;
- candidate providers must not mutate the machine;
- mutation must go through a `TuningAction`, `actions/runner.rs`, and
  `DaemonPolicy`;
- Rollback ownership stays explicit in action implementations, controller
  journals, startup recovery, shutdown handling, and emergency restore.

Documentation here does not enable a runtime mode. A mode is supported only when
policy, runtime enforcement, rollback, audit, and user-facing command behavior
all support it.

## Refactor Checklist

Before merging large refactors:

```sh
scripts/list-transitional-modules.sh
cargo test -p stutter architecture_tests
```

## `cli`

- Module owns: command-line parsing, Clap argument structs, default CLI value
  handling, shell completion/manpage argument surfaces, and conversion from raw
  arguments into command input DTOs.
- Module may depend on: `commands::input` DTOs, config value types, daemon mode
  and policy labels needed for parsing, service request builders, and stable
  public data types needed to validate CLI input.
- Module must not depend on: `actions::runner`, action implementations,
  daemon runtime internals, autotune live experiment internals, recorder
  writers, eBPF loading, or direct system mutation helpers.
- Public entry points: `cli::parse_app_command`,
  `cli::parse_app_command_from`, exported `*Args` structs, exported CLI enum
  types such as `AutotuneCommand`, `DaemonCommand`, `ServiceCommand`,
  `ReleaseCommand`, `RulesCommand`, and `ScenarioCommand`.
- Mutation permissions: none. CLI parsing may validate and normalize input but
  must not change system state, daemon state, restore files, audit logs, or run
  artifacts.
- Persistence permissions: none. CLI parsing must not write files.
- Test expectations: parser tests should verify argument compatibility,
  defaulting, validation, DTO conversion, version behavior, and rejected invalid
  combinations without requiring live system access.

## `commands`

- Module owns: top-level command dispatch from `AppCommand` to command handlers,
  command orchestration, command-specific output formatting, and command-level
  dry-run routing.
- Module may depend on: public entry points from `agent`, `daemon`,
  `autotune`, `actions`, `recorder`, `report`, `config`, `process_tree`,
  `system` readers, and service helpers when executing the requested command.
- Module must not depend on: private implementation details just to bypass a
  subsystem boundary, direct mutation helpers that skip `DaemonPolicy`, direct
  provider mutation, or ad hoc persistence paths owned by another subsystem.
- Public entry points: `commands::dispatch`, `commands::AppCommand`, and
  command handler functions such as `run_monitor_command`,
  `run_autotune_command`, `run_report_command`, `run_restore_command`,
  `run_agent_command`, `run_status_command`, and `run_service_command`.
- Mutation permissions: commands may request mutation only through the owning
  subsystem public entry point. System-changing commands must honor dry-run
  flags and route apply/rollback through action execution and daemon policy.
- Persistence permissions: commands may initiate persistence owned by another
  subsystem, such as reports, recordings, daemon state, audit logs, restore
  files, autotune journals, or service files, but must not create separate
  undocumented persistence formats for the same state.
- Test expectations: command tests should cover dispatch mapping, dry-run
  behavior, command input conversion, error messages for unsupported modes, and
  that command-level mutation paths call the owning subsystem rather than
  duplicating policy.

## `agent`

- Module owns: local agent/control API, Unix/TCP listener setup, request
  authentication and authorization, rate limiting, remote monitor/autotune
  request validation, remote mode limits, daemon control responses, and
  allowlisted run artifact access.
- Module may depend on: daemon state/control types, daemon policy and
  capabilities, autotune command/runtime inputs, monitor config types, recorder
  artifact metadata, report artifacts, and service-safe filesystem helpers.
- Module must not depend on: raw action mutation helpers, provider internals,
  direct privileged system mutation from non-local requests, unbounded artifact
  filesystem access, or CLI parsing.
- Public entry points: `agent::run_agent`, `AgentConfig`, `AgentLimits`,
  `AgentAuth`, `AgentState`, `AutotuneControllerHandle`, `RunHandle`,
  `default_runs_dir`, `default_agent_unix_socket_path`, and
  `load_bearer_token`.
- Mutation permissions: the agent may request daemon control-plane state
  changes only after authorization and policy checks. Privileged operations must
  go through the daemon privilege boundary or another explicit daemon-owned
  control path.
- Persistence permissions: the agent may read configured bearer tokens, prepare
  its Unix socket path, and serve allowlisted files from run artifact
  directories. It must not write tuning state, controller journals, restore
  files, or audit logs directly.
- Test expectations: tests should cover auth scopes, local versus remote
  privilege decisions, rate limits, request size limits, remote mode denials,
  artifact path validation, and policy rejections for unsafe remote apply.

## `daemon`

- Module owns: daemon runtime state, mode and policy enforcement, lifecycle
  boundaries, health and capability snapshots, status/explain output, watchdogs,
  privileged worker transport, state store, pause/resume control, restore and
  emergency restore orchestration, and daemon-facing autotune subsystem wiring.
- Module may depend on: `autotune` controller/runtime entry points, action
  descriptors and action runner entry points, monitor/session runtime entry
  points, config, process inspection, system inventory, health readers, and
  recorder/report summaries needed for daemon status.
- Module must not depend on: `cli`, `commands`, Clap parsing, direct provider
  mutation, undocumented persistence formats, or system-changing helpers that
  bypass `DaemonPolicy::check_action`.
- Public entry points: re-exports from `daemon::mod`, including
  `DaemonRuntime`, `DaemonRuntimeConfig`, `DaemonRuntimeEvent`,
  `DaemonTransition`, `DaemonPolicy`, `DaemonPolicyContext`, `DaemonMode`,
  `DaemonState`, `DaemonStateStore`, `DaemonCapabilities`,
  `SystemHealthMonitor`, `AutotuneSubsystem`, `PrivilegedActionService`,
  `InProcessPrivilegedActionService`, and restore/status/explain types.
- Mutation permissions: daemon-owned mutation is limited to policy-gated action
  apply/rollback, daemon state transitions, lifecycle responses, privileged
  worker operations, and restore/emergency restore. Every apply path must check
  daemon policy before mutation.
- Persistence permissions: daemon may persist daemon state snapshots,
  controller-visible daemon state, watchdog/health state when owned by daemon
  code, and may invoke owned action/autotune audit, journal, restore, and
  history persistence paths.
- Test expectations: tests should cover policy decisions, state transitions,
  lifecycle boundaries, restore behavior, privileged worker authorization,
  status/explain output, and architecture tests that daemon internals do not
  depend on CLI parsing.

## `autotune`

- Module owns: autotune observations, observation builders, data quality,
  situation classification, workload policy, candidate provider registry,
  candidate planning, conflict checks, controller state, decision logs,
  live experiment state, measurement windows, history, kept-candidate state,
  controller journal records, startup recovery, shutdown rollback registration,
  and autotune-specific human/status output.
- Module may depend on: daemon mode/policy/capabilities/health snapshots,
  action descriptors and `CandidateAction` types, focus resolution,
  process-tree snapshots, system context snapshots, monitor metrics, profiles,
  and recorder/report data needed for offline replay or status.
- Module must not depend on: `cli`, command parsing, direct sysfs/procfs
  mutation, direct provider mutation, direct action implementation mutation
  outside the action runner, or persistence paths that bypass its journal,
  history, decision log, and recovery modules.
- Public entry points: `autotune::autotune_command`,
  `AutotuneCommandInput`, `planner::PlanResult`,
  `providers::CandidateProvider`, `providers::CandidateProviderRegistry`,
  `runtime::AutotuneRuntime`, `live_experiment::LiveExperimentManager`,
  `controller::ControllerRuntimeState`, `startup_recovery`,
  `shutdown`, `status`, and `emergency_restore`.
- Mutation permissions: observation, quality checks, planning, provider
  proposal, scoring, ranking, and dry-run evaluation must not mutate the
  machine. Autotune may mutate only when an apply decision is executed through
  `TuningAction`, `actions/runner.rs`, and `DaemonPolicy`, or through the
  daemon privileged action service with equivalent policy enforcement.
- Persistence permissions: autotune may write only owned autotune state such as
  controller journals, decision logs, history, generated profile/candidate plan
  artifacts, startup recovery records, and shutdown rollback registrations.
- Test expectations: tests should cover provider proposal purity, policy
  denials, dry-run failures, conflict groups, objective comparisons, controller
  phase transitions, journal records, startup recovery, rollback-on-fault, and
  that providers do not perform machine mutation.

## `actions`

- Module owns: `TuningAction`, action identifiers, action safety classes,
  action warnings/state/outcomes, rollback tokens, rollback registry and
  handlers, concrete tuning action implementations, preflight/dry-run/apply/
  verify/rollback contracts, and audited action runner execution.
- Module may depend on: daemon policy descriptors and policy context,
  process/task identity readers needed to target an action, audit logging,
  filesystem paths required to create rollback records, and OS interfaces
  required by concrete action implementations.
- Module must not depend on: `cli`, `commands`, `agent`, daemon runtime state,
  autotune planner/controller internals, candidate providers, report rendering,
  or recorder live writers.
- Public entry points: `actions::TuningAction`, `ActionId`, `SafetyClass`,
  `ActionState`, `ActionOutcome`, `ActionWarning`, `RollbackToken`,
  `RollbackRegistry`, `RollbackHandler`, `RollbackCandidate`,
  `RollbackPreview`, `RollbackResult`, `RestoreAllInput`,
  `RestoreAllSummary`, `runner::ActionRunPolicy`,
  `runner::AuditedActionResult`, `runner::run_audited_action`, and
  `runner::run_audited_action_with_audit_path`.
- Mutation permissions: concrete actions may mutate only inside `apply` and
  `rollback`. `preflight`, `dry_run`, descriptor construction, and candidate
  conversion must not mutate. Runner apply must check policy before mutation and
  must obtain rollback state according to the action descriptor.
- Persistence permissions: actions may write rollback/restore records and audit
  events through owned action/audit paths. They must not write daemon controller
  journals, autotune history, recorder sessions, or reports directly.
- Test expectations: tests should cover descriptor accuracy, policy rejection,
  dry-run non-mutation, rollback token correctness, audit records, restore
  behavior for dead/reused tasks, and architecture tests preventing CLI/command
  dependencies.

## `focus`

- Module owns: focus snapshots, focus processes, focus groups, group scoring,
  focus classification, focus resolution policy, foreground fallback handling,
  protected-task warnings, and focus-related public types.
- Module may depend on: process-tree task classes and snapshots, foreground
  snapshots, scheduler/metric evidence, community process classification, and
  config values needed to tune focus policy.
- Module must not depend on: action implementations, action runner, autotune
  apply paths, daemon runtime mutation, recorder writers, report rendering, CLI
  parsing, or command dispatch.
- Public entry points: `FocusDecision`, `FocusPolicy`, `FocusResolver`,
  `ResolvedFocus`, `FocusSnapshot`, `FocusProcess`, `FocusGroup`,
  `FocusGroupKind`, `FocusClassification`, `FocusClassificationReason`,
  `SafetyWarning`, and `SystemTaskClass`.
- Mutation permissions: none. Focus code may classify, score, and choose a
  target but must not alter tasks, cgroups, scheduler settings, power settings,
  or persistent state.
- Persistence permissions: none. Focus output may be recorded by recorder or
  daemon/autotune state owners, but focus must not write those records itself.
- Test expectations: tests should cover scoring, classification, foreground
  fallback, protected-task warnings, stable target selection, and edge cases for
  system service or overly broad groups.

## `recorder`

- Module owns: live recording buffers, interval/runtime-slice record aliases,
  session schema version, session metadata, session file layout, JSON/CSV
  writers, retention policy, spike event buffering, recording counters, and
  exported recording event types.
- Module may depend on: metrics records, foreground events, scx events,
  decoded/normalized monitor events, filesystem paths for run artifacts, and
  config values that select recording behavior.
- Module must not depend on: action runner, concrete tuning actions, daemon
  policy decisions, autotune candidate providers, CLI parsing, command dispatch,
  or report rendering internals.
- Public entry points: `SESSION_SCHEMA_VERSION`, `LiveRecorder`,
  `LiveBuffers`, `ExporterState`, `RecordingCounters`, `IntervalRecord`,
  `RuntimeSliceRecord`, `RecordingRetentionPolicy`,
  `RecordingRetentionSummary`, `SessionInfo`, `SessionPaths`,
  `SpikeEventBuffer`, `SpikePushResult`, `write_interval_csv`, and exported
  writer/event record types from `recorder::mod`.
- Mutation permissions: recorder must not mutate machine tuning state. It may
  mutate in-memory recording buffers and counters.
- Persistence permissions: recorder owns writing run artifacts, session files,
  interval/event JSONL files, CSV files, retention deletion within the run
  artifact area, and recording metadata. It must not write daemon state,
  autotune journals/history, action restore files, or audit logs.
- Test expectations: tests should cover schema compatibility, session file
  paths, writer output, retention behavior, spike buffering, live buffer
  accounting, and fixture/regression loading.

## `report`

- Module owns: offline report analysis, artifact summaries, data-quality
  summaries, pressure/frame/focus/foreground report views, spike clustering,
  regression analysis, text report rendering, HTML report rendering, report
  diffs, and report JSON models.
- Module may depend on: recorder artifact schemas, metrics records, autotune
  report overlays/history summaries, config values needed to interpret
  artifacts, and static report templates.
- Module must not depend on: live action apply/rollback, daemon runtime mutation,
  candidate providers, eBPF loading, CLI parsing, or command dispatch.
- Public entry points: exported analysis/diff/html/regression/text APIs from
  `report::mod`, including `ReportAnalysisJson`, `HtmlReportModel`,
  `ArtifactsSummary`, `DataQualitySummary`, `FocusReportSummary`,
  `ForegroundReportSummary`, `FramePacingSummary`, `SpikeClusterAnalysis`,
  `RegressionMetric`, and report build/render helpers.
- Mutation permissions: none for machine state. Report code may build in-memory
  models and compute derived summaries only.
- Persistence permissions: report may write user-requested report outputs such
  as HTML, text, JSON, or diff artifacts. It must not write daemon state,
  autotune journals/history, action restore files, audit logs, or recorder live
  session files except through explicit report-output paths.
- Test expectations: tests should cover artifact parsing, report model
  construction, regression metrics, diff output, text/HTML rendering, and
  missing/corrupt artifact handling.

## `config`

- Module owns: monitor config model types, config layers, merge logic,
  effective config resolution, config schema conversion/diagnostics, config
  sources, provenance, and shared config enums such as `CsvStreamTarget`,
  `FocusSource`, `ForegroundSource`, and `TARGET_PIDS_MAX`.
- Module may depend on: serialization/deserialization crates, path/time value
  types, config error types, and stable value enums required by the config
  model.
- Module must not depend on: action runner, concrete actions, daemon runtime,
  autotune runtime, recorder live writers, report rendering, CLI parsing, or
  command dispatch.
- Public entry points: `effective::EffectiveMonitorConfig`,
  `effective::ResolvedMonitorConfig`,
  `effective::resolve_monitor_config_sources`, `layer::MonitorConfigLayer`,
  `merge::ConfigSources`, `merge::CliOverrides`, `merge::DefaultConfig`,
  `merge::PresetConfig`, `model::MonitorConfig`, schema/source types, and
  `types::{CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX}`.
- Mutation permissions: none for machine state. Config merge may mutate an
  in-memory config value while constructing an effective config.
- Persistence permissions: config may read configured config files through its
  source layer. It must not write config files unless a future explicit config
  writer boundary is added.
- Test expectations: tests should cover layer precedence, provenance,
  diagnostics, schema conversion, defaults, invalid values, and source merge
  behavior.

## `process_tree` / future `process`

- Module owns: `/proc` process scanning, process/thread identity, task
  classification, priority-band classification, process cache data,
  target snapshots, target diffing, task expansion, cgroup PID collection,
  process start-time parsing, thread enumeration, and tree rendering.
- Module may depend on: read-only procfs filesystem access, regex matching,
  community process rules, task-class definitions, and small compatibility
  re-exports from `procfs.rs`.
- Module must not depend on: action runner, concrete tuning actions, daemon
  runtime, autotune controller internals, recorder writers, report rendering,
  CLI parsing, or command dispatch.
- Public entry points: `ScanBudget`, `ScanBudgetReport`, `ProcessCache`,
  `ProcInfo`, `TaskInfo`, `TaskClass`, `PriorityBand`, `TargetSnapshot`,
  `TargetSnapshotInput`, `TaskFilters`, `TargetDiffAction`,
  `scan_processes_at`, `descendants_of`, `thread_ids_of_at_limited`,
  `thread_ids_of_at`, `task_comm_at`, `target_snapshot`, `diff_tasks_ref`,
  `find_auto_target_pids`, `expand_tasks_at`, `process_starttime_at`,
  `parse_proc_stat_starttime`, `parse_proc_stat_policy`, `classify_task`,
  `classify_task_with_context`, `render_tree`, `render_tree_at`, and
  `collect_cgroup_pids_at`.
- Mutation permissions: none. Process-tree code reads `/proc`, classifies
  tasks, and builds snapshots only.
- Persistence permissions: none. Any future `process` module split must keep
  process inspection read-only unless a separate policy-gated action boundary is
  created.
- Test expectations: tests should cover proc stat parsing, task
  classification, thread expansion, target snapshot construction, diffing,
  cgroup PID collection, tree rendering, scan budget behavior, and TID reuse
  safety assumptions.

## `system`

- Module owns: read-only system inventory and live system signal readers that
  are not themselves recorder/session storage, currently spread across
  `system_inventory.rs`, `topology.rs`, `hwmon.rs`, `psi.rs`,
  `perf_counters.rs`, `sched_state.rs`, `scx.rs`, `irq_inspect.rs`,
  `kernel_event.rs`, `mangohud.rs`, and foreground provider helpers.
- Module may depend on: read-only `/proc` and `/sys` roots, kernel event bytes,
  stutter-common event ABI types, config roots, command execution required by
  foreground providers, and parser helpers for system text formats.
- Module must not depend on: action apply/rollback, daemon state mutation,
  autotune provider mutation, recorder writers, report rendering, CLI parsing,
  or command dispatch.
- Public entry points: `SystemInventory`, `SystemInventoryRoot`,
  `PowerSourceSnapshot`, `TopologyModel`, `CpuInfo`, `CoreInfo`,
  `HwmonReader`, `probe_hwmon_with_options`, `PsiReader`, `PsiSnapshot`,
  `CpuPerfSampler`, `CpuPerfDelta`, `try_open_disabled_cycles_current_thread`,
  `classify_switch_prev_state`, `ScxTracker`, `ScxEvent`, `IrqLine`,
  `parse_proc_interrupts`, `filter_sort_and_limit_irqs`,
  `render_irqs_human`, `KernelEvent`, `KernelEventDecoder`,
  `read_frame_events`, `parse_frame_events`, `MangoHudLiveParser`,
  `ForegroundProvider`, `ForegroundResolver`, and foreground snapshot/event
  types.
- Mutation permissions: none for tuning state. System readers may sample,
  parse, cache in memory, or execute read-only discovery commands, but must not
  write sysfs, procfs, cgroup, scheduler, power, IRQ, GPU, or VM knobs.
- Persistence permissions: none. Samples may be handed to recorder, daemon,
  autotune, or report owners for persistence.
- Test expectations: tests should cover parser behavior for `/proc` and `/sys`
  text, topology parsing, inventory snapshots with fake roots, foreground title
  redaction, MangoHud parsing, kernel event decoding, and no-mutation behavior
  for readers.

## `events`

- Module owns: conversion from raw/shared event structs into monitor events,
  event runtime configuration, decoding helpers, interpretation helpers, IRQ
  record handling, migration handling, CPU frequency handling, block I/O record
  conversion, exec event handling, event logging helpers, and event record
  timestamp normalization.
- Module may depend on: `stutter-common` event structs/constants, monitor event
  types, scheduler/runtime metric records, recorder record structs when
  converting persisted event records, and system time/monotonic start
  information.
- Module must not depend on: recorder live writers from decode code, action
  runner, concrete tuning actions, daemon runtime mutation, autotune planning,
  report rendering, CLI parsing, or command dispatch.
- Public entry points: `events::decode`, `events::interpret`,
  `handle_irq_record`, `handle_migration_event`, `handle_cpu_freq_event`,
  `block_io_event_record`, `handle_block_io_record`, `handle_exec_event`,
  `EventRuntimeConfig`, `handle_event_with_runtime_config`,
  `irq_event_record`, `log_irq_record`, and `log_irq_event`.
- Mutation permissions: none. Event handling may create in-memory monitor
  events and log messages only.
- Persistence permissions: none. Recorder owns event persistence.
- Test expectations: tests should cover event decoding, event interpretation,
  timestamp conversion, runtime config behavior, block I/O conversion, exec
  events, IRQ logging records, and architecture tests keeping decode isolated
  from live recording.

## `stutter-common`

The eBPF crate keeps tracepoint entrypoints in one translation unit for verifier
and linking stability. Organization is maintained through internal sections
rather than Rust module splitting; behavior-sensitive offsets, map capacities,
and event structs should not be rearranged casually.

- Module owns: no-std shared event ABI constants, drop counter constants,
  C-compatible event structs shared between eBPF and userspace, optional
  userspace `aya::Pod` implementations behind the `user` feature, and
  compile-time layout assertions.
- Module may depend on: `core` and optional `aya` when the `user` feature is
  enabled.
- Module must not depend on: the `stutter` crate, `std`, filesystem APIs,
  logging, config, daemon, autotune, recorder, report, CLI, commands, or any
  mutation/persistence code.
- Public entry points: event constants such as `EVENT_RUNNABLE_LATENCY`,
  `EVENT_IRQ_LATENCY`, `EVENT_MIGRATION`, `EVENT_CPU_FREQ`,
  `EVENT_STAT_WAIT`, `EVENT_BLOCK_IO`, and `EVENT_EXEC`; drop constants such
  as `DROP_WAKEUP_DATA_INSERT_FAILED`, `DROP_RINGBUF_RESERVE_FAILED`,
  `DROP_IRQ_START_TIMES_INSERT_FAILED`, `DROP_BLOCK_START_INSERT_FAILED`,
  `DROP_WAKEUP_DATA_STALE_ENTRY`, and `DROP_COUNTERS_MAX`; event structs
  `SchedulerEvent`, `IrqEvent`, `MigrationEvent`, `CpuFreqEvent`,
  `StatWaitEvent`, `BlockIoEvent`, and `ExecEvent`.
- Mutation permissions: none.
- Persistence permissions: none.
- Test expectations: compile-time layout assertions must remain present for ABI
  safety. Any event ABI change must update eBPF/userspace expectations together
  and include tests or assertions that prove layout compatibility.
