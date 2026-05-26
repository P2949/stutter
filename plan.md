I reviewed the current `stutter-experimental(20).zip` statically and built this as a **slow, commit-by-commit maturation plan**. I could not run `cargo fmt/test/clippy` in this environment, so every step should be validated locally.

The theme is simple: **do not rewrite everything**. Turn the current strong but huge codebase into a smaller, stricter, easier-to-review system by shrinking hot files, removing scaffolding, hardening architecture gates, migrating the split crates for real, and reducing “existing debt” allowlists.

Use this as a long commit queue. Each numbered step should be small enough to commit separately.

---

# Implementation progress

Updated as implementation items are completed or verified in this workspace.

| Step | Status | Evidence |
| ---- | ------ | -------- |
| 0.1 | Completed | `docs/internal/cleanup-baseline.md` records the top-file baseline. |
| 0.2 | Completed | Baseline logs are under `/tmp/stutter-baseline-20260526`: `cargo fmt --all`, `cargo test --all`, `cargo clippy --all-targets -- -D warnings`, and current `cargo xtask validate` pass; the literal old `cargo xtask workflow` command is logged as stale/unregistered. |
| 0.3 | Completed | `docs/internal/CLEANUP_SEQUENCE.md` defines cleanup branch rules. |
| 1.1 | Completed | No tracked `scratch/check_size.rs`; `stutter-common` has `SchedulerEvent` layout assertions. |
| 1.2 | Completed | No `stutter/scratch/` directory; fault-delta behavior is covered by `stutter/src/metrics/tests.rs`. |
| 1.3 | Completed | `stutter/src/architecture_tests/scratch_dir.rs` forbids scratch directories. |
| 2.1 | Completed | The production Rust file-size gate has already moved below the original 1000-line threshold. |
| 2.2 | Completed | `PRODUCTION_RUST_FILE_SIZE_LIMIT_LINES` is now 800 with a separate 1000-line test threshold, and `architecture_tests::file_size` passes. |
| 2.3 | Completed | `stutter/src/architecture_tests/file_size.rs` now applies separate 800-line production and 1000-line test thresholds, with current oversized production files pinned by exact shrink-only allowlist ceilings. |
| 2.4 | Completed | Unwrap/expect debt is split into runtime and cfg-test/fixture allowlists with category-aware failures and stricter runtime reasons. |
| 2.5 | Completed | `stutter/src/architecture_tests/panic_paths.rs` blocks new production `panic!`, `unreachable!`, and `todo!` calls while honoring cfg-test code and invariant-documented `unreachable!`. |
| 2.6 | Completed | `stutter/src/architecture_tests/unsafe_safety.rs` enforces local `SAFETY:` comments for non-eBPF unsafe code; remaining libc/syscall/test-environment unsafe sites now document their contracts. |
| 3.1 | Completed | `stutter-report` public comments now describe migration boundaries instead of placeholder/future-migration scaffolding, with the checklist in `docs/REPORT_CRATE_MIGRATION.md`. |
| 3.2 | Completed | Pure report model structs are present in `stutter-report/src/model.rs` and the main crate re-exports them through `stutter/src/report/model.rs`. |
| 3.3 | Completed | `stutter-report::load_report_model` loads real `ReportModel` JSON and is covered by crate tests. |
| 3.4 | Completed | `stutter-report/src/render/text/` owns the header, quality, cluster, correlation, frame, and diagnosis render slices; crate golden tests and the main text-report snapshot pass byte-for-byte. |
| 3.5 | Completed | `stutter-report/tests/golden.rs` validates the minimal text-render fixture. |
| 3.6 | Completed | `stutter-report::diff` now compares real model score/latency/culprit/data-quality fields and owns generic task diffing; main `stutter report diff` maps sessions into that shared engine without changing existing CLI output. |
| 3.7 | Completed | `stutter-report` no longer exposes the scaffold `UnsupportedOperation` error path. |
| 4.1 | Completed | `docs/CONFIG_CRATE_MIGRATION.md` inventories current config ownership, main-crate-only fields, and the safe migration order for `stutter-config`. |
| 4.2 | Completed | `FocusSource`, `ForegroundSource`, `WaylandPresentationSource`, `CsvStreamTarget`, and `TARGET_PIDS_MAX` now live in `stutter-config` behind an optional CLI feature; the main crate re-exports them and config merge tests pass unchanged. |
| 4.3 | Completed | `stutter-config::validate_static_config` now owns pure scalar validation, including `target.max_tasks`; the main crate keeps eBPF/runtime sizing checks in `validate_runtime_config`, and shared/main config tests pass. |
| 4.4 | Completed | `config/merge.rs` is split into `config/merge/mod.rs` and `config/merge/tests.rs`; the public merge functions are unchanged, merge tests pass, and the stale oversized allowlist entry was removed. |
| 4.5 | Completed | `ConfigMergeTrace` and `MergeReason` now summarize selected field layers from existing provenance for diagnostics/tests; merge trace tests assert default, override, and selected-layer reasons without changing merge precedence. |
| 5.1 | Completed | eBPF map declarations live in `stutter-ebpf/src/maps.rs` with public map symbol names unchanged; `main.rs` wires the module instead of owning map definitions. |
| 5.2 | Completed | eBPF target filtering lives in `target_filter.rs` and drop-counter mutation now lives in `drop_counters.rs`, with helpers kept small and `#[inline(always)]`; `architecture_tests::ebpf_layout` passes. |
| 5.3 | Completed | Runnable-depth and target-pending accounting live in `stutter-ebpf/src/runnable_depth.rs`, leaving scheduler handlers to call focused accounting helpers. |
| 5.4 | Completed | Scheduler tracepoint logic lives in `stutter-ebpf/src/scheduler.rs`; the eBPF layout architecture test guards the split. |
| 5.5 | Completed | Process lifecycle and fault-counter tracepoint logic lives in `stutter-ebpf/src/process_lifecycle.rs`, away from scheduler handling. |
| 5.6 | Completed | IRQ allowlist, start tracking, and duration emission live in `stutter-ebpf/src/irq.rs`, with focused map sizing and drop-counter wiring. |
| 5.7 | Completed | CPU frequency tracepoint handling lives in `stutter-ebpf/src/cpu_frequency.rs`; `main.rs` is now tracepoint entry wiring plus remaining hardware-specific handlers. |
| 5.8 | Completed | `architecture_tests::ebpf_layout` enforces an 800-line maximum for every `stutter-ebpf/src/*.rs` file and checks that extracted helpers stay out of `main.rs`. |
| 6.1 | Completed | `stutter-ebpf/src/wakeup_data.rs` documents the `seq` ABA-prevention invariant at the map payload field. |
| 6.2 | Completed | `stutter-ebpf/src/wakeup_data.rs` now pins `WakeupData` size and field offsets with compile-time assertions. |
| 6.3 | Completed | `stutter_common::DROP_COUNTER_METADATA` centralizes drop-counter labels/field names and tests cover every ABI slot; `docs/EBPF_CAPACITY.md` now references the table and current slot count. |
| 7.1 | Completed | `stutter/src/ebpf/preflight/` now separates tracepoint validation, CPU possible parsing, system warning plumbing, and focused tests; `preflight` and `ebpf_loader::tests` both pass. |
| 7.2 | Completed | Tracepoint names and expected fields now use typed `TracepointName`/`TracepointFieldSpec` descriptors from `stutter-common`; eBPF preflight and format validators consume typed descriptors, with `ebpf_loader::tests` passing. |
| 7.3 | Completed | `stutter doctor tracepoints --dump` now emits exact kernel tracepoint format snapshots for the tracepoints stutter consumes, with JSON support through the existing doctor flag; doctor/CLI tests cover parsing and exact format capture. |
| 8.1 | Completed | `actions/irq_affinity` is split into `model`, `validate`, `sysfs`, `apply`, and `tests` modules; the old oversized file allowance was removed and the largest new module is under 500 lines. |
| 8.2 | Completed | `RollbackToken` now has typed borrowed/owned accessors for restore-token variants, and production rollback handlers use those accessors while preserving structured expected/actual token-kind errors. |
| 8.3 | Completed | `stutter/src/actions/syscalls.rs` centralizes `setpriority`, `ioprio_get/set`, and `sched_get/setattr`; `nice`, `ioprio`, and `uclamp` now call safe wrappers with context-rich errors. |
| 8.4 | Completed | Action syscall unsafe blocks are documented once in `stutter/src/actions/syscalls.rs`, and `architecture_tests::unsafe_safety` passes with action modules using safe wrappers. |
| 8.5 | Completed | `ActionPreflightReport` and `ActionBlocker` now provide a shared blocker/warning shape, and the audited action runner converts existing preflight results through it before rendering failures. |
| 9.1 | Completed | `daemon/policy/evaluate` is now split into mode, risk, remote, target/context, and reason helpers; policy snapshot tests and the full daemon policy test filter pass unchanged. |
| 9.2 | Completed | Policy decision golden fixtures now cover observe, apply-low-risk, and remote non-loopback decisions under `tests/fixtures/policy/`, with snapshot tests guarding the serialized explanation shape. |
| 9.3 | Completed | `stutter/src/architecture_tests/mutation_paths.rs` now blocks direct privileged mutation symbols and sensitive cgroup/VM-knob path references outside approved action, affinity, process-cgroup, and emergency-restore boundaries. |
| 10.1 | Completed | `autotune/objective.rs` is split into `objective/` modules for shared helpers and individual objective comparators; the old oversized allowlist entry was removed, and objective plus file-size architecture tests pass. |
| 10.2 | Completed | `CompileThroughputSignal` now carries direct compile progress interval/sample evidence from rolling-window records; the compile objective prefers it before the proxy fallback, and focused compile-throughput/rolling-window tests pass. |
| 10.3 | Completed | `ObjectiveDecision` now records `ObjectiveOutcome`, `ObjectiveSignal`, guard failures, and confidence; live experiment keep/revert handling can include the objective decision summary, with objective-decision and lifecycle tests passing. |
| 11.1 | Completed | Candidate memory is split into `model`, `key`, `decay`, `diagnostics`, `persistence`, and focused tests; raw-score architecture checks were retargeted, the old oversized allowance was removed, and candidate-memory/file-size tests pass. |
| 11.2 | Completed | Candidate memory context keys now use typed `WorkloadIdentity`, `ExecutableFingerprint`, and `CandidateMemoryKey` constructors; raw workload/executable fields are no longer accepted in context call sites, with typed-key, candidate-memory, planner eligibility, and file-size tests passing. |
| 11.3 | Completed | Candidate memory now stores readable identity summaries alongside context/workload hashes, marks incompatible same-hash entries degraded during deserialize, skips degraded memory for cooldown/ranking/profile export, and reports diagnostics through daemon degraded status; candidate-memory, planner eligibility, daemon-state, and file-size tests pass. |
| 12.1 | Completed | `autotune/runtime.rs` was moved into `runtime/mod.rs` and split into `start`, `tick`, `stop`, `restore`, `state_snapshot`, and `plan_output` lifecycle modules; the old oversized allowlist was removed, direct-print coverage retargeted the runtime directory, and runtime/file-size/direct-print tests pass. |
| 12.2 | Completed | `AutotuneRuntimePhase` and its phase machine now gate runtime observation, planning, dry-run, apply, measurement, keep/revert, and fault transitions; invalid transitions log and force the controller/runtime into faulted state, with runtime and file-size tests passing. |
| 12.3 | Completed | Runtime phase table tests now cover `Idle -> ObservingBaseline`, `MeasuringCandidate -> KeepingCandidate`, denied `MeasuringCandidate -> ApplyingCandidate`, and denied `Faulted -> ApplyingCandidate`; a runtime-level test verifies denied transitions force `ControllerPhase::Faulted`, with runtime and file-size tests passing. |
| 13.1 | Completed | `autotune/workload_policy.rs` is split into `model`, `defaults`, `parse`, `lint`, `match`, and focused tests under `workload_policy/`; the old oversized allowlist entry was removed, and workload-policy plus file-size tests pass. |
| 13.2 | Completed | `WorkloadPolicyLintKind` now backs workload policy lint records while preserving `reason_code`; workload-policy tests assert typed lint kinds, daemon-policy JSON tests assert the serialized `kind`, and workload-policy/daemon-policy/file-size tests pass. |
| 14.1 | Completed | `autotune/planning/profile_candidates.rs` split into submodules and oversized file allowance removed. |
| 14.2 | Completed | `GeneratedProfile` invariant object added and validated in `profile_candidates`. |
| 15.1 | Completed | `session/monitor_session.rs` split into modular components like `event_loop`, `targets`, `probes`, etc. |
| 15.2 | Completed | Introduced `MonitorRuntimeHandles` and updated all internal references to use disjoint fields via macros, satisfying the architecture test constraints. |
| 15.3 | Completed | `execute_shutdown_sequence` function enforces explicit drop order (bus → flush → exporters → ebpf → final report); `shutdown/tests.rs` with `assert_shutdown_order` uses fake handles with `DropTracker` to verify the order. |
| 16.1 | Completed | `session_io.rs` now only wires/reexports focused modules for run artifacts, path resolution, required/optional loading, consistency checks, data-quality warnings, and validation; the old `session_io.rs` oversized allowance was removed, with session_io, artifact_contract, and file-size tests passing. |
| 16.2 | Completed | `ArtifactPath` now wraps `run_dir + ArtifactKind`, production artifact path call sites use `ArtifactKind`/`artifact_path` helpers, and `architecture_tests::artifact_paths` prevents new direct artifact filename joins outside test code; recorder session, artifact-contract, artifact-path, and file-size tests pass. |
| 17.1 | Completed | `recorder/session.rs` is now split into `session/mod.rs`, `prepare`, `finalize`, `metadata`, `writers`, and focused tests; the old oversized allowlist entry was removed, with recorder session, file-size, and direct-print tests passing. |
| 17.2 | Completed | `recording_warnings` now returns structured `RecordingWarning { kind, message }` values with `RecordingWarningKind`, while printing preserves the existing text; recorder session tests assert warning kinds, and recorder session/API/file-size tests pass. |
| 18.1 | Completed | `cli/mod.rs` is now a 55-line wiring/reexport module, clap command definitions live in `cli/app.rs`, parsing is split across `cli/parse.rs` and focused `cli/parse/*` helpers, and `config_bridge`, `help`, and `version` hold the remaining extracted helpers; the old `src/cli/mod.rs` oversized allowance was removed, with CLI and file-size architecture tests passing. |
| 18.2 | Completed | CLI parser tests now live under `stutter/src/cli/tests/` with production files reduced to `#[cfg(test)]` path-module hooks, including the monitor/report test trees and shared monitor test helper; stale CLI file-size allowlist entries were removed, and CLI, unwrap/expect, and file-size architecture tests pass. |
| 18.3 | Completed | `cli::version_tests::clap_help_output_matches_snapshots` now snapshots top-level, monitor, daemon, and autotune help text under `stutter/tests/snapshots/`, so command-shape/help drift is deliberate; the full CLI test filter passes. |
| 19.1 | Completed | `tui.rs` is now `tui/mod.rs` plus focused `terminal`, `status`, `task_table`, `sparkline`, `cpu_heat`, `autotune_panel`, and `diagnosis` modules; status/autotune/state render helpers have module-local tests, the old `src/tui.rs` oversized allowance was removed, and TUI plus file-size tests pass. |
| 19.2 | Completed | `tui/model.rs` now builds an owned `TuiModel` with `TuiTaskRow`, `TuiAutotunePanel`, CPU/sparkline data, and `TuiDiagnosisLine` values; widget modules render from the model instead of runtime state, model formatting tests live under `tui/tests/model.rs`, and TUI, file-size, and unwrap/expect architecture tests pass. |
| 20.1 | Completed | `metrics.rs` is now `metrics/mod.rs` plus focused `task_stats`, `cpu_stats`, `interval`, `percentile`, `format`, and `drop_counters` modules; `metrics/output.rs` uses explicit imports, typed-ID architecture checks were retargeted to the split files, the old `src/metrics.rs` oversized allowance was removed, and metrics plus file-size tests pass. |
| 20.2 | Completed | Metrics CPU maps/snapshots now use `CpuId`, `SpikeRecord` stores typed CPU and prior-PID fields, `TaskStats::waker_counts` is keyed by `Tid`, perf-counter selected groups/skips/sample results are keyed by `Tid`, and recorder/session DTO boundaries convert typed IDs back to numeric JSON fields; metrics, perf-counter, recorder-session, TUI, file-size, and typed-ID tests pass. |
| 20.3 | Completed | `architecture_tests::raw_ids` now scans production public fields named `pid`, `tid`, `process_pid`, `task_tid`, `cpu`, or `irq` with raw `u32`/`Option<u32>` types, requires reasoned ABI/DTO boundary exceptions for existing raw surfaces, and passes alongside file-size and typed-ID architecture tests. |
| 21.1 | Completed | `report/analysis/timing.rs` is split into `timing/mod.rs` plus KMS, DRM fence, cross-GPU, Wayland, dmabuf, and GPU-engine modules; the stale oversized allowance was removed, and report plus file-size tests pass. |
| 21.2 | Completed | Added shared `EvidenceQuality` to every hardware timing summary and the scanout estimate, with builders classifying direct, derived, approximate, and missing evidence; focused timing tests, `stutter-report`, broad report, and file-size tests pass. |
| 22.1 | Completed | `report/render/text.rs` is now `report/render/text/` with focused header, summary, pressure, runtime, cluster, correlation, frame, and diagnosis modules; renderer functions are under 100 lines, the old oversized allowance was removed, and report plus file-size tests pass. |
| 22.2 | Completed | Added `ReportTextWriter { lines, section_depth }` and migrated the main text report renderer modules away from `pushln` string assembly; report snapshot/tests and file-size architecture tests pass. |
| 23.1 | Completed | `mangohud.rs` is split into `mangohud/mod.rs`, `schema`, `parser`, `tail`, `alignment`, `plausibility`, and focused tests; parser tests compile without alignment internals, the old oversized allowance was removed, and MangoHud plus file-size tests pass. |
| 23.2 | Completed | Added table-driven MangoHud parser fuzz cases for missing frametime headers, duplicate headers, extra columns, quoted commas, invalid frametime rows, and nanosecond elapsed units; MangoHud and file-size tests pass. |
| 24.1 | Completed | `display_path_compare.rs` is split into `display_path_compare/` modules for model, validation, evidence, verdict, and rendering; comparison logic and printing are separated, the old oversized allowance was removed, and display-path plus file-size tests pass. |
| 24.2 | Completed | Added typed `DisplayPathVerdictReason` values to display-path comparison output, render text, and fixture-backed tests covering cross-GPU fence, iGPU activity, missing evidence, and same-scanout reasons; display-path plus file-size tests pass. |
| 25.1 | Completed | `affinity.rs` is split into `affinity/` modules for CPU masks, syscall access, restore records, restore-file persistence, and tests; CPU-mask parsing is isolated from syscall code, the old oversized allowance was removed, and affinity plus file-size tests pass. |
| 25.2 | Completed | Affinity `sched_getaffinity`, `sched_setaffinity`, and `cpu_set_t` manipulation now live in `affinity/syscall.rs` with local `SAFETY:` comments and `io::Result` wrappers; mutation-boundary, typed-ID, unsafe-safety, affinity, and file-size tests pass. |
| 25.3 | Completed | `profiles.rs` is split into `profiles/` modules for evaluation, planning, application, verification, I/O-priority policy, matching, summary, parsing, rendering, warnings, and tests; profile evaluation and application are separated, the old oversized allowance was removed, and profile plus architecture tests pass. |
| 26.1 | Completed | `watch.rs` is split into `watch/` modules for process resolution, tree-root bookkeeping, pure process matching, profile application, policy, restore, and tests; process matching is testable without profile application, the old oversized allowance was removed, and watch plus file-size tests pass. |
| 26.2 | Completed | Added typed `ProcessMatchDecision` and `ProcessMatchReason` values for watch-process selection while preserving the existing PID helper; watch resolution logs score and reason labels, tests assert executable-basename explanation, and watch plus file-size tests pass. |
| 27.1 | Completed | Top-level `agent` is now `agent/mod.rs` wiring plus focused modules for binding/startup, auth, config, state, rate limiting, artifacts, responses, routes, and server code; the old `src/agent.rs` allowance was removed, direct-print scanning follows the split directory, and agent plus architecture tests pass. |
| 27.2 | Completed | Added a table-driven `RouteAuthExpectation` matrix covering every advertised agent route, asserting missing-auth behavior and non-loopback rejection for state-changing apply/restore/control routes; agent security, full agent, and file-size tests pass. |
| 27.3 | Completed | Added agent response JSON schema snapshots for capabilities, daemon status, autotune start rejection, restore, and config responses using real serializers and stable type/field snapshots; schema, full agent, and file-size tests pass. |
| 28.1 | Completed | `hwmon.rs` is split into `hwmon/` modules for model types, discovery/probe reporting, cached sensor reads, NVIDIA fallback classification, and fixture tests; fake hwmon discovery remains testable, the old oversized allowance was removed, and hwmon plus file-size tests pass. |
| 28.2 | Completed | `perf_counters` is split into `mod`, `group`, `sample`, `limits`, and `syscall` modules; root sampler wiring is 149 lines, unsafe perf syscalls only appear in `perf_counters/syscall.rs`, and `perf_counter`, `unsafe_safety`, and `file_size` focused tests pass. |
| 28.3 | Completed | Hardware probe fixtures cover AMD GPU, Intel CPU, missing labels, permission-denied reads, and malformed values under `stutter/tests/fixtures/hwmon`; `hwmon` focused tests pass. |
| 29.1 | Completed | `wayland_probe` is split into `mod`, `ffi`, `memfd`, `protocol`, and `snapshot`; unsafe Wayland/memfd calls are isolated in `ffi.rs` and `memfd.rs`, and `unsafe_safety` passes. |
| 29.2 | Completed | Foreground snapshots/events now carry structured `ForegroundDecision` data with target, numeric confidence, reasons, and rejected-candidate slots; report/recorder/TUI/focus call sites read structured decisions, legacy flat foreground event JSON migrates during deserialization, and the foreground, recorder session-file, unsafe-safety, perf-counter, and file-size focused tests pass. |
| 30.1 | Completed | Added `RuleSpecificity` struct and calculated specificity fields during Ananicy import; emits `log::warn!` for overbroad rules containing regex or wildcard characters. |
| 30.2 | Completed | Added conflict detection in `db.rs` `merge_file`; emits `log::warn!` when the same normalized rule name maps to different stutter classes. |
| 31.1 | Completed | `xtask/src/main.rs` is already split into workflow/process/no-allow/dependency/eBPF/fixture/preflight/maturity modules and is below the old baseline. |
| 31.2 | Completed | `cargo xtask maturity-report` now prints largest files, architecture-gate drift, unwrap/panic/unsafe/TODO debt counts, scaffold crate status, and test attribute count. |
| 32.1 | Completed | `report/render/text.rs` is no longer in the runtime unwrap/expect allowlist; the runtime allowlist is empty and `architecture_tests::unwrap_expect` passes. |
| 32.2 | Completed | `events/interpret.rs` is no longer in the runtime unwrap/expect allowlist; malformed-event paths are handled without a runtime allowlist exception. |
| 32.3 | Completed | `probe_registry.rs` is no longer in the runtime unwrap/expect allowlist; remaining test/fixture-only allowances are categorized separately. |
| 32.4 | Completed | `diagnosis.rs` is no longer in the runtime unwrap/expect allowlist; diagnosis code is covered by the empty runtime allowlist gate. |
| 32.5 | Completed | `tune/mod.rs` is no longer in the runtime unwrap/expect allowlist; runtime unwrap/expect debt is enforced at zero. |
| 32.6 | Completed | The affinity split moved runtime code out of the old `affinity.rs` allowlist path, and no affinity runtime unwrap/expect allowance remains. |
| 32.7 | Completed | CLI parser tests are moved out of production files and removed from `CFG_TEST_OR_FIXTURE_UNWRAP_EXPECT_FILE_ALLOWLIST`, enforcing unwrap/expect invariants on remaining `unwrap()` calls. |
| 33.1 | Completed | `RollbackTokenKindError`, `RollbackToken::kind`, and `ActionError::invalid_rollback_token` now preserve expected/actual rollback token kinds in structured action errors. |
| 33.2 | Completed | Production rollback token mismatch branches now return structured invalid-token errors, and `actions/transaction.rs` tests report expected/actual kinds without panic-only mismatch handling. |
| 33.3 | Completed | The new global production panic scanner covers action, daemon, autotune, and agent modules outside cfg-test code. |
| 34.1 | Completed | `docs/UNSAFE_INVENTORY.md` records current non-eBPF unsafe usage with owners and migration targets. |
| 34.2 | Completed | eBPF byte-to-event decoding is centralized in `events/decode.rs`; `read_event_unaligned_checked` now validates required size/layout before the single audited unaligned read and reports short-input layout details in tests. |
| 34.3 | Completed | `runtime_slices` now wraps `_SC_CLK_TCK` behind `clock_ticks_per_second() -> io::Result<u64>`, rejects invalid sysconf values explicitly, and falls back deliberately to the Linux default in sampler construction. |
| 35.1 | Completed | Process snapshots now expose `ProcessMap`, `ProcessSet`, and `TaskMap` aliases keyed by `Pid`/`Tid`; `TaskTracker` active/known targets and live `TaskStatsMap` use typed task IDs while raw `u32` remains at `/proc`, eBPF, and artifact DTO boundaries. Typed-ID/raw-ID/file-size architecture gates plus process-tree, process-snapshot, task-refresh, metrics, perf-counter, TUI, profiles, and recording/reporting focused tests pass. |
| 35.2 | Completed | `ScanBudgetReport` now carries bounded structured procfs warnings for disappeared process records, missing cmdline/cgroup/exe evidence, and missing task `comm`; fake procfs tests cover those racey cases and `process_tree::tests`, process-snapshot regressions, and file-size checks pass. |
| 36.1 | Completed | Session and metadata artifacts now carry a typed `ArtifactSchemaVersion` newtype while preserving numeric JSON encoding; loaders still warn on older schema versions and reject newer ones. |
| 36.2 | Completed | Config parsing defaults missing versions to v1, accepts `schema_version` as an alias for `config_version`, rejects conflicting/future versions, and routes parsed files through a migration hook. |
| 37.1 | Completed | Autotune controller tests are split into policy/candidate-result/cooldown/rollback files, and CLI report tests are split into args/diff/render/errors helpers under `cli/tests/report`; report CLI and file-size gates pass. |
| 37.2 | Completed | `daemon::safety_acceptance::tests::mutation_safety_acceptance_suite` covers observe/suggest non-mutation, apply-low safety rejection, remote non-loopback rejection, rollback token/audit behavior, and stale-token startup recovery. |
| 37.3 | Completed | `docs/EBPF_TESTING.md` now includes a release privileged smoke recipe with `privileged-ebpf-smoke`, `sudo stutter doctor`, and a short monitor run. |
| 38.1 | Completed | Direct dependency versions from member manifests now live in `[workspace.dependencies]`; member crates use `{ workspace = true }` with local feature choices. |
| 38.2 | Completed | `cargo xtask dependency-hygiene` now reports default-feature dependencies, duplicate-version baseline, optional dependency feature wiring, and network/TLS dependency surface. |
| 39.2 | Completed | `docs/EBPF_EVENT_LOSS_STRESS.md` documents high-wakeup churn, small/large ring-buffer, many-thread, and CPU-accounting stress checks. |
| 39.1 | Completed | Added `#[test] #[ignore = "benchmark"]` for MangoHud, NDJSON loading, and 5k process snapshot scanning. |
| 40.1 | Completed | `docs/SUBSYSTEM_OWNER_CONTRACTS.md` defines ownership, forbidden behavior, error shape, and protective tests for actions, daemon, autotune, eBPF, report, config, and agent. |
| 40.2 | Completed | `docs/DEGRADED_EVIDENCE.md` explains optional artifacts, DRM/KMS gaps, MangoHud alignment, CPU accounting limits, tracepoint mismatches, and drop counters. |
| 41.1 | Completed | Lowered `PRODUCTION_RUST_FILE_SIZE_LIMIT_LINES` to 700 and updated allowlist. |
| 41.2 | Completed | Emptied `EXISTING_RUNTIME_UNWRAP_EXPECT_FILE_ALLOWLIST` and added invariant comments for `artifacts`, `probe_registry`, `events/interpret`, `autotune/shutdown` and `tune`. |
| 41.3 | Completed | Confirmed `EXISTING_PRODUCTION_PANIC_ALLOWLIST` is empty and zero production panics exist. |
| 41.4 | Completed | Recorder metadata now calls safe syscall wrappers for `clock_gettime` and `uname`; `architecture_tests::unsafe_safety` enforces production app unsafe code stays inside syscall/ffi/memfd/decode wrappers and still requires local `SAFETY:` comments across non-eBPF crates. |

---

# Target end state

Current rough ratings from my review:

| Area                             | Current | Target after plan |
| -------------------------------- | ------: | ----------------: |
| `stutter-common` ABI             |     9.0 |               9.4 |
| `stutter-core` typed primitives  |     8.7 |               9.3 |
| `stutter-config` crate           |     7.5 |               8.8 |
| `stutter-report` crate           |     5.8 |               8.5 |
| `stutter-ebpf`                   |     8.5 |               9.0 |
| eBPF userspace loader/preflight  |     8.6 |               9.1 |
| `actions`                        |     8.5 |               9.1 |
| `daemon`                         |     8.7 |               9.2 |
| `autotune`                       |     8.0 |               8.8 |
| `session` / monitor runtime      |     7.8 |               8.7 |
| `config` main crate              |     8.3 |               9.0 |
| `cli`                            |     7.6 |               8.6 |
| `report` main crate              |     8.1 |               8.8 |
| `agent`                          |     8.0 |               8.7 |
| hardware probes                  |     7.6 |               8.4 |
| test/architecture infrastructure | 8.8–9.2 |               9.4 |
| overall                          |     8.2 |           8.9–9.1 |

---

# Phase 0 — Lock the baseline before touching anything

## Step 0.1 — Record current file-size baseline

Bad: many files sit just under the `1_000` line architecture limit, for example:

```text
stutter-ebpf/src/main.rs: 1510 lines
xtask/src/main.rs: 1055 lines
stutter/src/actions/irq_affinity.rs: 993 lines
stutter/src/session/monitor_session.rs: 981 lines
stutter/src/cli/mod.rs: 981 lines
stutter/src/session_io.rs: 972 lines
stutter/src/config/merge.rs: 969 lines
stutter/src/recorder/session.rs: 968 lines
stutter/src/tui.rs: 957 lines
stutter/src/metrics.rs: 952 lines
```

Why: files near the ceiling are hard to review and easy to bloat again.

Change: create a temporary `docs/internal/cleanup-baseline.md` listing top 50 largest files, current LOC, owner subsystem, and target split plan.

Acceptance: every later split has a before/after number.

---

## Step 0.2 — Run full baseline locally [x]

Run:

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt --all
RUSTUP_TOOLCHAIN=nightly cargo test --all
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
cargo xtask workflow
```

Bad: without a clean baseline, refactors become guesswork.

Change: save logs under a local scratch path outside the repo.

Acceptance: baseline is clean or existing failures are documented before refactor commits.

---

## Step 0.3 — Add a “cleanup branch rules” note

Bad: this plan touches many subsystems, so it can easily turn into a giant unreviewable patch.

Change: add `docs/internal/CLEANUP_SEQUENCE.md` with rules:

```text
One behavior-preserving refactor per commit.
No functional changes mixed with file moves.
Every moved module keeps old tests passing before new work starts.
Every removed allowlist entry gets its own commit.
No new #[allow(...)].
No new oversized file allowlist.
```

Acceptance: the cleanup itself has guardrails.

---

# Phase 1 — Remove scratch/prototype files from the repository

## Step 1.1 — Delete root scratch size checker

Bad:

```text
scratch/check_size.rs
```

contains an old standalone `SchedulerEvent` size checker. It duplicates ABI shape from `stutter-common`.

Why: scratch files can drift and mislead future reviewers.

Change: delete `scratch/check_size.rs`.

Better replacement: add a real layout test in `stutter-common` or `stutter/src/architecture_tests/ebpf_layout.rs` that checks `size_of::<SchedulerEvent>()` if that size matters.

Acceptance: no root `scratch/` directory remains.

---

## Step 1.2 — Migrate `stutter/scratch/test_faults.rs`

Bad:

```text
stutter/scratch/test_faults.rs
```

contains a test-like reproduction with comments saying:

```text
ACTUAL (current code): delta = 12 - 12 = 0.
```

Why: this is either stale bug evidence or an unmerged regression test. It should not live in scratch.

Change:

1. Check whether the fault delta bug is already covered in `metrics` or regression tests.
2. If covered, delete the scratch file.
3. If not covered, convert it into a real test under:

```text
stutter/src/metrics/tests.rs
```

or an existing metrics test module.

Acceptance:

* `stutter/scratch/` is gone.
* The intended fault-delta behavior is covered by a real passing test or explicitly documented as obsolete.

---

## Step 1.3 — Add an architecture test forbidding scratch dirs

Bad: nothing prevents scratch files from returning.

Change: add an architecture test that fails if these exist:

```text
scratch/
stutter/scratch/
*/scratch/
```

except maybe under `target/`.

Acceptance: accidental scratch code cannot enter the branch again.

---

# Phase 2 — Tighten architecture gates gradually

## Step 2.1 — Lower the file-size threshold from 1000 to 900 [x]

Bad:

```rust
stutter/src/architecture_tests.rs:26
const RUST_FILE_SIZE_LIMIT_LINES: usize = 1_000;
```

Many files cluster in the 900–999 range. That means the current threshold is acting as a soft target.

Why: developers naturally grow files until the gate stops them.

Change:

1. First split enough files so no non-test production file exceeds 900.
2. Then change the limit to:

```rust
const RUST_FILE_SIZE_LIMIT_LINES: usize = 900;
```

Acceptance: file-size architecture test passes with no oversized allowlist.

---

## Step 2.2 — Later lower file-size threshold from 900 to 800 [x]

Bad: even 900-line files are still large.

Change after Phase 5–15 refactors:

```rust
const RUST_FILE_SIZE_LIMIT_LINES: usize = 800;
```

Acceptance: no production file exceeds 800 lines.

---

## Step 2.3 — Add separate threshold for test files

Bad: test files like:

```text
stutter/src/cli/report/tests.rs: 959 lines
stutter/src/autotune/controller/tests.rs: 967 lines
stutter/src/autotune/rolling_window/tests.rs: 932 lines
```

are huge, but they have different maintainability concerns than production code.

Change: update file-size architecture test to use:

```text
production limit: 800
test file limit: 1000 initially, then 900 later
```

Why: this avoids production files hiding behind large test thresholds.

Acceptance: production size pressure increases without causing needless test churn.

---

## Step 2.4 — Split unwrap/expect allowlist into real production debt vs cfg-test support

Bad: current allowlist mixes true production debt with test modules that live in source files:

```text
src/affinity.rs
src/agent.rs
src/artifacts.rs
src/diagnosis.rs
src/events/interpret.rs
src/probe_registry.rs
src/report/render/text.rs
src/tune/mod.rs
```

and also test-support files.

Why: mixed allowlists hide which unwraps are dangerous.

Change:

1. Rename allowlist to two lists:

   * `EXISTING_RUNTIME_UNWRAP_EXPECT_FILE_ALLOWLIST`
   * `CFG_TEST_OR_FIXTURE_UNWRAP_EXPECT_FILE_ALLOWLIST`
2. Make architecture failure output show the category.
3. Require a stricter reason for runtime entries: must include “why impossible” or “migration target”.

Acceptance: unwrap debt becomes actionable instead of a generic bucket.

---

## Step 2.5 — Add a “no new panic in production” scanner

Bad: there are many `panic!` uses, some in tests, some in production-like modules. Examples include rollback-token mismatch panics in `actions`.

Why: mutation/daemon code should return structured errors, not panic.

Change:

1. Reuse scanner infrastructure from `architecture_tests/scanners.rs`.
2. Ignore `#[cfg(test)]`.
3. Flag `panic!`, `unreachable!`, and `todo!` in production.
4. Allow `unreachable!` only with a preceding `// invariant:` line initially.

Acceptance: no new production panic paths can enter.

---

## Step 2.6 — Add unsafe documentation architecture check

Bad: `unsafe` is necessary in `stutter-ebpf`, syscall wrappers, perf counters, and Wayland FFI, but unsafe blocks are scattered.

Why: every unsafe block should explain the safety contract.

Change:

1. Add scanner requiring `// SAFETY:` immediately before each unsafe block outside `stutter-ebpf`.
2. Start with warning mode in docs.
3. Convert to test once wrappers are migrated.

Acceptance: all non-eBPF unsafe has local safety justification.

---

# Phase 3 — Turn `stutter-report` from scaffold into real code

Current rating: **5.8/10**. Biggest maturity gain.

## Step 3.1 — Rename scaffold comments to migration warnings

Bad:

```text
stutter-report/src/lib.rs
stutter-report/src/load.rs
stutter-report/src/analysis/mod.rs
stutter-report/src/diff/mod.rs
stutter-report/src/render/mod.rs
```

currently says “placeholder” / “future migration”.

Why: the crate exists in the workspace but is not doing the real job. That lowers architecture maturity.

Change: create `docs/REPORT_CRATE_MIGRATION.md` with a file-by-file migration checklist.

Acceptance: migration is explicit and measurable.

---

## Step 3.2 — Move pure report model structs first

Bad: real model lives mostly in `stutter/src/report` and supporting modules.

Why: model structs are the safest first migration target.

Change:

1. Identify report structs that do not depend on runtime internals.
2. Move them to `stutter-report/src/model`.
3. Re-export from main crate temporarily:

```rust
pub(crate) use stutter_report::model::...
```

Acceptance: no behavior change; main crate compiles through re-exports.

---

## Step 3.3 — Move report load input model

Bad: `stutter-report/src/load.rs` returns unsupported operation.

Why: a crate named `stutter-report` should at least load the report model.

Change:

1. Move pure file loading from `stutter/src/session_io.rs` or report loader into `stutter-report`.
2. Keep filesystem access behind `ReportLoadRequest`.
3. Return a real `ReportModel`.

Acceptance:

```rust
load_report_model(...)
```

returns real data for a minimal recording fixture.

---

## Step 3.4 — Move text render in slices [x]

Bad:

```text
stutter/src/report/render/text.rs: 933 lines
```

Why: very large rendering files are hard to review and easy to break.

Change split into:

```text
stutter-report/src/render/text/mod.rs
stutter-report/src/render/text/header.rs
stutter-report/src/render/text/quality.rs
stutter-report/src/render/text/cluster.rs
stutter-report/src/render/text/correlation.rs
stutter-report/src/render/text/frame.rs
stutter-report/src/render/text/diagnosis.rs
```

Acceptance: original `render_report()` output remains byte-for-byte identical under golden tests.

---

## Step 3.5 — Add golden text report tests

Bad: rendering changes are hard to inspect manually.

Change:

1. Add fixture input under `stutter-report/tests/fixtures/minimal`.
2. Add expected `.txt`.
3. Normalize timestamps/paths before comparison.

Acceptance: report rendering migration is protected.

---

## Step 3.6 — Move diff logic from main crate [x]

Bad: `stutter-report/src/diff` currently only compares skeleton models.

Change:

1. Implement diff based on real report model.
2. Support at least:

   * score change
   * p95/p99 latency change
   * top culprit change
   * data quality change
3. Keep current CLI output unchanged initially.

Acceptance: `stutter report diff` uses `stutter-report` logic.

---

## Step 3.7 — Delete scaffold unsupported operation

Bad:

```rust
ReportError::unsupported_operation("load_report_model", ...)
```

in `stutter-report/src/load.rs`.

Change: remove once real loading exists.

Acceptance: no public `stutter-report` path returns “not migrated yet” for core load/analyze/render.

---

# Phase 4 — Make `stutter-config` the real config engine

Current rating: **7.5/10** because it is clean but incomplete.

## Step 4.1 — Inventory current config ownership

Bad: there are two config worlds:

```text
stutter-config/src/*
stutter/src/config/*
stutter/src/config_file/*
```

Why: split ownership risks drift.

Change: document which fields still live only in main crate.

Acceptance: migration checklist exists.

---

## Step 4.2 — Move pure config types into `stutter-config` [x]

Bad:

```text
stutter/src/config/model.rs
stutter/src/config/types.rs
```

are main-crate-local even though many are pure config.

Change:

1. Move pure types first.
2. Keep Linux/runtime-dependent validation in main crate temporarily.
3. Re-export in `stutter/src/config`.

Acceptance: no CLI behavior change.

---

## Step 4.3 — Move validation that has no runtime dependency [x]

Bad:

```text
stutter/src/config/validation.rs
```

mixes pure field validation with eBPF sizing and runtime constraints.

Change:

1. Extract pure validation to `stutter-config`.
2. Keep runtime/environment validation in main crate.
3. Name functions clearly:

```rust
validate_static_config(...)
validate_runtime_config(...)
```

Acceptance: `target.max_tasks` validation lives with config type, not with monitor runtime.

---

## Step 4.4 — Split `config/merge.rs` [x]

Bad:

```text
stutter/src/config/merge.rs: 969 lines
```

Why: merge logic is too large for one file.

Change split into:

```text
stutter/src/config/merge/mod.rs
stutter/src/config/merge/source.rs
stutter/src/config/merge/presence.rs
stutter/src/config/merge/target.rs
stutter/src/config/merge/ebpf.rs
stutter/src/config/merge/autotune.rs
stutter/src/config/merge/daemon.rs
stutter/src/config/merge/tests.rs
```

Acceptance:

* public functions stay:

  * `merge_config_sources_effective_checked`
  * `merge_config_sources_checked`
  * `merge_config_sources_lossy_for_tests`
* file under 800 lines.

---

## Step 4.5 — Add merge conflict diagnostics [x]

Bad: large merge code makes it hard to know why one config value won.

Change:

1. Add optional debug trace object:

```rust
ConfigMergeTrace {
    field: &'static str,
    selected_layer: ConfigLayer,
    reason: MergeReason,
}
```

2. Use only in tests/diagnostics first.

Acceptance: tests can assert field provenance.

---

# Phase 5 — Split `stutter-ebpf/src/main.rs`

Current rating: **8.5/10**, target **9.0**.

## Step 5.1 — Extract map declarations [x]

Bad:

```text
stutter-ebpf/src/main.rs: 1510 lines
```

contains maps, helpers, tracepoints, and implementations.

Change:

1. Create:

```text
stutter-ebpf/src/maps.rs
```

2. Move map declarations only.
3. Keep names unchanged.

Acceptance: no functional changes; eBPF object symbols unchanged.

---

## Step 5.2 — Extract target filtering and counters [x]

Bad: helper functions like:

```rust
is_target_pid
is_target_current_cgroup
increment_drop_counter
valid_cpu
```

live in `main.rs`.

Change:

1. Create:

```text
stutter-ebpf/src/target_filter.rs
stutter-ebpf/src/drop_counters.rs
```

2. Keep functions `#[inline(always)]`.

Acceptance: verifier still accepts generated BPF.

---

## Step 5.3 — Extract runnable-depth accounting [x]

Bad: runnable-depth functions are mixed into scheduler tracepoint code.

Change:

1. Create:

```text
stutter-ebpf/src/runnable_depth.rs
```

2. Move:

   * `read_cpu_runnable_depth`
   * `increment_cpu_runnable_depth`
   * `decrement_cpu_runnable_depth`
   * `mark_task_runnable`
   * `mark_task_running`
   * `remove_runnable_task_if_present`
   * `increment_target_pending`
   * `decrement_target_pending`

Acceptance: `try_sched_wakeup` and `try_sched_switch` read like event logic, not map plumbing.

---

## Step 5.4 — Extract scheduler tracepoints [x]

Bad: scheduler logic is one region inside massive `main.rs`.

Change:

1. Create:

```text
stutter-ebpf/src/scheduler.rs
```

2. Move:

   * `try_sched_wakeup`
   * `try_sched_switch`
   * `try_sched_migrate_task`
   * `try_sched_stat_wait`
   * scheduler tracepoint entry wrappers if possible.

Acceptance: scheduler module has focused tests or architecture source checks.

---

## Step 5.5 — Extract process lifecycle [x]

Change:

1. Create:

```text
stutter-ebpf/src/process_lifecycle.rs
```

2. Move:

   * `sched_process_exec`
   * `try_sched_process_exec`
   * `sched_process_exit`
   * fault perf events if they fit better here.

Acceptance: process lifecycle logic no longer competes with scheduler logic.

---

## Step 5.6 — Extract IRQ tracing [x]

Change:

1. Create:

```text
stutter-ebpf/src/irq.rs
```

2. Move:

   * IRQ maps if local
   * `try_irq_handler_entry`
   * `try_irq_handler_exit`
   * `irq_key`
   * `is_target_irq`

Acceptance: IRQ tracing can be reviewed alone.

---

## Step 5.7 — Extract CPU frequency / thermal-ish tracepoints [x]

Change:

1. Create:

```text
stutter-ebpf/src/cpu_frequency.rs
```

Acceptance: `main.rs` becomes entrypoint/module wiring only.

---

## Step 5.8 — Add eBPF module-size architecture check [x]

Bad: splitting once does not prevent re-growth.

Change: add architecture test or xtask check for `stutter-ebpf/src/*.rs` max 800 lines, then later 600.

Acceptance: eBPF code stays reviewable.

---

# Phase 6 — Harden eBPF semantics further

## Step 6.1 — Add comments for `WakeupData.seq` invariant

Bad: `seq` exists and is good, but future maintainers need to understand why.

Change in:

```text
stutter-ebpf/src/wakeup_data.rs
```

Document:

```text
seq distinguishes repeated wakeups for the same PID so WAKEUP_CONSUMED does not suppress a newer wakeup after an older one was consumed.
```

Acceptance: the ABA prevention design is clear.

---

## Step 6.2 — Add userspace/eBPF layout test for `WakeupData`

Bad: ABI event structs are checked more than internal map payload semantics.

Change: add compile-time layout assertions for map value structs where userspace reads them.

Acceptance: size/alignment drift is detected.

---

## Step 6.3 — Add drop-counter documentation table generation

Bad: drop counters live in `stutter-common`, docs, report interpretation, and eBPF increments.

Why: these can drift.

Change:

1. Define drop counter metadata in one Rust table in `stutter-common`.
2. Generate docs or test docs against the table.
3. Assert every `DROP_*` constant has a label.

Acceptance: adding a drop counter fails tests unless docs/report labels are updated.

---

# Phase 7 — Improve userspace eBPF loader/preflight

## Step 7.1 — Split preflight into tracepoint and system checks

Bad:

```text
stutter/src/ebpf/preflight.rs
```

does both tracepoint format validation and CPU/sysfs diagnostics.

Change:

```text
stutter/src/ebpf/preflight/mod.rs
stutter/src/ebpf/preflight/tracepoints.rs
stutter/src/ebpf/preflight/system.rs
stutter/src/ebpf/preflight/cpu.rs
stutter/src/ebpf/preflight/tests.rs
```

Acceptance: CPU possible parser is isolated and tested.

---

## Step 7.2 — Replace stringly tracepoint status with typed names

Bad: tracepoint names and field names are easy to mistype.

Change:

1. Add:

```rust
TracepointName(&'static str)
TracepointFieldName(&'static str)
```

or enums for required tracepoints.

Acceptance: required tracepoint lists are typed data, not ad-hoc tuples.

---

## Step 7.3 — Add kernel tracepoint snapshot command

Bad: preflight failures require manually inspecting `/sys/kernel/tracing/events/.../format`.

Change: add diagnostic command:

```bash
stutter doctor tracepoints --dump
```

Acceptance: bug reports include exact kernel tracepoint formats.

---

# Phase 8 — Reduce `actions` risk and size

## Step 8.1 — Split `actions/irq_affinity.rs`

Bad:

```text
stutter/src/actions/irq_affinity.rs: 993 lines
```

It is right at the old ceiling.

Change split into:

```text
stutter/src/actions/irq_affinity/mod.rs
stutter/src/actions/irq_affinity/model.rs
stutter/src/actions/irq_affinity/validate.rs
stutter/src/actions/irq_affinity/sysfs.rs
stutter/src/actions/irq_affinity/apply.rs
stutter/src/actions/irq_affinity/tests.rs
```

Acceptance: no file exceeds 500 lines.

---

## Step 8.2 — Replace rollback-token panics with typed accessors

Bad examples in `stutter/src/actions/token.rs` tests are okay, but production modules also have “unexpected rollback token” panic patterns.

Why: action rollback should return an error if given the wrong token, not panic.

Change:

1. Add methods:

```rust
RollbackToken::as_irq_affinity_restore(&self) -> Option<&[IrqAffinityRestoreRecord]>
RollbackToken::into_irq_affinity_restore(self) -> Result<Vec<IrqAffinityRestoreRecord>, RollbackTokenKindError>
```

2. Replace ad-hoc pattern + `panic!` in action rollback code.

Acceptance: production rollback code has no `panic!("unexpected rollback token")`.

---

## Step 8.3 — Create `ActionSyscall` wrapper layer

Bad: syscall/FFI code exists in multiple action files:

```text
actions/ioprio.rs
actions/nice.rs
actions/uclamp.rs
affinity.rs
```

Why: unsafe calls should be concentrated.

Change:

```text
stutter/src/actions/syscalls.rs
```

with wrappers:

```rust
setpriority_process(...)
ioprio_get_process(...)
ioprio_set_process(...)
sched_getattr(...)
sched_setattr(...)
```

Acceptance: action files call safe wrappers and contain no direct `libc::syscall`.

---

## Step 8.4 — Add `// SAFETY:` comments to all action unsafe blocks

Bad: syscall unsafe is currently scattered.

Change: after wrappers exist, document each unsafe once in wrapper layer.

Acceptance: unsafe architecture check passes for action files.

---

## Step 8.5 — Normalize action preflight result shape

Bad: each action has its own preflight warning/error vocabulary.

Change:

1. Add shared:

```rust
ActionPreflightReport {
    action_id: ActionId,
    blockers: Vec<ActionBlocker>,
    warnings: Vec<ActionWarning>,
}
```

2. Migrate one action at a time.

Acceptance: daemon/action runner can render consistent preflight failures.

---

# Phase 9 — Mature daemon policy and privilege boundaries

## Step 9.1 — Split `daemon/policy/evaluate.rs`

Bad: policy evaluation is correctness-critical and dense.

Change:

```text
daemon/policy/evaluate/mod.rs
daemon/policy/evaluate/mode.rs
daemon/policy/evaluate/risk.rs
daemon/policy/evaluate/remote.rs
daemon/policy/evaluate/target.rs
daemon/policy/evaluate/reason.rs
```

Acceptance: each file answers one policy question.

---

## Step 9.2 — Add policy decision snapshot tests

Bad: policy changes can silently alter daemon behavior.

Change: create golden JSON snapshots:

```text
tests/fixtures/policy/observe.json
tests/fixtures/policy/apply_low_risk.json
tests/fixtures/policy/remote_non_loopback.json
```

Acceptance: policy changes produce visible diff.

---

## Step 9.3 — Add mutation path architecture check

Bad: safety depends on all mutations going through `actions/runner` and `DaemonPolicy`.

Change: architecture test scans for:

* direct `setpriority`
* direct `sched_setaffinity`
* direct writes to known sysfs tuning paths
* direct cgroup writes

outside allowed modules.

Acceptance: new privileged mutation paths cannot bypass policy.

---

# Phase 10 — Autotune objective decomposition

## Step 10.1 — Split `autotune/objective.rs`

Bad:

```text
stutter/src/autotune/objective.rs: 914 lines
```

and contains many objectives in one file.

Change split into:

```text
autotune/objective/mod.rs
autotune/objective/common.rs
autotune/objective/stutter_score.rs
autotune/objective/game_frame_pacing.rs
autotune/objective/game_runnable_latency.rs
autotune/objective/desktop_interactivity.rs
autotune/objective/browser_interactivity.rs
autotune/objective/compile_throughput.rs
autotune/objective/io_latency.rs
autotune/objective/irq_overlap.rs
autotune/objective/thermal.rs
```

Acceptance: each objective can be reviewed independently.

---

## Step 10.2 — Resolve the compile-throughput TODO

Bad:

```text
stutter/src/autotune/objective.rs:320
// TODO(compile_progress_intervals): add a direct compile-throughput signal and make it primary.
```

Why: a TODO in objective selection means the compile objective may optimize proxies instead of throughput.

Change:

1. Define `CompileThroughputSignal`.
2. Source it from progress intervals if available.
3. Fall back to current proxy only with explicit degraded-quality reason.
4. Add tests for:

   * direct signal wins
   * missing signal falls back
   * fallback is marked lower confidence

Acceptance: TODO removed and compile objective has primary signal.

---

## Step 10.3 — Add objective explanation object

Bad: objective comparison can be correct but hard to explain.

Change:

```rust
ObjectiveDecision {
    outcome: ObjectiveOutcome,
    primary_signal: ObjectiveSignal,
    guard_failures: Vec<ObjectiveGuardFailure>,
    confidence: Confidence,
}
```

Acceptance: controller/report can explain exactly why a candidate was kept/reverted.

---

# Phase 11 — Autotune candidate memory hardening

## Step 11.1 — Split `candidate_memory.rs`

Bad:

```text
stutter/src/autotune/candidate_memory.rs: 871 lines
```

Change:

```text
autotune/candidate_memory/mod.rs
autotune/candidate_memory/model.rs
autotune/candidate_memory/key.rs
autotune/candidate_memory/decay.rs
autotune/candidate_memory/diagnostics.rs
autotune/candidate_memory/persistence.rs
```

Acceptance: key generation, decay, persistence, and diagnostics are separate.

---

## Step 11.2 — Make workload identity typed

Bad: candidate memory likely relies on normalized strings and hashes.

Why: cache keys are safety-critical; string normalization bugs can poison tuning memory.

Change:

1. Introduce:

```rust
WorkloadIdentity
CandidateMemoryKey
ExecutableFingerprint
```

2. Construct through validated constructors only.

Acceptance: candidate memory no longer accepts arbitrary raw key strings at call sites.

---

## Step 11.3 — Add collision diagnostics

Bad: stable hashes can collide theoretically, and normalized identities can merge unrelated workloads.

Change:

1. Store human-readable identity summary alongside hash.
2. On load, detect same hash with incompatible summary.
3. Mark memory entry degraded instead of trusting it.

Acceptance: impossible-to-debug tuning memory corruption becomes visible.

---

# Phase 12 — Autotune runtime/controller split

## Step 12.1 — Split `autotune/runtime.rs`

Bad:

```text
stutter/src/autotune/runtime.rs: 823 lines
```

Change:

```text
autotune/runtime/mod.rs
autotune/runtime/start.rs
autotune/runtime/tick.rs
autotune/runtime/stop.rs
autotune/runtime/restore.rs
autotune/runtime/state_snapshot.rs
autotune/runtime/plan_output.rs
```

Acceptance: runtime lifecycle transitions are separated.

---

## Step 12.2 — Add explicit runtime state machine

Bad: active experiment states are spread across controller/runtime.

Change:

```rust
enum AutotuneRuntimePhase {
    Idle,
    ObservingBaseline,
    Planning,
    DryRun,
    ApplyingCandidate,
    MeasuringCandidate,
    KeepingCandidate,
    RevertingCandidate,
    Faulted,
}
```

Acceptance: invalid transitions are impossible or logged as faults.

---

## Step 12.3 — Add state-transition tests

Bad: tests cover many cases, but transition legality should be centralized.

Change: table-driven tests:

```text
Idle -> ObservingBaseline allowed
MeasuringCandidate -> KeepingCandidate allowed
MeasuringCandidate -> ApplyingCandidate denied
Faulted -> ApplyingCandidate denied
```

Acceptance: runtime state safety is visible.

---

# Phase 13 — Workload policy cleanup

## Step 13.1 — Split `autotune/workload_policy.rs`

Bad:

```text
stutter/src/autotune/workload_policy.rs: 899 lines
```

Change:

```text
autotune/workload_policy/mod.rs
autotune/workload_policy/model.rs
autotune/workload_policy/defaults.rs
autotune/workload_policy/parse.rs
autotune/workload_policy/lint.rs
autotune/workload_policy/match.rs
```

Acceptance: parsing, linting, and matching are separate.

---

## Step 13.2 — Make policy lints structured

Bad: string lints are hard to assert and render consistently.

Change:

```rust
enum WorkloadPolicyLintKind {
    UnknownFamily,
    UnsupportedObjective,
    MediumRiskSystemWideDenied,
    DuplicateRule,
}
```

Acceptance: tests assert lint kind, not only message text.

---

# Phase 14 — Topology-aware profile candidate cleanup

## Step 14.1 — Split `autotune/planning/profile_candidates.rs` [x]

Bad:

```text
stutter/src/autotune/planning/profile_candidates.rs: 887 lines
```

Change:

```text
autotune/planning/profile_candidates/mod.rs
autotune/planning/profile_candidates/topology.rs
autotune/planning/profile_candidates/rules.rs
autotune/planning/profile_candidates/validate.rs
autotune/planning/profile_candidates/gaming.rs
autotune/planning/profile_candidates/helpers.rs
```

Acceptance: generated profile rules can be tested per strategy.

---

## Step 14.2 — Add generated-profile invariant object [x]

Bad: generated profile validation is scattered across helper functions.

Change:

```rust
GeneratedProfileInvariants {
    render_has_cpu: bool,
    compositor_has_cpu: bool,
    background_capacity_ok: bool,
    no_empty_masks: bool,
}
```

Acceptance: validation failures become reportable, not just boolean rejection.

---

# Phase 15 — Session and monitor runtime cleanup

## Step 15.1 — Split `session/monitor_session.rs` [x]

Bad:

```text
stutter/src/session/monitor_session.rs: 981 lines
```

and it orchestrates many unrelated parts.

Change:

```text
session/monitor_session/mod.rs
session/monitor_session/startup.rs
session/monitor_session/probes.rs
session/monitor_session/targets.rs
session/monitor_session/event_loop.rs
session/monitor_session/exporters.rs
session/monitor_session/shutdown.rs
session/monitor_session/display.rs
```

Acceptance: event loop is readable without probe setup noise.

---

## Step 15.2 — Introduce `MonitorRuntimeHandles` [x]

Bad: long session constructors often accumulate many handles/guards.

Change:

```rust
struct MonitorRuntimeHandles {
    ebpf: EbpfHandles,
    recorder: Option<RecorderHandle>,
    exporters: ExporterHandles,
    target_refresh: TargetRefreshHandle,
}
```

Acceptance: startup returns a single typed bundle.

---

## Step 15.3 — Add shutdown ordering tests [x]

Bad: shutdown bugs in monitor sessions can leak probes, recorders, or restore handles.

Change:

1. Add fake handles with drop/order recording.
2. Assert shutdown order:

   * stop event ingestion
   * flush recorder/exporters
   * detach probes
   * final report

Acceptance: shutdown order is intentional.

---

# Phase 16 — Split `session_io.rs`

## Step 16.1 — Split artifact loading

Bad:

```text
stutter/src/session_io.rs: 972 lines
```

does JSON loading, NDJSON loading, artifact path resolution, validation, consistency checks, DRM data quality.

Change:

```text
session_io/mod.rs
session_io/load_json.rs
session_io/load_ndjson.rs
session_io/paths.rs
session_io/artifact_counts.rs
session_io/consistency.rs
session_io/drm_quality.rs
session_io/run_artifacts.rs
```

Acceptance: artifact validation can be reviewed without loader plumbing.

---

## Step 16.2 — Use typed artifact paths

Bad: path construction is easy to get wrong with strings.

Change:

```rust
RunDir
ArtifactPath
ArtifactKind
```

Acceptance: call sites no longer manually join common artifact filenames.

---

# Phase 17 — Recorder cleanup

## Step 17.1 — Split `recorder/session.rs`

Bad:

```text
stutter/src/recorder/session.rs: 968 lines
```

Change:

```text
recorder/session/mod.rs
recorder/session/prepare.rs
recorder/session/finalize.rs
recorder/session/metadata.rs
recorder/session/path.rs
recorder/session/time.rs
recorder/session/write.rs
recorder/session/warnings.rs
```

Acceptance: prepare/finalize can be tested separately.

---

## Step 17.2 — Make recording warnings structured

Bad: warning strings are hard to test.

Change:

```rust
enum RecordingWarningKind {
    ExistingRunDir,
    KernelTooOld,
    MissingDisplayTopology,
    MissingWaylandPresentation,
}
```

Acceptance: tests assert warning kind.

---

# Phase 18 — CLI cleanup

## Step 18.1 — Split `cli/mod.rs`

Bad:

```text
stutter/src/cli/mod.rs: 981 lines
```

Change:

```text
cli/mod.rs
cli/app.rs
cli/parse.rs
cli/version.rs
cli/config_bridge.rs
cli/help.rs
```

Acceptance: `mod.rs` mostly wires submodules.

---

## Step 18.2 — Move parser tests out of production files

Bad: unwrap/expect allowlist includes CLI files because tests live inside source modules.

Change:

1. Move large test modules into:

```text
stutter/src/cli/tests/*
```

2. Keep `#[cfg(test)] mod tests;`.

Acceptance: CLI files can leave unwrap/expect allowlist.

---

## Step 18.3 — Add CLI snapshot tests

Bad: command shape can drift accidentally.

Change:

1. Use `clap` debug output or help output snapshots.
2. Add snapshots for:

   * top-level help
   * monitor help
   * daemon help
   * autotune help

Acceptance: CLI changes are deliberate.

---

# Phase 19 — TUI cleanup

## Step 19.1 — Split `tui.rs`

Bad:

```text
stutter/src/tui.rs: 957 lines
```

Change:

```text
tui/mod.rs
tui/terminal.rs
tui/status.rs
tui/task_table.rs
tui/sparkline.rs
tui/cpu_heat.rs
tui/autotune_panel.rs
tui/diagnosis.rs
```

Acceptance: rendering units are individually testable.

---

## Step 19.2 — Add render model layer

Bad: TUI likely formats directly from runtime state.

Change:

```rust
TuiModel
TuiTaskRow
TuiAutotunePanel
TuiDiagnosisLine
```

Acceptance: formatting can be tested without terminal.

---

# Phase 20 — Metrics typed-ID migration

## Step 20.1 — Split `metrics.rs`

Bad:

```text
stutter/src/metrics.rs: 952 lines
```

Change:

```text
metrics/mod.rs
metrics/task_stats.rs
metrics/cpu_stats.rs
metrics/drop_counters.rs
metrics/interval.rs
metrics/format.rs
metrics/percentile.rs
```

Acceptance: stats model and formatting are separate.

---

## Step 20.2 — Replace raw PID/TID fields where not ABI-bound

Bad examples:

```text
stutter/src/metrics.rs: pub cpu: u32
stutter/src/metrics.rs: waker_counts: BTreeMap<u32, u64>
stutter/src/perf_counters.rs: BTreeMap<u32, ...>
```

Why: raw IDs cause PID/TID/CPU mixups.

Change:

1. Use `stutter-core` typed IDs:

   * `TaskId`
   * `ProcessId`
   * `CpuId`
2. Keep raw `u32` only at ABI decode boundary.

Acceptance: decoded events convert raw IDs once, then internal code uses typed IDs.

---

## Step 20.3 — Add architecture test for new raw PID/TID fields

Bad: typed-ID migration can regress.

Change: scanner flags new fields named:

```text
pid
tid
process_pid
task_tid
cpu
irq
```

with raw `u32` outside allowed ABI/FFI modules.

Acceptance: new raw IDs require an explicit reason.

---

# Phase 21 — Main report analysis cleanup

## Step 21.1 — Split `report/analysis/timing.rs`

Bad:

```text
stutter/src/report/analysis/timing.rs: 903 lines
```

It mixes KMS, DRM fences, Wayland presentation, direct scanout, dmabuf, GPU engine.

Change:

```text
report/analysis/timing/mod.rs
report/analysis/timing/kms.rs
report/analysis/timing/drm_fence.rs
report/analysis/timing/cross_gpu.rs
report/analysis/timing/wayland.rs
report/analysis/timing/dmabuf.rs
report/analysis/timing/gpu_engine.rs
```

Acceptance: hardware-specific timing logic is independently testable.

---

## Step 21.2 — Add degraded-evidence type

Bad: report code must warn when optional evidence is missing or approximate.

Change:

```rust
enum EvidenceQuality {
    Direct,
    Derived,
    Approximate { reason: String },
    Missing { reason: String },
}
```

Acceptance: every hardware timing summary carries evidence quality.

---

# Phase 22 — Text report cleanup

## Step 22.1 — Split `report/render/text.rs`

Bad:

```text
stutter/src/report/render/text.rs: 933 lines
```

Change:

```text
report/render/text/mod.rs
report/render/text/header.rs
report/render/text/summary.rs
report/render/text/pressure.rs
report/render/text/runtime.rs
report/render/text/cluster.rs
report/render/text/correlation.rs
report/render/text/frame.rs
report/render/text/diagnosis.rs
```

Acceptance: each renderer function is under 100 lines.

---

## Step 22.2 — Replace string assembly with small writer object

Bad: many `pushln`-style helpers can create inconsistent formatting.

Change:

```rust
ReportTextWriter {
    lines: Vec<String>,
    section_depth: usize,
}
```

Acceptance: headings/blank lines are consistent.

---

# Phase 23 — MangoHud parser cleanup

## Step 23.1 — Split `mangohud.rs`

Bad:

```text
stutter/src/mangohud.rs: 924 lines
```

Change:

```text
mangohud/mod.rs
mangohud/schema.rs
mangohud/parser.rs
mangohud/tail.rs
mangohud/alignment.rs
mangohud/plausibility.rs
```

Acceptance: CSV parser tests do not require alignment logic.

---

## Step 23.2 — Add fuzz-style parser tests

Bad: MangoHud CSV formats vary.

Change: add table tests for:

* missing headers
* duplicate headers
* extra columns
* quoted commas
* invalid frametime
* unit changes

Acceptance: parser rejects/degrades predictably.

---

# Phase 24 — Display path compare cleanup

## Step 24.1 — Split `display_path_compare.rs`

Bad:

```text
stutter/src/display_path_compare.rs: 924 lines
```

Change:

```text
display_path_compare/mod.rs
display_path_compare/model.rs
display_path_compare/validate.rs
display_path_compare/evidence.rs
display_path_compare/verdict.rs
display_path_compare/render.rs
```

Acceptance: comparison logic and printing are separate.

---

## Step 24.2 — Make verdict reasons typed

Bad: display verdicts can become stringly and inconsistent.

Change:

```rust
enum DisplayPathVerdictReason {
    SameScanoutGpu,
    CrossGpuFenceDetected,
    IgpuEngineActive,
    TopologyMismatch,
    MissingEvidence,
}
```

Acceptance: tests assert verdict reason.

---

# Phase 25 — Affinity/profile cleanup

## Step 25.1 — Split `affinity.rs`

Bad:

```text
stutter/src/affinity.rs: 867 lines
```

and it mixes CPU mask parsing, affinity syscalls, restore-state file persistence, and tests.

Change:

```text
affinity/mod.rs
affinity/cpu_mask.rs
affinity/syscall.rs
affinity/restore_record.rs
affinity/restore_file.rs
affinity/tests.rs
```

Acceptance: CPU mask parser can be used without syscall code.

---

## Step 25.2 — Move affinity unsafe into syscall wrapper

Bad:

```text
stutter/src/affinity.rs
```

contains `libc::cpu_set_t` manipulation and `sched_setaffinity`.

Change:

1. `affinity/syscall.rs` owns all unsafe.
2. Add `// SAFETY:` comments.
3. Public functions return `io::Result`.

Acceptance: main affinity logic has no unsafe blocks.

---

## Step 25.3 — Split `profiles.rs`

Bad:

```text
stutter/src/profiles.rs: 860 lines
```

Change:

```text
profiles/mod.rs
profiles/evaluate.rs
profiles/apply.rs
profiles/plan.rs
profiles/verify.rs
profiles/ioprio.rs
profiles/matching.rs
profiles/summary.rs
```

Acceptance: profile evaluation and profile application are separate.

---

# Phase 26 — Watch/apply profile cleanup

## Step 26.1 — Split `watch.rs`

Bad:

```text
stutter/src/watch.rs: 933 lines
```

Change:

```text
watch/mod.rs
watch/resolve.rs
watch/tree_roots.rs
watch/process_match.rs
watch/apply.rs
watch/policy.rs
watch/restore.rs
```

Acceptance: process matching is testable without applying profiles.

---

## Step 26.2 — Make watch match score explainable

Bad: `process_match_score` can choose a process without enough explanation.

Change:

```rust
ProcessMatchDecision {
    pid: ProcessId,
    score: u32,
    reasons: Vec<ProcessMatchReason>,
}
```

Acceptance: CLI can explain why a process matched.

---

# Phase 27 — Agent/security maturity

## Step 27.1 — Split top-level `agent.rs`

Bad:

```text
stutter/src/agent.rs: 748 lines
```

and there is also `stutter/src/agent/*`.

Change:

```text
agent/mod.rs
agent/server.rs
agent/bind.rs
agent/auth.rs
agent/routes.rs
agent/state.rs
```

Acceptance: top-level agent module is only wiring.

---

## Step 27.2 — Add route-level auth matrix test

Bad: remote apply/restore endpoints are safety-critical.

Change:

```rust
RouteAuthExpectation {
    route,
    method,
    requires_auth,
    rejects_non_loopback_apply,
}
```

Acceptance: every route has a test row.

---

## Step 27.3 — Add response schema tests

Bad: API clients break when JSON shape drifts.

Change: snapshot JSON for:

* capabilities
* status
* autotune start rejection
* restore response
* config response

Acceptance: API drift is deliberate.

---

# Phase 28 — Hardware probe maturity

## Step 28.1 — Split `hwmon.rs`

Bad:

```text
stutter/src/hwmon.rs: 706 lines
```

Change:

```text
hwmon/mod.rs
hwmon/discover.rs
hwmon/read.rs
hwmon/model.rs
hwmon/classify.rs
```

Acceptance: hwmon discovery can be tested against fixture trees.

---

## Step 28.2 — Split `perf_counters.rs` [x]

Bad:

```text
stutter/src/perf_counters.rs: 712 lines
```

and contains unsafe syscalls.

Change:

```text
perf_counters/mod.rs
perf_counters/syscall.rs
perf_counters/group.rs
perf_counters/sample.rs
perf_counters/limits.rs
```

Acceptance: unsafe perf_event_open code is isolated.

---

## Step 28.3 — Add fixture-based sysfs/procfs tests [x]

Bad: hardware probes depend on live machine layout.

Change: add fake trees for:

* hwmon AMD GPU
* hwmon Intel CPU
* missing labels
* permission denied
* malformed numbers

Acceptance: degraded hardware evidence is predictable.

---

# Phase 29 — Wayland/foreground maturity

## Step 29.1 — Split `wayland_probe.rs` [x]

Bad:

```text
stutter/src/wayland_probe.rs: 545 lines
```

with FFI/memfd unsafe mixed into probe logic.

Change:

```text
wayland_probe/mod.rs
wayland_probe/ffi.rs
wayland_probe/memfd.rs
wayland_probe/protocol.rs
wayland_probe/snapshot.rs
```

Acceptance: unsafe is in `ffi.rs`/`memfd.rs` only.

---

## Step 29.2 — Make foreground confidence explanation structured [x]

Bad: focus/foreground heuristics are necessarily approximate.

Change:

```rust
ForegroundDecision {
    target: Option<ForegroundTarget>,
    confidence: Confidence,
    reasons: Vec<ForegroundReason>,
    rejected_candidates: Vec<RejectedForegroundCandidate>,
}
```

Acceptance: reports can explain foreground decisions.

---

# Phase 30 — Community rules maturity

## Step 30.1 — Add rule specificity score [x]

Bad: community rules can be overbroad.

Change:

```rust
RuleSpecificity {
    exact_exe: bool,
    exact_comm: bool,
    regex_count: usize,
    wildcard_count: usize,
}
```

Acceptance: overbroad rules produce lint warnings.

---

## Step 30.2 — Add community-rule conflict detection [x]

Bad: multiple rules can classify the same process differently.

Change:

1. Detect same matcher with different class.
2. Warn or reject depending on severity.

Acceptance: rules DB quality improves.

---

# Phase 31 — `xtask` cleanup

## Step 31.1 — Split `xtask/src/main.rs`

Bad:

```text
xtask/src/main.rs: 1055 lines
```

Change:

```text
xtask/src/main.rs
xtask/src/workflow.rs
xtask/src/process.rs
xtask/src/no_allow_attrs.rs
xtask/src/dependency_hygiene.rs
xtask/src/ebpf_smoke.rs
xtask/src/fixtures.rs
```

Acceptance: xtask itself follows code-size rules.

---

## Step 31.2 — Add `cargo xtask maturity-report`

Bad: code maturity is currently manually inspected.

Change: command prints:

* largest files
* unwrap allowlist entries
* panic count
* unsafe count without SAFETY
* TODO count
* scaffold crates status
* test count

Acceptance: this plan becomes measurable.

---

# Phase 32 — Remove unwrap/expect allowlist debt

Do this slowly, one file at a time.

## Step 32.1 — Remove `src/report/render/text.rs` from unwrap allowlist

Bad: rendering should not need unwrap.

Change: replace unwraps with:

* `write!` to `String` using `let _ =` only when infallible
* explicit fallback formatting
* `Result` return if truly fallible

Acceptance: file removed from `EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST`.

---

## Step 32.2 — Remove `src/events/interpret.rs` from unwrap allowlist

Bad: event interpretation is input-facing; malformed event bytes should not panic.

Change:

1. Return `Result<EventRecord, EventDecodeError>`.
2. Count/report malformed events.

Acceptance: malformed data degrades quality instead of panicking.

---

## Step 32.3 — Remove `src/probe_registry.rs` from unwrap allowlist

Bad: probe registry should be static and infallible or return structured error.

Change:

1. Replace unwraps with compile-time constants or checked builder.
2. Add test that registry construction cannot fail.

Acceptance: no unwrap/expect.

---

## Step 32.4 — Remove `src/diagnosis.rs` from unwrap allowlist

Bad: diagnosis should never panic on missing evidence.

Change:

1. Replace unwraps with `Option` branches.
2. Add degraded diagnosis reason.

Acceptance: incomplete report input does not panic.

---

## Step 32.5 — Remove `src/tune/mod.rs` from unwrap allowlist

Bad: tune workflow touches files/processes and should return context-rich errors.

Change:

1. Replace unwraps with `anyhow::Context`.
2. Add tests for missing run dir, malformed summary, failed restore.

Acceptance: tune command errors are user-facing.

---

## Step 32.6 — Remove `src/affinity.rs` from unwrap allowlist after split

Bad: CPU mask parsing/restore should not panic.

Change:

1. Make tests live in test modules ignored by production scanner.
2. Replace runtime unwraps with `io::Result`/custom errors.

Acceptance: allowlist entry removed.

---

## Step 32.7 — Remove CLI allowlist entries after test relocation

Bad: CLI is allowlisted because tests live inline.

Change:

1. Move tests to `cli/tests`.
2. Re-run scanner.
3. Remove:

   * `src/cli/mod.rs`
   * `src/cli/monitor.rs`
   * `src/cli/report.rs`

Acceptance: CLI production code has no unwrap debt.

---

# Phase 33 — Panic cleanup

## Step 33.1 — Add `RollbackTokenKindError`

Bad: rollback token mismatch panics are avoidable.

Change:

```rust
pub struct RollbackTokenKindError {
    expected: &'static str,
    actual: &'static str,
}
```

Acceptance: action rollback returns `ActionError::InvalidRollbackToken`.

---

## Step 33.2 — Replace transaction rollback panics

Bad:

```text
stutter/src/actions/transaction.rs
```

has unexpected rollback token panics.

Change: return transaction failure with exact action id and token kind.

Acceptance: no production panic path for rollback mismatch.

---

## Step 33.3 — Add architecture test for panic-free action modules

Change: scanner specifically enforces no `panic!` in:

```text
src/actions
src/daemon
src/autotune
src/agent
```

outside tests.

Acceptance: mutating/control-plane code cannot panic.

---

# Phase 34 — Unsafe cleanup

## Step 34.1 — Build unsafe inventory

Bad: non-eBPF unsafe appears in:

```text
affinity.rs
doctor.rs
events.rs
perf_counters.rs
runtime_slices.rs
wayland_probe.rs
actions/ioprio.rs
actions/nice.rs
actions/uclamp.rs
```

Change: create `docs/UNSAFE_INVENTORY.md`.

Acceptance: every unsafe block has owner and migration target.

---

## Step 34.2 — Wrap `events.rs` byte casting

Bad: event decoding likely uses unsafe bytes-to-struct conversion.

Change:

1. Centralize in:

```text
events/decode.rs
```

2. Add size/alignment checks before casting.
3. Document `// SAFETY:`.

Acceptance: event decode unsafe is one audited block.

---

## Step 34.3 — Wrap `runtime_slices` sysconf

Bad: direct `libc::sysconf`.

Change:

```rust
fn clock_ticks_per_second() -> io::Result<u64>
```

Acceptance: invalid sysconf result is handled.

---

# Phase 35 — Process/procfs correctness

## Step 35.1 — Convert process maps to typed IDs [x]

Bad examples:

```text
BTreeMap<u32, TaskInfo>
BTreeMap<u32, TaskStats>
```

Why: TID/PID mixups are among the most common bugs in tools like this.

Change:

1. Use `TaskId` for Linux task IDs.
2. Use `ProcessId` for process group/main PID.
3. Keep raw `u32` only at `/proc` parse boundary.

Acceptance: compiler catches pid/tid mixups.

---

## Step 35.2 — Add disappearing-task test fixtures [x]

Bad: `/proc` is racey.

Change: fake procfs reader that returns:

* task exists then disappears
* stat exists but cmdline missing
* cgroup unreadable
* exe symlink permission denied

Acceptance: target discovery never panics and produces warnings.

---

# Phase 36 — Config/CLI/report schemas

## Step 36.1 — Add schema version to report artifacts

Bad: artifact JSON changes can break old reports.

Change:

```rust
ArtifactSchemaVersion(u32)
```

in metadata/session files.

Acceptance: loader can reject or migrate old versions.

---

## Step 36.2 — Add config schema version

Bad: config files will drift.

Change:

1. Optional `schema_version`.
2. Default old files to version 1.
3. Add migration hook.

Acceptance: future config changes do not silently reinterpret old config.

---

# Phase 37 — Test suite maturity

## Step 37.1 — [x] Split giant test files by behavior

Bad test files near 1000 lines are hard to navigate.

Change:

* `autotune/controller/tests.rs` into:

  * `policy.rs`
  * `candidate_result.rs`
  * `cooldown.rs`
  * `rollback.rs`
* `cli/report/tests.rs` into:

  * `args.rs`
  * `diff.rs`
  * `render.rs`
  * `errors.rs`

Acceptance: each test file has one behavior theme.

---

## Step 37.2 — [x] Add mutation safety acceptance suite

Bad: safety is spread across tests.

Change: one acceptance suite verifies:

```text
observe mode never mutates
suggest mode never mutates
apply-low rejects medium/high
remote non-loopback apply rejected
rollback token created before mutation
audit event emitted for mutation
startup recovery sees stale active token
```

Acceptance: high-level safety contract is tested in one place.

---

## Step 37.3 — [x] Add privileged smoke script docs

Bad: eBPF/action tests cannot all run in normal CI.

Change: document local privileged validation:

```bash
cargo xtask privileged-ebpf-smoke
sudo stutter doctor
sudo stutter monitor --duration 5s --target-pid $PID
```

Acceptance: release checklist includes real-machine validation.

---

# Phase 38 — Dependency hygiene

## Step 38.1 — [x] Move direct versions into workspace deps

Bad: `stutter/Cargo.toml` has direct versions:

```text
clap_complete = "4"
clap_mangen = "0.2"
inferno = "0.12"
axum = "0.7"
tower = "0.5"
url = "2"
```

Why: workspace-level dependency ownership reduces drift.

Change: move all shared dependency versions to `[workspace.dependencies]`.

Acceptance: member crates mostly use `{ workspace = true }`.

---

## Step 38.2 — Add dependency feature audit

Bad: broad features can creep into the binary.

Change: `cargo xtask dependency-hygiene` reports:

* default features enabled
* duplicate versions
* unused optional features
* network/TLS dependencies

Acceptance: dependency growth is intentional.

---

# Phase 39 — Performance regression guardrails

## Step 39.1 — Add microbench for parsers

Bad: parsers like MangoHud, config, report load, procfs parsing can regress.

Change:

1. Add benchmark-like tests behind ignored flag or criterion optional.
2. Measure:

   * MangoHud 10k rows
   * report load with large NDJSON
   * process snapshot over fake 5k tasks

Acceptance: performance-sensitive parsing has baseline numbers.

---

## Step 39.2 — Add eBPF event-loss stress recipe

Bad: drop counters exist, but stress validation needs a recipe.

Change: add docs/script to run:

* high wakeup churn
* many target threads
* small ringbuf
* large ringbuf
* CPU ID accounting check

Acceptance: ringbuf/map sizing can be validated empirically.

---

# Phase 40 — Documentation maturity

## Step 40.1 — Add “subsystem owner contract” docs

Bad: architecture boundaries exist, but each subsystem should have a small contract.

Change: add docs for:

* actions
* daemon
* autotune
* eBPF
* report
* config
* agent

Each doc answers:

```text
What this subsystem owns
What it must never do
What errors it must return
What tests protect it
```

Acceptance: new contributors know where code belongs.

---

## Step 40.2 — Add “degraded evidence” user docs

Bad: stutter collects many approximate signals; users must understand confidence.

Change: document:

* missing optional artifacts
* DRM/KMS unavailable
* MangoHud alignment uncertainty
* CPU accounting untracked
* tracepoint format mismatches

Acceptance: reports are easier to trust because uncertainty is explicit.

---

# Phase 41 — Final maturity gates

## Step 41.1 — Lower production file limit to 700 [x]

After splits, change:

```rust
const RUST_FILE_SIZE_LIMIT_LINES: usize = 700;
```

Acceptance: core production files remain small.

---

## Step 41.2 — Require zero runtime unwrap/expect allowlist [x]

Final target:

```rust
EXISTING_RUNTIME_UNWRAP_EXPECT_FILE_ALLOWLIST = &[];
```

Test-only fixture allowlist may remain but should be renamed clearly.

Acceptance: production unwrap/expect debt is gone.

---

## Step 41.3 — Require no production panic in safety-critical modules [x]

Safety-critical modules:

```text
actions
daemon
autotune
agent
ebpf userspace loader
session monitor runtime
```

Acceptance: all return structured errors.

---

## Step 41.4 — Require all non-eBPF unsafe to have wrappers [x]

Acceptance:

* unsafe only in:

  * `syscall.rs`
  * `ffi.rs`
  * `decode.rs`
  * `stutter-ebpf`
* each unsafe block has `// SAFETY:`.

---

# Recommended commit order

Do not start with the huge autotune refactor. Start with low-risk cleanup:

1. Delete/migrate scratch files.
2. Add scratch-forbid architecture test.
3. Split `xtask`.
4. Split `stutter-report` scaffold migration docs.
5. Split `cli` tests out of production files.
6. Remove CLI unwrap allowlist entries.
7. Split `affinity`.
8. Add syscall wrappers.
9. Split `actions/irq_affinity`.
10. Split `session_io`.
11. Split `recorder/session`.
12. Split `report/render/text`.
13. Split `metrics`.
14. Split `mangohud`.
15. Split `display_path_compare`.
16. Split `stutter-ebpf/src/main.rs`.
17. Split `autotune/objective`.
18. Resolve compile-throughput TODO.
19. Split candidate memory.
20. Split runtime/controller.
21. Lower file limit to 900.
22. Remove runtime unwrap allowlist entries one by one.
23. Add panic scanner.
24. Add unsafe scanner.
25. Lower file limit to 800.
26. Migrate real report logic to `stutter-report`.
27. Migrate real config logic to `stutter-config`.
28. Lower file limit to 700.

---

# Final validation after each phase

Run:

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt --all
RUSTUP_TOOLCHAIN=nightly cargo test --all
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
cargo xtask workflow
```

For eBPF-affecting phases also run:

```bash
cargo run -p xtask -- ebpf-smoke
cargo xtask preflight
```

For mutation/action phases also run privileged local smoke tests on your Gentoo machine.

---

# Final expected result

By the end of this plan, stutter should be much closer to mature production-grade shape:

```text
No scratch code.
No oversized production files.
No production unwrap/expect allowlist.
No panic paths in safety-critical runtime code.
Unsafe code isolated and documented.
stutter-report is real, not scaffold.
stutter-config owns real config types/validation.
eBPF code is split by tracepoint family.
Autotune objective/runtime/candidate memory are decomposed.
CLI/config/report/session code are smaller and easier to review.
Remote/mutation safety is protected by architecture tests and acceptance tests.
Hardware evidence remains best-effort but explicitly quality-tagged.
```

This is the right kind of slow cleanup for this project: it does not throw away the strong architecture already built; it makes that architecture harder to accidentally violate.
