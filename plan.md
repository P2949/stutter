# Stutter big-file decomposition plan

## Goal

Turn the current large-file architecture into a set of focused, readable, maintainable modules without changing behavior. Every step should be small enough to review by inspection, and every patch should keep the project green under:

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt --all
RUSTUP_TOOLCHAIN=nightly cargo build --all-targets
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=nightly cargo test --all-targets
```

## Implementation progress

- [x] Recognized pre-existing decomposition: `focus/mod.rs` is already a thin facade with focus tests in child modules.
- [x] Recognized pre-existing decomposition: `src/daemon/policy_tests/*`, `src/foreground/tests/*`, `src/actions/runner_tests/*`, `src/autotune/apply_low_risk_tests/*`, and `src/autotune/planner_tests/*` already hold extracted test modules.
- [x] Patch G2-G7: moved `foreground.rs` model, provider, provider implementations, parser helpers, and resolver into existing foreground child modules.
- [x] Patch 1.7/profiles tests: moved embedded `profiles.rs` tests into `src/profiles/tests.rs`.
- [x] Patch 1.7/remaining embedded tests: moved embedded tests out of `diagnosis.rs`, `autotune/runtime.rs`, `ebpf_loader.rs`, `cli/monitor.rs`, `process_tree.rs`, `community_rules.rs`, and `session.rs`.
- [x] Patch profiles/parse-render-warnings: moved profile loading/parsing/validation, TOML rendering/templates, and warning detection into `src/profiles/{parse,render,warnings}.rs`; `profiles.rs` is below 1,000 lines.
- [x] Patch community-rules commands/render: moved command dispatch, command reports, filesystem mutation helpers, and string rendering into `src/community_rules/{commands,render}.rs`; `community_rules.rs` is below 1,000 lines and no longer needs an oversized-file allowlist entry.
- [x] Patch community-rules facade: moved rule models/config/status, loading orchestration, database indexing, name normalization, and classification/context policy into `src/community_rules/{model,load,db,normalize,classify}.rs`; `community_rules.rs` is a 52-line facade.
- [x] Patch process/model: moved scan budgets, process/task models, task classes, target snapshot inputs, filters, and target diff records into `src/process/model.rs`; `process_tree.rs` dropped to 1,422 lines and its stale unwrap/expect allowance was removed.
- [x] Patch process/classify-cgroup: moved built-in task classification predicates into `src/process/classify.rs` and recursive cgroup PID collection into `src/process/cgroup.rs`; `process_tree.rs` is below 1,000 lines and no longer needs an oversized-file allowlist entry.
- [x] Patch process/tree-render: moved render-tree traversal, class suffix formatting, and tree entry ordering into `src/process/tree.rs`; `process_tree.rs` dropped to 818 lines.
- [x] Patch process/snapshot: moved target snapshot construction, diffing, auto-target discovery, task expansion, and task-info/stat parsing helpers into `src/process/snapshot.rs`; `process_tree.rs` dropped to 310 lines.
- [x] Patch process/procfs-facade: moved bounded procfs scans, proc status/stat/cmdline/exe readers, descendant expansion, thread discovery, and task comm reads into `src/process/procfs.rs`; `process_tree.rs` is a 48-line compatibility facade.
- [x] Patch report/analysis foreground: moved foreground/focus summaries and foreground cluster annotation helpers into `src/report/analysis/foreground.rs`; `report/analysis.rs` dropped to 2,490 lines.
- [x] Patch report/analysis timing: moved KMS flip, DRM fence, Wayland presentation, timing proximity, and percentile helpers into `src/report/analysis/timing.rs`; `report/analysis.rs` dropped to 2,135 lines.
- [x] Patch report/analysis task-quality-pressure-clusters-diagnosis: moved task row selection/formatting into `tasks.rs`, data quality and block-I/O labels into `quality.rs`, pressure timelines into `pressure.rs`, spike clustering/wake graphs into `clusters.rs`, and the diagnosis bridge into `diagnosis.rs`; `report/analysis.rs` dropped to 1,177 lines.
- [x] Patch report/analysis density: moved spike-density bucket construction plus ns/ms and percentile helpers into `src/report/analysis/density.rs`; `report/analysis.rs` dropped to 1,090 lines.
- [x] Patch report/analysis correlation: moved text report correlation section construction into `src/report/analysis/correlation.rs`, removed the now-unneeded `report/analysis.rs` oversized-file allowance, and cleaned moved exact unwraps; `report/analysis.rs` dropped to 720 lines.
- [x] Patch report/analysis runtime: moved runtime-slice availability notes, source counts, and top runtime/wait thread ranking into `src/report/analysis/runtime.rs`; `report/analysis.rs` dropped to 580 lines.
- [x] Patch report/analysis format-frame-final: moved shared formatting/cluster label helpers into `src/report/analysis/format.rs` and frame-pacing, frame diagnosis, and correlation-window helpers into `src/report/analysis/frame.rs`; `report/analysis.rs` is a 215-line orchestration module.
- [x] Patch report/analysis architecture cleanup: removed the stale production unwrap/expect allowance for `src/report/analysis.rs` after the moved frame sort switched to `total_cmp`.
- [x] Patch session/targeting: moved `SessionTargetPlan`, initial auto-target/focus resolution, foreground resolver construction, and `needs_tree_tick_from_parts` into `src/session/targeting.rs`; `session.rs` dropped to 2,181 lines.
- [x] Patch session/focus-foreground ticks: moved focus change/clear emission, focus tick handling, foreground identity comparison, and foreground tick handling into `src/session/ticks/{focus,foreground}.rs`; `session.rs` dropped to 2,028 lines.
- [x] Patch session/runtime stages: moved recording, exporter, alert, sampler, UI, and hwmon runtime setup into `src/session/{recording,exporter,alerts,sampler,ui,hwmon}.rs`; `session.rs` dropped to 1,788 lines.
- [x] Patch session/display-timing: moved KMS flip, DRM fence, GPU role, and monotonic timestamp conversion helpers into `src/session/display_timing.rs`; `session.rs` dropped to 1,725 lines.
- [x] Patch session/monitor-session facade: moved `MonitorSession` and its impl into `src/session/monitor_session.rs`, re-exported the type from `session.rs`, removed the stale `session.rs` unwrap allowance, and left `session.rs` as a 268-line facade/top-level entry module.
- [x] Patch daemon/privilege tests: moved the embedded privilege tests into `src/daemon/privilege/tests.rs`; `daemon/privilege.rs` dropped to 1,861 lines.
- [x] Patch daemon/privilege model: moved privilege roles/transports/operations, command request/decision DTOs, candidate plan/apply/rollback request/result DTOs, and privileged-worker IPC request/response models into `src/daemon/privilege/model.rs`; `daemon/privilege.rs` dropped to 1,483 lines.
- [x] Patch daemon/privilege allowlist-audit: moved `PrivilegeCommandAllowlist` and decision helpers into `src/daemon/privilege/allowlist.rs`, and audit event/sink/input records into `src/daemon/privilege/audit.rs`; `daemon/privilege.rs` dropped to 1,268 lines.
- [x] Patch daemon/privilege services: moved `InProcessPrivilegedActionService` and `UnixSocketPrivilegedActionService` into `src/daemon/privilege/{in_process,socket}.rs`; privilege service tests stayed green.
- [x] Patch daemon/privilege worker-revalidation: moved privileged worker process/socket/request execution into `src/daemon/privilege/worker.rs`, live target revalidation into `src/daemon/privilege/revalidate.rs`, and removed the `daemon/privilege.rs` oversized-file allowance; `daemon/privilege.rs` is a 133-line facade.
- [x] Patch ebpf/model: moved the loaded-eBPF handle, block-I/O correlation basis, drop-counter snapshot/report model, map sizing model, and memlock policy report into `src/ebpf/model.rs`; `ebpf_loader.rs` dropped to 2,155 lines.
- [x] Patch ebpf/maps: moved map sizing constants, budget calculations, config overrides, map sizing report construction, and ring-buffer rounding helpers into `src/ebpf/maps.rs`; `ebpf_loader.rs` dropped to 1,977 lines.
- [x] Patch ebpf/memlock-memory: moved memlock limit read/raise/report logic into `src/ebpf/memlock.rs` and memory/page-size/byte-format helpers into `src/ebpf/memory.rs`; `ebpf_loader.rs` dropped to 1,840 lines.
- [x] Patch ebpf/tracepoint-format: moved generic tracepoint format parsing, field-offset validation, required/optional format validation, and layout mismatch errors into `src/ebpf/tracepoint_format.rs`; `ebpf_loader.rs` dropped to 1,556 lines.
- [x] Patch ebpf/tracepoints: moved block I/O, KMS, and DRM fence provider-specific tracepoint offset validation into `src/ebpf/tracepoints/{block_io,kms,drm_fence}.rs`; `ebpf_loader.rs` dropped to 1,287 lines.
- [x] Patch ebpf/attach: moved tracepoint attach helpers, KMS and DRM-fence optional attach dispatch, and software perf-event attach helpers into `src/ebpf/attach.rs`; `ebpf_loader.rs` dropped to 1,131 lines.
- [x] Patch ebpf/object: moved prebuilt object file validation and `STUTTER_BPF_OBJECT`/embedded object byte resolution into `src/ebpf/object.rs`; `ebpf_loader.rs` dropped to 1,083 lines.
- [x] Patch ebpf/load-preflight-facade: moved loader orchestration into `src/ebpf/load.rs`, tracepoint availability/preflight assembly into `src/ebpf/preflight.rs`, reduced `ebpf_loader.rs` to a 99-line compatibility facade, and removed its oversized-file and unwrap/expect allowances.
- [x] Patch autotune/planner model-summary: moved `CandidateEvaluation`, `CandidateEvaluationDraft`, `PlanResult`, planner summary DTOs, and summary impls into `src/autotune/planning/model.rs`; `planner/mod.rs` dropped to 709 lines.
- [x] Patch autotune/planner denial: moved `CandidateDenyReason`, reason-code mapping, denial grouping/name helpers, and denial normalization into `src/autotune/planning/denial.rs`.
- [x] Patch autotune/planner ranking: moved candidate sorting, workload-memory rank adjustment, and no-action reason construction into `src/autotune/planning/ranking.rs`; `planner/mod.rs` dropped to 666 lines.
- [x] Patch autotune/planner policy: moved daemon-policy intent, family gating, workload autonomy, policy-context construction, and rejection reason mapping into `src/autotune/planning/policy.rs`; `planner/mod.rs` dropped to 588 lines.
- [x] Patch autotune/planner evaluate: moved proposal evaluation, dry-run gating, and system-wide allowlist/evidence parsing helpers into `src/autotune/planning/evaluate.rs`; `planner/mod.rs` dropped to 121 lines and no longer needs an oversized-file allowance.
- [x] Patch autotune/planner facade: moved `CandidatePlanner`, `PlannerInput`, and provider orchestration into `src/autotune/planning/planner.rs`; `planner/mod.rs` is now a 17-line compatibility facade.
- [x] Patch autotune/runtime config: moved `AutotuneRuntimeConfig`, runtime-mode daemon config construction, workload-policy resolution, and runtime config validation into `src/autotune/runtime/config.rs`; `runtime.rs` dropped to 1,808 lines.
- [x] Patch autotune/runtime stream: moved decision stream DTOs and stdout JSON emission into `src/autotune/runtime/stream.rs`, then removed the stale direct-print architecture allowance.
- [x] Patch autotune/runtime history-context: moved `RuntimeHistoryContext`, its live-experiment conversion, rollback metadata formatting, and `LifecycleHistoryEventInput` into `src/autotune/runtime/history.rs`; `runtime.rs` dropped to 1,741 lines.
- [x] Patch autotune/runtime daemon-state: moved daemon phase mapping, daemon profile hashing/class/confidence helpers, and history enum mapping into `src/autotune/runtime/daemon_state.rs`; `runtime.rs` dropped to 1,669 lines.
- [x] Patch autotune/runtime planning: moved runtime-specific plan denial helpers, simulated dry-run record construction, candidate situation ranking, and simulated candidate selection into `src/autotune/runtime/planning.rs`; `runtime.rs` dropped to 1,543 lines.
- [x] Patch autotune/runtime worker-session: moved privileged worker startup/missing-socket helpers into `src/autotune/runtime/worker.rs` and controller session loop/finish handling into `src/autotune/runtime/session.rs`; `runtime.rs` dropped to 1,378 lines.
- [x] Patch autotune/runtime readable-orchestration cleanup: moved target-state DTOs, decision/data-quality view helpers, daemon-state snapshot methods, stream-entry construction, and decision-log append logic into runtime child modules; `runtime.rs` dropped to 997 lines and no longer needs an oversized-file allowance.
- [x] Patch daemon/policy model: moved daemon modes, action effect scopes, rollback/source descriptors, policy intents, and verdict DTOs into `src/daemon/policy/model.rs`; `policy.rs` dropped to 1,543 lines.
- [x] Patch daemon/policy rejection: moved `PolicyRejection`, its `Display`/`Error` impls, verdict mapping, and reason-code helpers into `src/daemon/policy/rejection.rs`; `policy.rs` dropped to 1,290 lines.
- [x] Patch daemon/policy context-build: moved policy context/build input DTOs into `src/daemon/policy/context.rs` and daemon-policy construction plus safety/confidence threshold helpers into `src/daemon/policy/build.rs`; `policy.rs` dropped to 1,100 lines.
- [x] Patch daemon/policy remote: moved `RemoteApplyPolicy`, remote policy construction, remote mode/target checks, and remote rule recording into `src/daemon/policy/remote.rs`; `policy.rs` dropped to 902 lines and no longer needs an oversized-file allowance.
- [x] Patch daemon/policy evaluation-facade: moved policy evaluation/check/explanation methods into `src/daemon/policy/evaluate.rs`, action-family/capability predicates into `src/daemon/policy/capability.rs`, and constructor impls into `src/daemon/policy/build.rs`; `policy.rs` is now a 59-line facade and the stale unwrap/expect allowance was removed.
- [x] Patch 1.8/architecture allowlists: moved the architecture file-size allowlist table and lookup helper into `src/architecture_tests/allowlists.rs`; `architecture_tests.rs` dropped to 2,107 lines.
- [x] Patch 1.8/architecture remaining allowlists: moved public-module, unwrap/expect, and direct-print allowlist tables plus lookup/summary helpers into `src/architecture_tests/allowlists.rs`; `architecture_tests.rs` dropped to 1,869 lines.
- [x] Patch 1.8/architecture scanners: moved Rust file walking, architecture boundary assertions, and Rust path lexer/parser helpers into `src/architecture_tests/scanners.rs`; `architecture_tests.rs` dropped to 1,310 lines.
- [x] Patch 1.8/architecture dependencies: moved the dependency matrix, matrix helper, and coverage test into `src/architecture_tests/dependencies.rs`; `architecture_tests.rs` dropped to 889 lines and no longer needs an oversized-file allowance.
- [x] Patch 1.8/architecture file-size-public-api: moved Rust file-size checks into `src/architecture_tests/file_size.rs` and public API surface checks into `src/architecture_tests/public_api.rs`; `architecture_tests.rs` dropped to 728 lines.
- [x] Patch F2: Move process-family tree helpers: created `src/focus/tree_walk.rs` and moved tree walk helper functions.
- [x] Patch F3: Move safety helpers: created `src/focus/safety.rs` and moved safety check logic and helper functions.
- [x] Patch F4: Move group-build helpers: created `src/focus/group_build.rs` and moved group construction helper functions.
- [x] Patch F5: Move scoring penalty helpers: moved scoring penalty functions from `process_scan.rs` to `score.rs`.
- [x] Patch F6: Move community-rule bridge: created `src/focus/community_rules.rs` and moved community classification helpers.
- [x] Patch F7: Shrink `focus/mod.rs` to facade: verified `focus/mod.rs` is a clean module-decl/re-export facade and all sub-modules are tidy.

## Current shape from the uploaded code

Largest files currently inspected:

```text
4030 src/focus/mod.rs
3468 src/ebpf_loader.rs
3467 src/autotune/planner.rs
2828 src/autotune/runtime.rs
2684 src/daemon/policy.rs
2623 src/report/analysis.rs
2514 src/regression_tests.rs
2501 src/session.rs
2452 src/daemon/privilege.rs
2408 src/foreground.rs
2407 src/cli/monitor.rs
2393 src/architecture_tests.rs
2379 src/process_tree.rs
2351 src/diagnosis.rs
2293 src/profiles.rs
2249 src/community_rules.rs
2146 src/test_fixture_builder.rs
2123 src/agent/tests.rs
1989 src/autotune/apply_low_risk.rs
1747 src/actions/runner.rs
```

Important observation: many target directories already exist, but several files are still transitional facades. Examples:

- `src/focus/{classify,groups,resolve,score,snapshot}.rs` already contain real code, but `focus/mod.rs` still holds large helper and test sections.
- `src/ebpf/{attach,maps,preflight,ringbuf,tracepoint_format}.rs` exist mostly as stubs.
- `src/autotune/planning/*` contains real candidate-plan modules, but `autotune/planner.rs` still owns planner evaluation and 2,500+ lines of tests.
- `src/autotune/runtime/*`, `src/daemon/policy/*`, `src/profiles/*`, `src/foreground/*`, and `src/process/*` are staged for decomposition but are still partly or mostly façade stubs.

The refactor should exploit this scaffolding instead of inventing a new layout.

## Ground rules

1. **Move first, redesign later.** The first pass should be mostly mechanical extraction. No behavior cleanup, no naming overhaul, and no algorithm changes in the same patch as a move.
2. **One owner per patch.** A patch should touch one domain and preferably one source file plus one new child module.
3. **Keep old import paths stable.** Parent files should become façades with `pub(crate) use ...` re-exports until callers migrate naturally.
4. **Lower the line-count allowlist after every successful extraction.** Do not leave allowlist ceilings at old values.
5. **Prefer `pub(crate)` or narrower.** Do not make items public just to make a move compile.
6. **Move tests before production logic.** This gives a large line-count win with low behavior risk and makes later production diffs easier to review.
7. **Do not mix test extraction with production extraction.** A test-only patch should only move tests and test helpers.
8. **Let compiler errors guide imports.** Avoid broad `use super::*` in production modules once the module compiles; narrow imports before merging.
9. **Temporary façade allowances must be removed.** `#![allow(unused_imports)]` and `#![allow(dead_code)]` are acceptable during one-step moves only if a follow-up removes or justifies them.
10. **Every new child module needs an ownership comment.** A short module-level comment should state what it owns and what it must not do.

## Standard micro-patch template

Use this exact rhythm for every extraction:

1. Create the target module file.
2. Add `mod target_module;` or `#[path = "..."] mod target_module;` in the parent.
3. Copy the target item(s) into the new file unchanged.
4. Add the smallest imports needed to compile.
5. Re-export from the parent if existing callers use the parent path.
6. Delete the original copied item(s) from the parent.
7. Run `cargo fmt --all`.
8. Run the narrowest target test first.
9. Run `cargo build --all-targets`.
10. Run `cargo clippy --all-targets -- -D warnings`.
11. Run `cargo test --all-targets`.
12. Lower `OVERSIZED_RUST_FILE_ALLOWLIST` for the file just reduced.
13. Commit with a message like `refactor(focus): move foreground focus tests`.

## Phase 1: test-only extraction pass

### Patch 1.6: Split `src/daemon/policy.rs` tests

Current embedded test section:

```text
lines 1653-2684, about 1032 lines
```

Steps:

1. Create `src/daemon/policy_tests/mod.rs` or `src/daemon/policy/tests.rs`.
2. Split by policy dimension:
   - `mode.rs`
   - `remote.rs`
   - `safety.rs`
   - `capabilities.rs`
   - `explain.rs`
3. Lower `daemon/policy.rs` allowlist.

### Patch 1.7: Split remaining embedded tests

Do the same for:

- `src/profiles.rs`, about 997 test lines.
- `src/diagnosis.rs`, about 767 test lines.
- `src/autotune/runtime.rs`, about 843 test lines.
- `src/ebpf_loader.rs`, about 1186 test lines spread across several test modules.
- `src/cli/monitor.rs`, about 1152 test lines spread across four test modules.
- `src/process_tree.rs`, about 409 test lines.
- `src/community_rules.rs`, about 885 test lines.
- `src/session.rs`, about 172 inline test lines plus external `tree_tick_tests`.

Each patch should target one file only.

### Patch 1.8: Split top-level test files

These files are test-only but still too broad:

- `src/regression_tests.rs`
- `src/architecture_tests.rs`
- `src/test_fixture_builder.rs`
- `src/agent/tests.rs`

Do not rewrite test logic. Make them dispatch modules:

```rust
mod support;
mod event_lifecycle;
mod recording_artifacts;
mod daemon_acceptance;
```

For `architecture_tests.rs`, split into:

- `architecture_tests/allowlists.rs`
- `architecture_tests/file_size.rs`
- `architecture_tests/public_api.rs`
- `architecture_tests/dependencies.rs`
- `architecture_tests/scanners.rs`
- `architecture_tests/direct_prints.rs`
- `architecture_tests/unwrap_expect.rs`

After this patch, `architecture_tests.rs` should be a thin test module instead of holding all scanner implementation and tables.

## Phase 2: turn existing façade stubs into real modules

This phase should avoid new architecture. The repository already declares many child modules. Fill those first.

## `src/focus/mod.rs` decomposition

Target final shape:

```text
src/focus/mod.rs              <= 250 lines, facade and module docs only
src/focus/classify.rs         existing classification model and helpers
src/focus/groups.rs           group data model and group construction
src/focus/group_build.rs      tree-to-group construction helpers
src/focus/score.rs            pure scoring primitives
src/focus/penalties.rs        game/browser/compile/idle/desktop penalties
src/focus/safety.rs           protected/system/RT safety checks
src/focus/tree_walk.rs        ancestor/descendant/process-family helpers
src/focus/foreground.rs       foreground fallback and safe foreground target logic
src/focus/community_rules.rs  community-rule integration bridge
src/focus/resolve.rs          FocusResolver and final decision selection
src/focus/snapshot.rs         FocusSnapshot, FocusProcess, FocusCache
src/focus/test_support.rs     shared test builders
src/focus/tests/*             tests
```

### Patch F1: Move foreground fallback helpers

Move from `focus/mod.rs`:

- `process_name_looks_like_xwayland`
- `is_foreground_fallback_group`
- `add_foreground_fallback_group_if_needed`
- foreground-source-mode related helpers if still in `mod.rs`

into `src/focus/foreground.rs`.

Keep exports stable:

```rust
pub(crate) use foreground::{
    add_foreground_fallback_group_if_needed,
    apply_foreground_source_mode_to_snapshot,
    foreground_process_is_safe_auto_target,
};
```

### Patch F2: Move process-family tree helpers

Create `src/focus/tree_walk.rs` and move:

- `descendants_of_process`
- `same_process_family`
- `is_process_ancestor`
- `descendants_of_pid`
- `has_ancestor_in_set`
- `process_appears_tied_to_root`
- `same_non_empty_cgroup`

These are pure helpers and should not depend on scoring policy.

### Patch F3: Move safety helpers

Replace `src/focus/safety.rs` façade with real logic. Move:

- scheduler constants used only by focus safety
- `is_critical_realtime_process`
- `is_unknown_foreground_like`
- `is_too_broad_system_service_group`
- `is_system_service_root`
- `process_name_looks_like_systemd`
- `safety_warning_reason`

Keep `SafetyWarning` exported from the model owner, but put safety predicates here.

### Patch F4: Move group-building helpers

Create `src/focus/group_build.rs` and move:

- `build_tree_groups_for_kind`
- `root_pids_from_members`
- group primary/root selection helpers
- `display_name_for_group`

Do not move scoring penalties in this patch.

### Patch F5: Move scoring penalty helpers

Create `src/focus/penalties.rs` or move into `score.rs` if it stays below 1,000 lines. Move:

- `low_to_moderate_activity_bonus`
- `game_group_penalty`
- `browser_group_penalty`
- `compile_group_penalty`
- `idle_group_penalty`
- `desktop_group_penalty`
- `compare_process_preference`

If `score.rs` would exceed 1,000 lines, use `penalties.rs` and re-export selected helpers through `score.rs` only when needed.

### Patch F6: Move community-rule bridge

Replace the tiny `focus/community_rules.rs` stub with the real test/prod classification bridge:

- `try_community_rules_classification`
- `system_class_for_community_task_class`
- any focus-specific community classification adapters

### Patch F7: Shrink `focus/mod.rs` to facade

After F1-F6, `focus/mod.rs` should contain:

- module docs
- `mod` declarations
- focused re-exports
- constants only if truly cross-module

Lower the allowlist to the new count or remove it if under 1,000 lines.

## `src/ebpf_loader.rs` decomposition

Target final shape:

```text
src/ebpf_loader.rs              <= 200 lines, compatibility facade
src/ebpf/model.rs               LoadedEbpf, DropCountersSnapshot, BlockIoCorrelationBasis
src/ebpf/load.rs                load_and_attach orchestration
src/ebpf/attach.rs              attach_tracepoint and per-provider attach helpers
src/ebpf/maps.rs                map sizing and map budget calculations
src/ebpf/memlock.rs             memlock limit read/raise/report logic
src/ebpf/memory.rs              available-memory and page-size helpers
src/ebpf/preflight.rs           tracepoint preflight report assembly
src/ebpf/tracepoint_format.rs   tracepoint format parser and field validation
src/ebpf/tracepoints/block_io.rs
src/ebpf/tracepoints/kms.rs
src/ebpf/tracepoints/drm_fence.rs
src/ebpf/object.rs              ebpf_object_bytes
src/ebpf/tests/*                tests
```

### Patch E1: Move eBPF tests

Move `map_sizing_tests`, `block_io_tracepoint_validation_tests`, `sched_wakeup_new_coverage_tests`, and generic loader tests into `src/ebpf/tests/*`. Keep the production file unchanged.

### Patch E2: Move model types

Create `src/ebpf/model.rs` and move:

- `LoadedEbpf`
- `BlockIoCorrelationBasis`
- `DropCountersSnapshot`
- `EbpfMapSizingReport`
- `TracepointAvailability`
- `TracepointPreflightReport` if not yet moved to preflight

Re-export from `ebpf_loader.rs` for compatibility.

### Patch E3: Move map sizing

Use the existing `src/ebpf/maps.rs` as the real owner for:

- map sizing constants
- `wakeup_data_entries_for_config`
- `map_sizing_for_config_after_memlock`
- `map_sizing_for_config_from_memory`
- `ebpf_map_sizing_report`
- `dynamic_map_sizing`
- power-of-two and alignment helpers

### Patch E4: Move memlock and memory helpers

Create:

- `src/ebpf/memlock.rs`
- `src/ebpf/memory.rs`

Move:

- `locked_memory_limit_bytes`
- `memlock_limit_bytes_from_rlim`
- `read_memlock_rlimit`
- `raise_memlock_limit`
- `log_memlock_policy_report`
- `available_memory_bytes`
- `parse_mem_available_bytes`
- `system_page_size`
- `format_optional_bytes`

### Patch E5: Move tracepoint format parsing

Use `src/ebpf/tracepoint_format.rs` as the real owner for:

- `TracepointField`
- `TracepointFormat`
- `parse_tracepoint_format_at`
- `parse_tracepoint_format`
- `parse_tracepoint_field_line`
- `parse_tracepoint_field_name`
- field offset/size validation helpers

### Patch E6: Move tracepoint provider validation

Create a tracepoint subdirectory:

```text
src/ebpf/tracepoints/mod.rs
src/ebpf/tracepoints/block_io.rs
src/ebpf/tracepoints/kms.rs
src/ebpf/tracepoints/drm_fence.rs
```

Move the block I/O, KMS, and DRM fence offset structs and provider-specific validation helpers into those files.

### Patch E7: Move attach logic

Use `src/ebpf/attach.rs` for:

- `attach_tracepoint`
- `attach_kms_tracepoints`
- `attach_optional_kms_tracepoint`
- `attach_drm_fence_tracepoints`
- `attach_optional_drm_fence_tracepoint`
- perf event attach helpers

Keep `load_and_attach` in `ebpf/load.rs` so attach remains a helper layer, not the orchestration owner.

### Patch E8: Move object loading

Use `src/ebpf/object.rs` for `ebpf_object_bytes` and any future object-path selection.

### Patch E9: Reduce `ebpf_loader.rs` to compatibility facade

Keep old imports stable:

```rust
pub use crate::ebpf::model::{LoadedEbpf, DropCountersSnapshot, BlockIoCorrelationBasis};
pub use crate::ebpf::load::load_and_attach;
pub use crate::ebpf::preflight::tracepoint_preflight;
pub use crate::ebpf::maps::ebpf_map_sizing_report;
```

Then lower or remove the allowlist.

## `src/autotune/planner.rs` decomposition

Target final shape:

```text
src/autotune/planner.rs             <= 200 lines, compatibility facade
src/autotune/planning/model.rs      PlanResult, summaries, CandidateEvaluation
src/autotune/planning/denial.rs     CandidateDenyReason and denial grouping
src/autotune/planning/evaluate.rs   static proposal evaluation
src/autotune/planning/policy.rs     daemon policy mapping and intent helpers
src/autotune/planning/ranking.rs    sort/rank/no-action selection
src/autotune/planning/planner.rs    CandidatePlanner orchestration
src/autotune/planning/tests/*       planner tests
```

### Patch P1: Move model and summary types

Move from `planner.rs`:

- `CandidateEvaluation`
- `CandidateEvaluationDraft`
- `PlanResult`
- `PlannerSummary`
- `PlannerSelectedSummary`
- `PlannerEvaluationSummary`
- `PlannerDenySummary`
- `PlannerNoActionSummary`
- summary `impl`s

into `planning/model.rs` or `planning/summary.rs`.

### Patch P2: Move denial reason enum

Move `CandidateDenyReason` and `impl CandidateDenyReason` into `planning/denial.rs`. Also move:

- `grouped_denials`
- `names_for_reason`
- `names_for_any_reason`
- `normalize_evaluation_denials`

### Patch P3: Move ranking helpers

Use `planning/ranking.rs` for:

- `sort_candidate_evaluations`
- `rank_with_workload_memory`
- `no_action_reason_for_evaluations`

### Patch P4: Move daemon-policy mapping

Create `planning/policy.rs` and move:

- `policy_intent_for_mode`
- `policy_family_enabled`
- `policy_family_denied`
- `policy_family_matches`
- `mode_requires_autonomous_workload_family`
- `policy_context_for_input`
- `deny_reason_from_policy`

### Patch P5: Move evaluation logic

Create `planning/evaluate.rs` and move:

- `evaluate_proposals_with_runner`
- `evaluate_proposal_static`
- `dry_run_candidate_if_still_eligible`

### Patch P6: Move `CandidatePlanner`

Create `planning/planner.rs` and move `CandidatePlanner` and its `impl`.

Then make `autotune/planner.rs` a compatibility facade re-exporting the stable planner API. Lower the allowlist aggressively.

## `src/autotune/runtime.rs` decomposition

Target final shape:

```text
src/autotune/runtime.rs                  <= 300-400 lines, facade and main runtime type
src/autotune/runtime/config.rs           AutotuneRuntimeConfig and validation
src/autotune/runtime/stream.rs           AutotuneDecisionStreamEntry and output boundary
src/autotune/runtime/target_state.rs     RuntimeTargetState and target hashing
src/autotune/runtime/history.rs          RuntimeHistoryContext and lifecycle event helpers
src/autotune/runtime/daemon_state.rs     daemon phase/profile mapping
src/autotune/runtime/session.rs          run_autotune_controller_session and exit type
src/autotune/runtime/worker.rs           privileged worker spawn/warnings
src/autotune/runtime/planning.rs         candidate selection/simulated dry-run helpers
src/autotune/runtime/decision_view.rs    decision_label/reason/action_kind helpers
src/autotune/runtime/tests/*             tests
```

### Patch R1: Move tests

Move the `mod tests` section into `src/autotune/runtime/tests/*` before production extraction.

### Patch R2: Move runtime config

Move:

- `AutotuneRuntimeConfig`
- `resolve_workload_policy_config`
- `daemon_config_for_runtime_mode`
- `validate_runtime_config`
- `AutotuneRuntimeConfig::observe/suggest/apply_low_risk/from_daemon_config/from_daemon_parts/for_mode`

into `runtime/config.rs`.

### Patch R3: Move decision stream output

Move:

- `AutotuneDecisionStreamEntry`
- JSON stream entry construction
- the remaining direct `println!` stream boundary

into `runtime/stream.rs`.

This should also remove the direct-print allowlist from `architecture_tests.rs`, because the output boundary becomes explicit and can be allowlisted as CLI/rendering-like infrastructure if needed.

### Patch R4: Move history context and lifecycle history helpers

Use existing `runtime/history.rs` for:

- `RuntimeHistoryContext`
- `From<LiveExperimentHistoryContext>` impl
- `LifecycleHistoryEventInput`
- history conversion helpers if they only support runtime history writing

### Patch R5: Move daemon-state mapping

Create `runtime/daemon_state.rs` and move:

- `daemon_phase_from_controller_phase`
- `daemon_profile_workload_identity_hash`
- `daemon_profile_action_kind_and_safety_class`
- `daemon_profile_confidence_milli`
- `history_phase`
- `history_mode`
- `history_situation`

### Patch R6: Move candidate selection helpers

Create `runtime/planning.rs` and move:

- `select_best_candidate_for_situation`
- `candidate_situation_rank`
- `simulated_dry_run_records`
- top-denied-reason helpers if they are runtime-specific

### Patch R7: Move worker/session helpers

Create:

- `runtime/worker.rs`
- `runtime/session.rs`

Move:

- `maybe_spawn_privileged_worker`
- `warn_if_unmanaged_privileged_worker_missing`
- `finish_autotune_controller_session`
- `AutotuneControllerExit`

### Patch R8: Keep `AutotuneRuntime` orchestration readable

After helpers move, keep `AutotuneRuntime` methods in either:

- `runtime.rs`, if the file is under 1,000 lines, or
- `runtime/controller.rs`, if the parent should be a pure facade.

Do not split one logical state-machine transition across multiple modules unless the helper boundaries are clear.

## `src/daemon/policy.rs` decomposition

Target final shape:

```text
src/daemon/policy.rs             <= 200 lines, facade
src/daemon/policy/model.rs       modes, descriptors, intents, verdicts
src/daemon/policy/rejection.rs   PolicyRejection and reason helpers
src/daemon/policy/context.rs     DaemonPolicyContext and build input
src/daemon/policy/build.rs       build_daemon_policy and config conversion
src/daemon/policy/evaluate.rs    DaemonPolicy::check_action / explain_action implementation
src/daemon/policy/remote.rs      RemoteApplyPolicy and remote context rules
src/daemon/policy/capability.rs  action family and unavailable capability helpers
src/daemon/policy/tests/*        tests
```

### Patch D1: Move tests

Move the embedded policy tests first.

### Patch D2: Move basic model types

Replace `daemon/policy/model.rs` façade with real definitions for:

- `DaemonMode`
- `ActionEffectScope`
- `RollbackRequirement`
- `ActionSource`
- `ActionDescriptor`
- `PolicyIntent`
- `DaemonPolicyVerdict`

### Patch D3: Move rejection model

Create `daemon/policy/rejection.rs` and move:

- `PolicyRejection`
- `Display` impl
- `Error` impl
- reason-code helpers

### Patch D4: Move context/build config

Use `context.rs` for context structs and `build.rs` for policy construction:

- `DaemonPolicyContext`
- `RemotePolicyContext`
- `DaemonPolicyBuildInput`
- `build_daemon_policy`
- `max_safety_class_for_mode`
- `allowed_effect_scopes_for_mode`
- config-derived thresholds

### Patch D5: Move remote rules

Create `remote.rs` and move:

- `RemoteApplyPolicy`
- `remote_apply_policy_for_config`
- `remote_mode_supported_by_context`
- `remote_target_count_for_config`
- remote target count rule recording

### Patch D6: Move evaluation implementation

Use `evaluate.rs` for:

- `DaemonPolicy::check_action`
- `DaemonPolicy::explain_action`
- policy rule recording functions
- `validate_policy_descriptor_shape`

Keep `DaemonPolicy` itself in `model.rs` or `policy.rs` depending on what produces the cleanest imports.

## `src/report/analysis.rs` decomposition

Target final shape:

```text
src/report/analysis.rs            <= 250 lines, build_report_analysis orchestration
src/report/analysis/timing.rs     KMS, DRM fence, Wayland presentation summaries
src/report/analysis/foreground.rs foreground/focus summaries and annotation
src/report/analysis/tasks.rs      top task row selection and formatting
src/report/analysis/density.rs    spike density and percentile helpers
src/report/analysis/correlation.rs text report correlation sections
src/report/analysis/runtime.rs    runtime slice summaries
src/report/analysis/quality.rs    data quality summary
src/report/analysis/pressure.rs   pressure timeline, notes, windows
src/report/analysis/clusters.rs   spike points, cluster creation, wake graph
src/report/analysis/diagnosis.rs  bridge to diagnosis engine
src/report/analysis/format.rs     shared formatting helpers
```

### Patch A1: Move display timing summaries

Move:

- `build_kms_timing_summary`
- `build_scanout_window_estimate`
- `build_drm_fence_timing_summary`
- `top_drm_fence_waits`
- DRM/KMS proximity helpers
- `build_wayland_presentation_summary`
- Wayland proximity helpers
- optional percentile helpers if used only here

into `report/analysis/timing.rs`.

### Patch A2: Move foreground/focus summaries

Move:

- `foreground_report_summary`
- `focus_report_summary`
- `annotate_clusters_with_foreground`
- `foreground_for_cluster`
- `foreground_for_elapsed_ms`

into `report/analysis/foreground.rs`.

### Patch A3: Move task row and formatting helpers

Move:

- `top_task_rows_by_max_latency`
- `top_task_rows_by_p99_latency`
- `filtered_latency_tasks`
- `format_task_cpu_perf`
- `format_process_pid`
- `format_elapsed`
- `format_option`

into `report/analysis/tasks.rs` or `format.rs`.

### Patch A4: Move pressure timeline

Move:

- `build_pressure_timeline`
- `pressure_window_near_spike`
- `push_pressure_peak_window`
- `PressureNoteInput`
- `build_pressure_notes`
- `push_pressure_note_if_above`
- `pressure_kind_label`
- pressure formatting helpers

into `report/analysis/pressure.rs`.

### Patch A5: Move clustering and wake graph

Move:

- `spike_cluster_analysis`
- `spike_clusters_from_points`
- `flatten_spike_events`
- `flatten_top_spikes`
- `spike_point_from_task`
- `cluster_from_points`
- `build_wake_graph`
- cluster elapsed/label helpers

into `report/analysis/clusters.rs`.

### Patch A6: Move diagnosis bridge

Move:

- `perform_diagnosis`
- `explain_diagnosis`
- any report-specific diagnosis conversion helpers

into `report/analysis/diagnosis.rs`.

### Patch A7: Leave orchestration in `analysis.rs`

`build_report_analysis_from_input` should read like a pipeline:

1. load artifacts
2. compute frame spikes
3. cluster spikes
4. compute correlation windows
5. load correlations
6. diagnose clusters
7. annotate foreground
8. build summaries
9. return model

Everything else belongs in child modules.

## `src/session.rs` decomposition

Target final shape:

```text
src/session.rs                    <= 250-400 lines, facade and high-level docs
src/session/monitor_session.rs    MonitorSession struct and main run loop
src/session/runtime.rs            runtime construction and shutdown boundaries
src/session/recording.rs          RecordingRuntime
src/session/exporter.rs           ExporterRuntime
src/session/alerts.rs             AlertRuntime
src/session/sampler.rs            SamplerRuntime
src/session/hwmon.rs              HwmonRuntime
src/session/ui.rs                 UiRuntimeStage and TUI snapshots
src/session/targeting.rs          target plan and target tick logic
src/session/probes.rs             probe plan and drain logic
src/session/ticks/*               tick context/event helpers
src/session/display_timing.rs     KMS/DRM/Wayland event conversion helpers
src/session/tests/*               tests
```

### Patch S1: Move tests

Move `foreground_session_tests`, `tree_tick_tests`, and generic tests into `src/session/tests/*`.

### Patch S2: Move target plan helpers

Move:

- `SessionTargetPlan`
- `needs_tree_tick_from_parts`
- target tick context/event types

into `session/targeting.rs` or `session/ticks/target.rs`.

### Patch S3: Move foreground/focus helpers

Move:

- `foreground_capture_enabled`
- `foreground_resolver_from_config`
- `foreground_identity_changed`
- `FocusTickContext`
- `ForegroundTickContext`

into `session/ticks/foreground.rs` and `session/ticks/focus.rs`.

### Patch S4: Move runtime stage structs

Replace current tiny façade files with real owners:

- `RecordingRuntime` -> `session/recording.rs`
- `ExporterRuntime` -> `session/exporter.rs`
- `AlertRuntime` -> `session/alerts.rs`
- `SamplerRuntime` -> `session/sampler.rs`
- `UiRuntimeStage` -> `session/ui.rs`
- `HwmonRuntime` -> `session/hwmon.rs`

### Patch S5: Move display timing conversion helpers

Create `session/display_timing.rs` and move:

- `elapsed_ms_from_event_timestamp`
- `kms_flip_event_kind_name`
- `kms_flip_provider_name`
- `kms_flip_flag_names`
- `drm_fence_event_kind_name`
- `drm_fence_provider_name`
- `drm_gpu_role_name`

### Patch S6: Move `MonitorSession`

Move `MonitorSession` and its impl into `session/monitor_session.rs`. Make fields `pub(super)` only if child tick modules require them.

`session.rs` then becomes a facade plus `run_monitor` re-export.

## `src/daemon/privilege.rs` decomposition

Target final shape:

```text
src/daemon/privilege.rs              <= 250 lines, facade
src/daemon/privilege/model.rs        roles, transports, operations, request/response DTOs
src/daemon/privilege/allowlist.rs    PrivilegeCommandAllowlist
src/daemon/privilege/audit.rs        audit event construction/sink
src/daemon/privilege/in_process.rs   InProcessPrivilegedActionService
src/daemon/privilege/socket.rs       UnixSocketPrivilegedActionService client
src/daemon/privilege/worker.rs       worker handle, spawn, accept loop
src/daemon/privilege/protocol.rs     request/response serialization over socket
src/daemon/privilege/revalidate.rs   live target identity revalidation
src/daemon/privilege/tests/*         tests
```

### Patch V1: Move tests

Extract `mod tests` first.

### Patch V2: Move model/protocol types

Move:

- `PrivilegeProcessRole`
- `PrivilegeTransport`
- `PrivilegedOperation`
- `PrivilegeCommandRequest`
- `PrivilegeDecision`
- `CandidatePlanRequest`
- `CandidateApplyRequest`
- `RollbackRequest`
- `ApplyResult`
- `RollbackResult`
- `PrivilegedWorkerCandidatePlan`
- `PrivilegedWorkerRequest`
- `PrivilegedWorkerResponse`

### Patch V3: Move allowlist and audit

Move:

- `PrivilegeCommandAllowlist`
- `privileged_operation_audit_event`
- `PrivilegeAuditSink`
- boundary audit input structs and audit writers

### Patch V4: Move services

Move:

- `InProcessPrivilegedActionService` -> `in_process.rs`
- `UnixSocketPrivilegedActionService` -> `socket.rs`
- `PrivilegedWorkerHandle`, spawn/wait/path helpers -> `worker.rs`

### Patch V5: Move worker execution and revalidation

Move:

- `run_privileged_worker`
- `handle_privileged_worker_connection`
- `read_privileged_worker_request`
- `execute_privileged_worker_request`
- `TargetRevalidationError`
- `revalidate_candidate_targets`
- task identity readers

Keep all mutation through the same service trait.

## `src/foreground.rs` decomposition

Target final shape:

```text
src/foreground.rs                   <= 150 lines, facade
src/foreground/model.rs             source/status/snapshot/event models
src/foreground/provider.rs          provider trait and unsupported provider
src/foreground/providers/auto.rs    provider auto-selection
src/foreground/providers/hyprland.rs
src/foreground/providers/sway.rs
src/foreground/providers/x11.rs
src/foreground/parse/sway.rs
src/foreground/parse/x11.rs
src/foreground/resolver.rs
src/foreground/command.rs
src/foreground/tests/*
```

### Patch G1: Move tests

Extract tests first.

### Patch G2: Move models

Replace `foreground/model.rs` façade with:

- `ForegroundSource`
- `ForegroundProviderStatus`
- `ForegroundWindowSnapshot`
- `ForegroundAvailableInput`
- `ForegroundEvent`
- `ForegroundEventInput`
- redaction helper if model-only

### Patch G3: Move provider trait and unsupported provider

Move `ForegroundProvider` and `UnsupportedForegroundProvider` into `provider.rs`.

### Patch G4: Move auto-selection

Move:

- `auto_foreground_provider`
- `auto_foreground_resolver`
- `is_generic_wayland_without_supported_foreground_api`
- `current_desktop_looks_like_gnome_or_kde`

into `providers/auto.rs`.

### Patch G5: Move provider implementations

Move:

- Hyprland structs/functions into `providers/hyprland.rs`
- Sway structs/functions into `providers/sway.rs`
- X11 provider into `providers/x11.rs`

### Patch G6: Move parser helpers

Move:

- `SwayNode`, `SwayWindowProperties`, `focused_sway_snapshot_from_tree`, `find_focused_sway_node`, `sway_confidence` into `parse/sway.rs` or keep provider-local if not reused.
- `X11WindowProperties`, `parse_x11_*`, `x11_confidence` into `parse/x11.rs`.

### Patch G7: Move resolver

Move `ForegroundResolver`, `is_good_foreground_snapshot`, and `reduce_stale_confidence` into `resolver.rs`.

## `src/cli/monitor.rs` decomposition

Target final shape:

```text
src/cli/monitor.rs                  <= 250 lines, facade/re-export
src/cli/monitor/args.rs             MonitorArgs and Clap attributes
src/cli/monitor/presence.rs         MonitorArgPresence
src/cli/monitor/defaults.rs         Default impls
src/cli/monitor/merge.rs            merge_bool and config merge helpers
src/cli/monitor/validate.rs         validation functions
src/cli/monitor/foreground.rs       foreground-related normalization/validation
src/cli/monitor/tests/*             tests
```

Steps:

1. Move tests first.
2. Move `MonitorArgPresence` to `presence.rs`.
3. Move `RecordingMode`, `FocusSource`, and `ForegroundSource` CLI parsing impls into focused files if these are CLI-only impls.
4. Move `Default for MonitorArgs` into `defaults.rs`.
5. Move merge helpers into `merge.rs`.
6. Move foreground normalization/validation into `foreground.rs`.
7. Keep Clap struct definitions in `args.rs`; this makes the CLI surface easy to scan.

## `src/process_tree.rs` decomposition

Target final shape:

```text
src/process_tree.rs             <= 200 lines, compatibility facade
src/process/model.rs            ScanBudget, TaskClass, TaskInfo, TargetSnapshot
src/process/cache.rs            ProcessCache and CachedProcInfo
src/process/procfs.rs           /proc readers and stat parsing
src/process/classify.rs         classify_task and classification predicates
src/process/snapshot.rs         target_snapshot, diff_tasks_ref, expand_tasks_at
src/process/tree.rs             descendants, render_tree, collect_cgroup_pids
src/process/community.rs        community-rule classification adapter
src/process/tests/*             tests
```

Steps:

1. Move tests first.
2. Move model types into `process/model.rs`.
3. Move `ProcessCache` into `process/cache.rs`.
4. Move `/proc` readers into `process/procfs.rs`.
5. Move `classify_task` and all `is_*_comm` helpers into `process/classify.rs`.
6. Move target snapshot/diff/expand into `process/snapshot.rs`.
7. Move render/tree traversal/cgroup helpers into `process/tree.rs`.
8. Keep `process_tree.rs` as a compatibility facade until all callers use `crate::process::*` paths.

## `src/diagnosis.rs` decomposition

Target final shape:

```text
src/diagnosis.rs                 <= 250 lines, facade
src/diagnosis/config.rs          DiagnosisConfig and threshold docs
src/diagnosis/model.rs           causes, confidence, evidence, diagnosis models
src/diagnosis/anchor.rs          select_anchor and anchor helpers
src/diagnosis/candidates.rs      candidate scoring/sorting/rejection
src/diagnosis/scheduler.rs       scheduler-delay evidence
src/diagnosis/context.rs         missing evidence/context summary
src/diagnosis/runtime_slice.rs   runtime-slice evidence
src/diagnosis/cpu.rs             CPU frequency/perf/migration evidence
src/diagnosis/memory.rs          page fault evidence
src/diagnosis/tests/*            tests
```

Steps:

1. Move tests first.
2. Move config and model types.
3. Move anchor selection.
4. Move generic candidate helpers.
5. Move evidence builders by evidence source.
6. Leave `diagnose_cluster_with_config` as the orchestration function until everything else is extracted.

## `src/profiles.rs` decomposition

Target final shape:

```text
src/profiles.rs             <= 150 lines, facade
src/profiles/model.rs       Profile, ProfileRule, warnings, apply result models
src/profiles/cache.rs       ProfileApplyCache
src/profiles/matcher.rs     matching rules and matched counts
src/profiles/plan.rs        ProfileApplyPlan and planned_profile_apply
src/profiles/apply.rs       apply/verify/preflight action execution
src/profiles/parse.rs       TOML parsing
src/profiles/validate.rs    profile validation
src/profiles/render.rs      render_profiles_toml and templates
src/profiles/warnings.rs    offline CPU and overlap warnings
src/profiles/tests/*        tests
```

Steps:

1. Move tests first.
2. Move structs to `model.rs`.
3. Move cache types to `cache.rs`.
4. Move matching functions to `matcher.rs`.
5. Move plan construction to `plan.rs`.
6. Move action execution/preflight/verify to `apply.rs`.
7. Move TOML parsing to `parse.rs`.
8. Move validation to `validate.rs`.
9. Move rendering to `render.rs`.
10. Move warnings to `warnings.rs`.
11. Make `profiles.rs` a facade.

## `src/community_rules.rs` decomposition

Target final shape:

```text
src/community_rules.rs              <= 150 lines, facade
src/community_rules/model.rs        file/source/rule/config/status models
src/community_rules/load.rs         load status and load db functions
src/community_rules/loader.rs       existing loader implementation
src/community_rules/commands.rs     rules_command dispatch and subcommands
src/community_rules/render.rs       render check/import/status/list output
src/community_rules/db.rs           CommunityRulesDb methods
src/community_rules/classify.rs     classify_process_identity and context signals
src/community_rules/normalize.rs    normalize_process_name and candidates
src/community_rules/paths.rs        default paths and imported file discovery
src/community_rules/importer.rs     existing import implementation
src/community_rules/tests/*         tests
```

Steps:

1. Move tests first.
2. Move model types to `model.rs`.
3. Move load functions to `load.rs`.
4. Move command dispatch and filesystem mutation commands to `commands.rs`.
5. Move string renderers to `render.rs`.
6. Move `CommunityRulesDb` and DB lookup to `db.rs`.
7. Move classification helpers to `classify.rs`.
8. Move normalization helpers to `normalize.rs`.
9. Move path helpers to `paths.rs`.
10. Leave `community_rules.rs` as a facade re-exporting intentional crate-private API.

## `src/test_fixture_builder.rs` decomposition

Target final shape:

```text
src/test_fixture_builder.rs          <= 150 lines, dispatcher/facade
src/test_fixture_builder/model.rs    metadata/expected/privacy structs
src/test_fixture_builder/public.rs   public example fixtures
src/test_fixture_builder/real.rs     real captured fixture definitions
src/test_fixture_builder/synthetic.rs synthetic/small fixtures
src/test_fixture_builder/write.rs    write_fixture and serialization helpers
src/test_fixture_builder/session.rs  base_session and dummy config/time helpers
src/test_fixture_builder/events.rs   spike/interval/task helper builders
```

Steps:

1. Move model structs.
2. Move fixture families one at a time.
3. Move writer helpers.
4. Move event/session builders.
5. Keep top-level builder file as a command dispatcher.

## `src/agent/tests.rs` decomposition

Target final shape:

```text
src/agent/tests.rs               <= 100 lines, module list and shared imports
src/agent/tests/support.rs       shared request/state builders
src/agent/tests/status.rs
src/agent/tests/auth.rs
src/agent/tests/recording.rs
src/agent/tests/autotune.rs
src/agent/tests/daemon.rs
src/agent/tests/policy.rs
src/agent/tests/artifacts.rs
```

Steps:

1. Move common helpers like `minimal_remote_request`, `test_agent_state`, and `test_capabilities` into `support.rs`.
2. Move one route group per patch.
3. Keep each test module below 500 lines.
4. Remove the `agent/tests.rs` allowlist or lower it below 200 lines.

## `src/autotune/apply_low_risk.rs` decomposition

Target final shape:

```text
src/autotune/apply_low_risk.rs                <= 150-250 lines, facade
src/autotune/apply_low_risk/model.rs          ApplyLowRiskPlan/Outcome
src/autotune/apply_low_risk/executor.rs       LowRiskActionExecutor and CPU affinity executor
src/autotune/apply_low_risk/audit.rs          audited apply and history append
src/autotune/apply_low_risk/target.rs         target resolution and live-tree validation
src/autotune/apply_low_risk/experiment.rs     experiment selection/comparison/readiness
src/autotune/apply_low_risk/command.rs        apply_low_risk_command
src/autotune/apply_low_risk/tests/*           tests
```

Steps:

1. Move tests first.
2. Move model structs.
3. Move executor trait and concrete executor.
4. Move rollback guards and audit helpers.
5. Move target resolution.
6. Move experiment selection/comparison readiness logic.
7. Move command entry point.
8. Keep old `autotune::apply_low_risk::*` import paths stable via re-export.

## `src/actions/runner.rs` decomposition

Target final shape:

```text
src/actions/runner.rs              <= 150-250 lines, facade
src/actions/runner/model.rs        AuditedActionResult and ActionRunPolicy
src/actions/runner/audit.rs        audit event append/update helpers
src/actions/runner/policy.rs       policy check helpers
src/actions/runner/rollback.rs     timeout and hook-failure rollback helpers
src/actions/runner/execute.rs      run_audited_action implementation
src/actions/runner/tests/*         tests
```

Steps:

1. Move tests first.
2. Move model types.
3. Move audit helpers.
4. Move policy checking.
5. Move rollback helpers.
6. Move execution functions.
7. Keep `actions::runner::run_audited_action` stable through re-export.


PHASE 2:


# Plan 1.1 — Make multi-target `apply()` failures rollback-safe

Affected files:

* `stutter/src/actions/runner.rs`
* `stutter/src/actions/nice.rs`
* `stutter/src/actions/ioprio.rs`
* `stutter/src/actions/uclamp.rs`
* `stutter/src/actions/token.rs`
* action tests

Current problem: `NiceAction`, `IoPrioAction`, and `UclampAction` mutate each task inside a loop. If target 1 succeeds and target 2 fails, `apply()` returns `Err`, and the runner never receives a rollback token.

Implementation steps:

1. Introduce a new apply error shape that can carry a partial rollback token, for example:

   ```rust
   pub struct PartialApplyError {
       pub source: anyhow::Error,
       pub rollback: Option<RollbackToken>,
   }
   ```

2. Change the action trait result from plain `anyhow::Result<RollbackToken>` to an action-specific result that can represent partial rollback metadata.

3. In `ActionRunner`, handle partial failure like this:

   * if `apply()` succeeds, continue as today;
   * if `apply()` fails with a partial token, immediately rollback the partial token;
   * report both the original apply error and rollback result;
   * persist/audit that a partial mutation happened.

4. Update `NiceAction::apply()`:

   * collect all snapshots first;
   * build restore records before mutation;
   * apply mutations one by one;
   * track which records have actually been applied;
   * on failure, return the partial rollback token for the applied subset.

5. Repeat the same pattern for `IoPrioAction::apply()` and `UclampAction::apply()`.

6. Add tests:

   * first target succeeds, second target fails;
   * runner receives partial rollback;
   * first target is restored;
   * final action outcome is failed, not successful;
   * audit includes “partial rollback attempted”.

Done when: no successful mutation can escape without either a rollback token reaching the runner or the action internally restoring the already-applied subset.

---

# Plan 1.2 — Add a shared target-loop transaction helper

After Plan 1.1 works, remove duplicated transactional logic.

Affected files:

* `stutter/src/actions/transaction.rs` new
* `nice.rs`
* `ioprio.rs`
* `uclamp.rs`
* maybe `cgroup.rs`

Implementation steps:

1. Add a helper type:

   ```rust
   struct ApplyTransaction<R> {
       planned: Vec<R>,
       applied: Vec<R>,
   }
   ```

2. Give it methods:

   * `plan(record)`
   * `mark_applied(record)`
   * `partial_token()`
   * `rollback_applied()`

3. Convert nice/ioprio/uclamp apply loops to use this helper.

4. Keep action-specific setters outside the helper.

5. Add one generic unit test for the helper and one action-level test per action.

Done when: future multi-target actions cannot accidentally reintroduce non-transactional loops.

---

# Plan 2.1 — Add identity-bearing restore records

Affected files:

* `stutter/src/actions/model.rs`
* `stutter/src/actions/token.rs`
* `stutter/src/actions/nice.rs`
* `stutter/src/actions/ioprio.rs`
* `stutter/src/actions/uclamp.rs`
* `stutter/src/actions/cgroup.rs`
* serde compatibility tests

Current problem: restore records mostly store only numeric `tid`/`pid` and old values. That is not safe if Linux reuses a TID/PID.

Implementation steps:

1. Add a reusable restore identity type:

   ```rust
   pub struct TaskRestoreIdentity {
       pub tid: u32,
       pub process_pid: u32,
       pub starttime_ticks: Option<u64>,
       pub comm: Option<String>,
       pub exe: Option<PathBuf>,
   }
   ```

2. Embed it in restore records:

   ```rust
   pub struct NiceRestoreRecord {
       pub tid: u32,
       pub original_nice: i32,
       #[serde(default)]
       pub identity: Option<TaskRestoreIdentity>,
   }
   ```

3. Do the same for:

   * `IoPrioRestoreRecord`
   * `UclampRestoreRecord`
   * `CgroupRestoreRecord`

4. Preserve backward compatibility with old rollback tokens using `#[serde(default)]`.

5. When building rollback records, copy identity from the existing `TaskIdentity` snapshots. The code already collects `tid`, `process_pid`, `comm`, and `starttime_ticks` in `TaskIdentity`.

6. Add tests for deserializing old rollback tokens without identity.

Done when: new rollback tokens carry enough process identity to avoid restoring a reused TID/PID.

---

# Plan 2.2 — Verify identity before every restore

Affected files:

* `stutter/src/actions/restore_identity.rs` new
* `nice.rs`
* `ioprio.rs`
* `uclamp.rs`
* `cgroup.rs`
* rollback/emergency restore tests

Implementation steps:

1. Add a helper:

   ```rust
   enum RestoreIdentityStatus {
       SameTask,
       Missing,
       Mismatch { reason: String },
       UnknownLegacy,
   }
   ```

2. Implement:

   ```rust
   fn verify_task_identity(proc_root: &Path, identity: &TaskRestoreIdentity)
       -> RestoreIdentityStatus
   ```

3. Verification should check:

   * `/proc/<tid>` exists;
   * `/proc/<tid>/stat` start time matches `starttime_ticks`;
   * process pid/thread group still matches when available;
   * `comm` and `exe` can be advisory warnings, not hard requirements.

4. Before restoring nice/ioprio/uclamp/cgroup placement:

   * `SameTask` → restore;
   * `Missing` → skip and warn;
   * `Mismatch` → skip and warn loudly;
   * `UnknownLegacy` → follow chosen compatibility policy.

5. For legacy records without identity, choose one of these policies:

   * safer: skip by default unless `--allow-legacy-restore`;
   * compatible: restore numeric ID but warn that identity could not be verified.

For this codebase, I would choose **compatible for one release, then safer later**, because existing journals may already contain old rollback tokens.

Done when: restore never writes to a reused TID/PID when identity data is available.

---

# Plan 2.3 — Make restore loops `ESRCH`-tolerant

Affected files:

* `nice.rs`
* `ioprio.rs`
* `uclamp.rs`
* `cgroup.rs`

Current problem: nice/ioprio/uclamp restore loops abort on the first failure. Dead tasks should not break the whole rollback.

Implementation steps:

1. Normalize restore errors into:

   ```rust
   enum RestoreWriteError {
       MissingTask,
       PermissionDenied,
       InvalidValue,
       Io(anyhow::Error),
   }
   ```

2. Map `ESRCH` / missing `/proc/<tid>` to `MissingTask`.

3. In each restore loop:

   * skip `MissingTask`;
   * skip identity mismatch;
   * collect real failures;
   * continue restoring remaining records.

4. Return a restore summary:

   ```rust
   struct RestoreSummary {
       restored: usize,
       skipped_missing: usize,
       skipped_identity_mismatch: usize,
       failed: usize,
   }
   ```

5. Only return `Err` if there are real failures after attempting all records.

6. Add tests:

   * dead second task does not prevent first/third restore;
   * identity mismatch is skipped;
   * permission failure is reported after remaining records are attempted.

Done when: rollback is best-effort across the whole token and dead tasks do not poison the entire restore.

---

# Plan 3 — Fix `/autotune/restore`

Affected files:

* `stutter/src/agent/autotune.rs`
* `stutter/src/agent/routes.rs`
* daemon restore/journal code
* `stutter/src/agent/tests.rs`

Current problem: the handler authorizes and audits, then always returns `no_active_apply`.

Implementation steps:

1. Replace the current fake-success response with one of two real behaviors:

   Preferred:

   * inspect active daemon/autotune rollback state;
   * call the same restore path used by emergency restore / daemon restore;
   * return a real restore summary.

   Acceptable temporary fallback:

   * return `501 Not Implemented`;
   * body says remote restore is not implemented;
   * do **not** return `200 no_active_apply`.

2. Wire the handler to the existing restore implementation instead of duplicating rollback logic.

3. Response should include:

   * `status: "restored" | "nothing_to_restore" | "restore_failed"`;
   * number of records restored;
   * number skipped because missing;
   * number skipped because identity mismatch;
   * number failed.

4. Audit outcome should match reality:

   * successful restore → audit success;
   * no active rollback → audit neutral/no-op;
   * restore failure → audit failure.

5. Update existing tests that currently expect safe no-op behavior.

6. Add tests:

   * unauthorized request still fails;
   * no active apply returns true no-op;
   * active rollback invokes restore;
   * restore failure returns non-2xx or explicit failed response.

Done when: `/autotune/restore` either restores something real or clearly says it cannot.

---

# Plan 4.1 — Reap finished recording tasks

Affected files:

* `stutter/src/agent/recording.rs`
* `stutter/src/agent/daemon.rs`
* `stutter/src/agent/tests.rs`

Current problem: `active_run.is_some()` means “active”, even if the join handle has already finished.

Implementation steps:

1. Add:

   ```rust
   async fn reap_finished_recording(state: &AgentState) -> RecordingReapStatus
   ```

2. Inside it:

   * lock `active_run`;
   * check `handle.join.is_finished()`;
   * if not finished, return still active;
   * if finished, `take()` the handle;
   * drop the mutex guard;
   * await the join;
   * update daemon state/result.

3. Call this helper at the start of:

   * recording status handler;
   * recording start handler;
   * daemon status handler.

4. Add tests:

   * finished recording is cleared by status;
   * finished recording no longer blocks new recording start;
   * failed join marks state as failed/degraded.

Done when: natural recording completion clears `active_run`.

---

# Plan 4.2 — Reap finished autotune tasks

Affected files:

* `stutter/src/agent/autotune.rs`
* `stutter/src/agent/daemon.rs`
* `stutter/src/agent/tests.rs`

Implementation steps:

1. Add:

   ```rust
   async fn reap_finished_autotune(state: &AgentState) -> AutotuneReapStatus
   ```

2. Use the same pattern as recording:

   * check `active_autotune`;
   * if join is finished, take handle;
   * drop lock;
   * await join;
   * update daemon/autotune status.

3. Call it before:

   * autotune status;
   * autotune start conflict check;
   * daemon status.

4. Add tests:

   * completed autotune is reaped by status;
   * completed autotune does not block a new session;
   * failed autotune join is exposed as failed/degraded.

Done when: `active_autotune.is_some()` only means actually running, not stale finished handle.

---

# Plan 5.1 — Add Unix socket connection cap

Affected file:

* `stutter/src/agent/server.rs`

Current problem: request middleware rate-limits HTTP requests, but the socket server accepts unlimited open connections and spawns one Tokio task per connection.

Implementation steps:

1. Add a connection limit config value:

   ```rust
   max_agent_unix_connections: usize
   ```

   Default: `64` or `128`.

2. In `serve_unix_socket`, create:

   ```rust
   let connection_permits = Arc::new(Semaphore::new(max_connections));
   ```

3. On accept:

   * try to acquire a permit;
   * if no permit is available, drop the socket immediately and log `agent_unix_connection_limit_reached`;
   * if permit is acquired, move it into the spawned task.

4. The permit must live until the connection future exits.

5. Add metrics/logging for:

   * active connections;
   * rejected connections;
   * connection errors.

6. Add tests with many idle UnixStream clients and assert the cap is enforced.

Done when: idle clients cannot create unlimited spawned tasks.

---

# Plan 5.2 — Add connection idle/read timeout

Affected file:

* `stutter/src/agent/server.rs`

Implementation steps:

1. Add an idle/read timeout config value:

   ```rust
   agent_unix_connection_timeout: Duration
   ```

2. Wrap the connection future:

   ```rust
   let result = tokio::time::timeout(
       connection_timeout,
       HyperConnectionBuilder::new(...)
           .serve_connection_with_upgrades(socket, hyper_service),
   ).await;
   ```

3. On timeout:

   * log debug/warn;
   * drop the socket;
   * release the semaphore permit naturally.

4. Keep the request rate limiter. It still protects request bursts; it just does not protect idle connections.

5. Add tests:

   * idle connection is closed after timeout;
   * active request still works;
   * rejected idle connections do not consume permits forever.

Done when: connection count and idle lifetime are both bounded.

---

# Plan 6.1 — Extend cgroup rollback token to include cpuset state

Affected files:

* `stutter/src/actions/model.rs`
* `stutter/src/actions/token.rs`
* `stutter/src/actions/cgroup.rs`

Current problem: `CgroupPlacementAction::apply()` writes `cpuset.cpus` and `cpuset.mems`, but rollback only moves tasks back.

Implementation steps:

1. Add cpuset rollback metadata:

   ```rust
   pub struct CgroupCpusetRestoreRecord {
       pub cgroup_path: PathBuf,
       pub original_cpuset_cpus: Option<String>,
       pub original_cpuset_mems: Option<String>,
   }
   ```

2. Add it to the cgroup rollback token:

   ```rust
   CgroupRestore {
       records: Vec<CgroupRestoreRecord>,
       cpuset: Option<CgroupCpusetRestoreRecord>,
   }
   ```

3. Use `#[serde(default)]` for backward compatibility.

4. Before writing `cpuset.cpus` or `cpuset.mems`, read the existing values and store them.

5. If the file does not exist, store `None` and do not try to restore that file later.

Done when: the rollback token fully describes every cgroup mutation made by apply.

---

# Plan 6.2 — Make cgroup placement apply transactional

Affected file:

* `stutter/src/actions/cgroup.rs`

Implementation steps:

1. Read original cpuset state before the first cpuset write.

2. Build the cpuset rollback token before mutation.

3. Write `cpuset.cpus`.

4. If that fails, return without mutation.

5. Write `cpuset.mems`.

6. If that fails, restore `cpuset.cpus`.

7. Move tasks one by one.

8. If a task move fails:

   * move already-moved tasks back;
   * restore original cpuset files;
   * return partial rollback/apply failure.

9. Add tests:

   * `cpuset.mems` write fails after `cpuset.cpus` write;
   * second task move fails after first move;
   * rollback restores task cgroup and cpuset files.

Done when: cgroup apply either fully succeeds or restores every mutation it already made.

---

# Plan 7 — Treat `ActiveConfigMatch::Unknown` as degraded

Affected files:

* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/planner/mod.rs`
* reporting/status files

Current problem: planner code handles `Differs`, while `Matches` and `Unknown` both effectively become “no blocker” in some paths.

Implementation steps:

1. Add explicit handling wherever `ActiveConfigMatch` is matched:

   ```rust
   match active_match {
       Matches => ...
       Differs { .. } => ...
       Unknown { reason } => ...
   }
   ```

2. For `Unknown`, choose a conservative policy:

   * do not apply conflicting changes automatically;
   * mark candidate as degraded/blocked;
   * expose the reason in planner output.

3. Add a new reason enum:

   ```rust
   CandidateDenyReason::ActiveConfigUnknown
   ```

4. Include `Unknown` counts in data-quality/report output.

5. Add tests:

   * `Unknown` does not pass as `Matches`;
   * active experiment with unknown config is reported degraded;
   * planner output includes the unknown reason.

Done when: unknown active config is visible and conservative, not silently treated as safe.

---

# Plan 8 — Fix IRQ events with missing timestamps

Affected file:

* `stutter/src/autotune/rolling_window.rs`

Current problem: `push_irq_event()` stores `elapsed_ms: None` events without pruning, while `prune_to()` treats missing timestamps as `0`.

Implementation steps:

1. Pick one policy. Best option: **assign ingestion timestamp**.

2. Change the IRQ insertion path so missing timestamp becomes:

   ```rust
   event.elapsed_ms = Some(current_window_elapsed_ms)
   ```

3. If no current elapsed time exists, use a small bounded side queue instead:

   ```rust
   untimestamped_irq_events: VecDeque<IrqEventRecord>
   ```

   with a hard cap like `128`.

4. Remove `unwrap_or(0)` from pruning.

5. Replace it with one of:

   * guaranteed `Some(elapsed_ms)`;
   * separate bounded queue not pruned by time;
   * drop untimestamped records at insert.

6. Add tests:

   * untimestamped IRQ events cannot grow unbounded;
   * untimestamped events are not treated as ancient timestamp `0`;
   * later timestamped events prune correctly.

Done when: missing IRQ timestamps have one clear policy and cannot distort the rolling window.

---

# Plan 9 — Add collision/degraded handling for block I/O fallback keys

Affected files:

* `stutter-ebpf/src/main.rs`
* `stutter-common/src/lib.rs`
* `stutter/src/ebpf_loader.rs`
* telemetry/reporting code

Current problem: fallback block I/O keys based on sector/dev/rwbs can collide for distinct in-flight requests.

Implementation steps:

1. Add a new drop/degraded counter:

   ```rust
   DROP_BLOCK_FALLBACK_KEY_COLLISION
   ```

2. In the eBPF block start path, before inserting fallback key:

   ```rust
   if using_fallback_key && BLOCK_START.get(&key).is_some() {
       increment_drop_counter(DROP_BLOCK_FALLBACK_KEY_COLLISION);
       BLOCK_START.remove(&key);
       return Ok(0);
   }
   ```

3. Do **not** attribute latency for ambiguous fallback keys.

4. Extend userspace drop counter decoding.

5. Add data quality output:

   * `block_io_fallback_collisions`;
   * warning: “block I/O latency attribution degraded due to fallback key collisions”.

6. Add tests around loader/reporting.

7. Optional stronger fix: include a small per-CPU sequence number in fallback key if feasible, but collision detection is the minimum safe fix.

Done when: ambiguous fallback matches are counted and avoided instead of silently mis-correlated.

---

# Plan 10 — Move `sched_switch` extra reads after cheap filters

Affected file:

* `stutter-ebpf/src/main.rs`

Current problem: `prev_pid_raw` and `prev_state` are read before confirming there is wakeup data and before target-policy filtering.

Implementation steps:

1. Keep the first `next_pid` read at the top.

2. Keep the `next_pid <= 0` early return.

3. Move these reads:

   ```rust
   prev_pid_raw
   prev_state
   ```

   after:

   * `WAKEUP_DATA.get(pid)`;
   * `is_target_pid(pid)`.

4. Only read previous-task context when the event is actually relevant.

5. Keep userspace offset validation.

6. Optional hardening: validate expected field size/type, not just offset.

7. Add a source-level regression test or verifier-friendly test that confirms irrelevant events avoid the extra reads.

Done when: irrelevant `sched_switch` events do less work while preserving existing behavior for relevant events.

---

# Plan 11 — Replace socket `exists()` polling with connect retry

Affected file:

* `stutter/src/daemon/privilege.rs`

Current problem: `wait_for_privileged_worker_socket()` polls `Path::exists()`, which only proves the path exists, not that the worker is accepting connections.

Implementation steps:

1. Replace the loop with `UnixStream::connect(socket_path)` retry.

2. Retry until deadline on:

   * `NotFound`;
   * `ConnectionRefused`;
   * maybe `WouldBlock`.

3. Return success only after connect succeeds.

4. Optional better version: after connecting, send a tiny ping/hello and require a valid response.

5. If handshake is too invasive, connect success is still much better than `exists()`.

6. Add tests:

   * stale socket path exists but connect fails;
   * listener accepts connect and wait succeeds;
   * timeout returns a clear error.

Done when: worker readiness means “connectable”, not “path exists”.

---

# Plan 12 — Do not hold `active_run` lock while awaiting recording shutdown

Affected file:

* `stutter/src/agent/recording.rs`

Current problem: `stop_record_handler()` takes the handle and awaits `handle.join.await` while still inside the locked scope.

Implementation steps:

1. Change this:

   ```rust
   let mut active = state.active_run.lock().await;
   let handle = active.take();
   // await join
   ```

2. To this:

   ```rust
   let handle = {
       let mut active = state.active_run.lock().await;
       active.take()
   };

   if let Some(handle) = handle {
       // await join here, after lock is dropped
   }
   ```

3. Mirror the autotune stop pattern.

4. Add a test:

   * call stop;
   * while stop awaits join, another task can acquire `active_run` lock;
   * test fails if lock is held across await.

Done when: no mutex guard lives across `join.await`.

---

# Plan 13 — Avoid PATH-based foreground helper execution in privileged contexts

Affected file:

* `stutter/src/foreground.rs`

Current problem: commands like `hyprctl` and `swaymsg` are invoked by name, so resolution depends on `PATH`.

Implementation steps:

1. Decide the intended privilege boundary:

   * foreground detection should run as the user, not elevated;
   * privileged daemon paths should not invoke user-resolved helpers.

2. Replace string command names with resolved absolute paths:

   ```rust
   struct SwayForegroundProvider {
       swaymsg: PathBuf,
   }
   ```

3. Resolve helpers at startup from trusted locations:

   * `/usr/bin/swaymsg`
   * `/usr/local/bin/swaymsg` only if explicitly allowed;
   * same for `hyprctl`.

4. When invoking helpers, use sanitized environment:

   * fixed minimal `PATH`;
   * no inherited dangerous env when privileged.

5. Add tests:

   * malicious temporary `PATH` entry does not get selected;
   * configured absolute path is used;
   * missing helper degrades cleanly.

Done when: privileged or semi-privileged code does not depend on ambient `PATH`.

---

# Plan 14 — Exclude zero frametimes if zero means missing data

Affected file:

* `stutter/src/autotune/rolling_window.rs`

Current problem: `frame_p99_ms()` and `frame_max_ms()` accept `0.0` via `>= 0.0`.

Implementation steps:

1. Confirm producer semantics:

   * if `0.0` means real frame time, keep it;
   * if `0.0` means missing/skipped/unresolved, filter it.

2. Assuming zero means missing, change filters from:

   ```rust
   value >= 0.0
   ```

   to:

   ```rust
   value > 0.0
   ```

3. Add a counter for dropped invalid frametimes:

   * negative;
   * non-finite;
   * zero.

4. Add tests:

   * `[0.0, 16.0, 20.0]` reports from `16.0/20.0`, not `0.0`;
   * all-zero frames return `None`;
   * negative/non-finite are still ignored.

Done when: missing frame data cannot improve latency stats by appearing as `0.0 ms`.

---

# Plan 15 — Replace string mode checks with enum checks

Affected file:

* `stutter/src/agent/autotune.rs`

Current problem: code parses `DaemonMode`, then later compares string values like `"apply-low-risk"`.

Implementation steps:

1. Compute once:

   ```rust
   let is_apply_low_risk = policy.mode == DaemonMode::ApplyLowRisk;
   ```

2. Use that bool everywhere currently doing:

   ```rust
   mode == "apply-low-risk"
   ```

3. Keep `mode.as_str()` only for response serialization/log text.

4. Add a test for `ApplyLowRisk` behavior:

   * duration is `None`;
   * runtime config uses low-risk apply behavior;
   * response message is correct.

Done when: control flow depends on the enum, not string spelling.

---

# Plan 16 — Harden `push_intervals()` ordering assumptions

Affected file:

* `stutter/src/autotune/rolling_window.rs`

Current problem: `push_intervals()` appends a batch and prunes using max elapsed timestamp. This is okay only if records are monotonic or same-tick.

Implementation steps:

1. Choose the complete fix: make it robust to out-of-order records.

2. Sort incoming batch by `elapsed_ms` before appending:

   ```rust
   records.sort_by_key(|record| record.elapsed_ms);
   ```

3. After appending, either:

   * keep `VecDeque` sorted and use front-prune; or
   * use `retain`-style pruning for this stream.

4. For performance, sorting a batch is probably enough if all existing records are already monotonic.

5. Add tests:

   * batch arrives out of order;
   * old record after new record is pruned;
   * same-tick batch still behaves as today;
   * empty batch no-op.

6. Document the remaining assumption if full global sorting is not implemented.

Done when: an out-of-order interval batch cannot leave stale records behind the prune boundary.

---

# Plan 17 — Do not “fix” `irq_key()` as a collision bug

Affected file:

* `stutter-ebpf/src/main.rs`

Current code:

```rust
((irq as u64) << 32) | cpu as u64
```

That gives IRQ the high 32 bits and CPU the low 32 bits. A CPU ID of `65536` does **not** collide; it is still inside the low 32-bit half.

Implementation steps:

1. Close the original issue as not founded.

2. Add a comment near `irq_key()` explaining the layout:

   ```rust
   // high 32 bits: irq
   // low 32 bits: cpu
   ```

3. Optional: add a small host-side unit test for the packing helper if the helper can be shared outside eBPF.

4. Optional: sanity-log absurd CPU IDs elsewhere, but do not treat `>= 65536` as a key-overlap bug.

Done when: no functional change is made for a nonexistent collision, but the encoding is documented.



PHASE 3:




# 1. Automatic GPU/display topology detection

## Existing code to build on

Current config has:

```rust
MonitorConfig {
    kms_timing: KmsTimingConfig,
    drm_fence: DrmFenceConfig,
    wayland_presentation: WaylandPresentationConfig,
    display_path: DisplayPathConfig,
}
```

`DisplayPathConfig` currently only stores:

```rust
label
render_gpu
scanout_gpu
connector
```

And session metadata currently only writes those same manual fields through `display_path_metadata()` in:

```text
stutter/src/recorder/session.rs
```

The CLI already exposes manual fields:

```text
--display-path-label
--display-render-gpu
--display-scanout-gpu
--display-connector
```



## Goal

Detect this automatically:

```text
render GPU:       card1 / renderD129 / amdgpu / RX 9070 XT
scanout GPU:      card0 / i915 / UHD630
active connector: HDMI-A-1 / DP-1
session:          wayland/x11/gamescope
compositor:       kwin_wayland / gnome-shell / gamescope / sway
cross-GPU path:   true/false/unknown
```

## New module

Add:

```text
stutter/src/display_topology.rs
```

Core structs:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayTopologySnapshot {
    pub collected_at_elapsed_ms: Option<u64>,
    pub session_type: Option<String>,
    pub compositor: Option<CompositorInfo>,
    pub drm_devices: Vec<DrmDeviceInfo>,
    pub connectors: Vec<ConnectorInfo>,
    pub guessed_path: Option<DisplayPathGuess>,
    pub warnings: Vec<String>,
}

pub struct DrmDeviceInfo {
    pub card: String,                  // card0
    pub render_node: Option<String>,   // /dev/dri/renderD128
    pub driver: Option<String>,        // i915, amdgpu
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub pci_slot: Option<String>,
    pub boot_vga: Option<bool>,
    pub hwmon_paths: Vec<PathBuf>,
}

pub struct ConnectorInfo {
    pub card: String,
    pub name: String,                  // HDMI-A-1, DP-1
    pub status: Option<String>,        // connected/disconnected
    pub enabled: Option<String>,
    pub modes: Vec<String>,
    pub edid_hash: Option<String>,
}

pub struct DisplayPathGuess {
    pub render_card: Option<String>,
    pub render_driver: Option<String>,
    pub scanout_card: Option<String>,
    pub scanout_driver: Option<String>,
    pub connector: Option<String>,
    pub is_cross_gpu: Option<bool>,
    pub confidence: String,
    pub reasons: Vec<String>,
}
```

## Collection logic

Use `/sys/class/drm`:

```text
/sys/class/drm/card0
/sys/class/drm/card0-HDMI-A-1/status
/sys/class/drm/card0-HDMI-A-1/enabled
/sys/class/drm/card0-HDMI-A-1/modes
/sys/class/drm/card0/device/vendor
/sys/class/drm/card0/device/device
/sys/class/drm/card0/device/driver
/sys/class/drm/renderD128
```

The repo already has similar inventory logic in `stutter/src/system_inventory.rs`, so do **not** duplicate everything blindly. Either:

1. move the useful DRM device scanning into `display_topology.rs`, or
2. extend `system_inventory.rs` and have `display_topology.rs` call it.

I would choose option 2.

## Artifact changes

Add to `stutter/src/artifacts.rs`:

```rust
ArtifactKind::DisplayTopology
```

Add artifact spec:

```rust
ArtifactSpec {
    kind: ArtifactKind::DisplayTopology,
    file_name: "display_topology.json",
    encoding: ArtifactEncoding::JsonObject,
    required: false,
    legacy_aliases: &[],
    counter_field: None,
}
```

Add loader field in `stutter/src/session_io.rs`:

```rust
pub display_topology: Option<DisplayTopologySnapshot>,
```

Add this to `RunArtifacts`.

## Session metadata changes

Extend `DisplayPathMetadata` in:

```text
stutter/src/recorder/session_files.rs
```

Current metadata only has:

```rust
label
render_gpu
scanout_gpu
connector
```

Add:

```rust
pub render_card: Option<String>,
pub render_render_node: Option<String>,
pub render_driver: Option<String>,
pub scanout_card: Option<String>,
pub scanout_driver: Option<String>,
pub is_cross_gpu: Option<bool>,
pub session_type: Option<String>,
pub compositor: Option<String>,
pub topology_confidence: Option<String>,
pub topology_warnings: Vec<String>,
```

Use `#[serde(default)]` on every new field so old runs remain readable.

## Recorder integration

In `stutter/src/recorder/session.rs`, replace the current `display_path_metadata(config)` with something like:

```rust
fn display_path_metadata(
    config: &MonitorConfig,
    topology: Option<&DisplayTopologySnapshot>,
) -> Option<DisplayPathMetadata>
```

Priority order:

```text
CLI/config override
→ detected topology
→ unknown
```

## Tests

Add unit tests:

```text
stutter/src/display_topology.rs
```

with fake sysfs directories:

```text
fixtures/sysfs/uhd630_scanout_rx9070xt_render/
fixtures/sysfs/direct_rx9070xt_scanout/
```

Test cases:

```text
Intel card has connected HDMI, AMD card has render node → scanout=i915, render=amdgpu
AMD card has connected DP → direct path
no connected connector → unknown, warning
```

---

# 2. Direct-scanout detection

## Existing code to build on

The project already has `WaylandPresentationEventRecord`:

```rust
zero_copy: Option<bool>,
discarded: bool,
flags: Vec<String>,
surface_role: Option<String>,
source: String,
```

And the Wayland docs already say arbitrary Wayland client presentation feedback requires cooperative support from the client, compositor, Gamescope, or wrapper. 

So the implementation must be honest:

```text
direct scanout: yes/no/unknown
```

not:

```text
direct scanout: no
```

when evidence is missing.

## Add report model

In:

```text
stutter/src/report/model.rs
```

add:

```rust
#[derive(Debug, Clone, Serialize, Default)]
pub struct DirectScanoutSummary {
    pub status: String, // "yes", "no", "mixed", "unknown"
    pub confidence: String,
    pub zero_copy_ratio: Option<f64>,
    pub direct_scanout_event_count: usize,
    pub composited_event_count: usize,
    pub blocking_reasons: Vec<String>,
    pub evidence: Vec<String>,
    pub notes: Vec<String>,
}
```

Then add this into a larger `DisplayPathDiagnosisSummary`, covered below.

## Analysis logic

In:

```text
stutter/src/report/analysis.rs
```

add:

```rust
fn build_direct_scanout_summary(
    wayland_events: &[WaylandPresentationEventRecord],
    topology: Option<&DisplayTopologySnapshot>,
) -> DirectScanoutSummary
```

Rules:

```text
if zero_copy true for most game/gamescope_output events:
    status = yes or mixed
if zero_copy false for most game/gamescope_output events:
    status = no
if flags contain composited:
    status = no
if flags contain direct_scanout:
    status = yes
if no cooperative events:
    status = unknown
```

Recognize flags:

```text
direct_scanout
composited
overlay_active
scaling
fractional_scaling
hdr
vrr_constraint
format_modifier_mismatch
cursor_plane_fallback
multi_monitor_constraint
```

Do not invent those flags from nowhere; just define them as a supported log schema for future compositor/Gamescope integrations.

## CLI additions

In `stutter/src/cli/monitor.rs`:

```rust
#[arg(long = "direct-scanout-log", value_name = "PATH")]
pub(super) direct_scanout_log: Option<PathBuf>,
```

This can initially reuse the existing Wayland presentation log machinery if the schema is extended, or become a separate artifact later.

## Phase split

### Phase A: derived-only

Use existing `wayland_presentation_events.json`.

No new live probe.

### Phase B: compositor-specific helpers

Add:

```text
stutter/src/direct_scanout.rs
```

Support external NDJSON first:

```json
{
  "elapsed_ms": 1234,
  "source": "gamescope",
  "surface_role": "game",
  "direct_scanout": false,
  "reason": "format_modifier_mismatch"
}
```

### Phase C: optional compositor adapters

Later add optional helpers for:

```text
gamescope
kwin_wayland
sway
mutter
```

But keep these optional because compositor internals change.

---

# 3. Stronger cross-GPU wait attribution

## Existing code to build on

`DrmFenceEventRecord` already carries:

```rust
gpu_role
context
seqno
timeline_hash
wait_start_ns
wait_done_ns
duration_ns
exporter_driver
importer_driver
correlation_basis
confidence
```

The report already computes:

```rust
cross_gpu_candidate_count
waits_near_frame_outliers
waits_near_kms_delays
```

in `build_drm_fence_timing_summary()`. The existing logic is useful but still coarse.

## First fix: preserve signal time

`stutter-common/src/lib.rs` has `DrmFenceEvent` with `signal_ns`, but `DrmFenceEventRecord` does **not** currently preserve it.

Add to:

```text
stutter/src/recorder/event_types.rs
```

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub signal_ns: Option<u64>,
```

Then update conversion in:

```text
stutter/src/session.rs
```

inside the `EVENT_DRM_FENCE` branch:

```rust
signal_ns: (event.signal_ns != 0).then_some(event.signal_ns),
```

This avoids touching eBPF layout.

## Add cross-GPU fence analysis struct

In `stutter/src/report/model.rs`:

```rust
#[derive(Debug, Clone, Serialize, Default)]
pub struct CrossGpuFenceSummary {
    pub candidate_count: usize,
    pub high_confidence_count: usize,
    pub display_side_wait_count: usize,
    pub render_side_wait_count: usize,
    pub waits_near_frame_outliers: usize,
    pub waits_near_kms_delays: usize,
    pub p95_display_wait_ms: Option<f64>,
    pub p99_display_wait_ms: Option<f64>,
    pub top_candidates: Vec<CrossGpuFenceCandidate>,
    pub confidence: String,
    pub notes: Vec<String>,
}

pub struct CrossGpuFenceCandidate {
    pub elapsed_ms: u64,
    pub duration_ms: Option<f64>,
    pub wait_start_ns: Option<u64>,
    pub wait_done_ns: Option<u64>,
    pub signal_ns: Option<u64>,
    pub importer_driver: Option<String>,
    pub exporter_driver: Option<String>,
    pub context: Option<u64>,
    pub seqno: Option<u64>,
    pub timeline_hash: Option<u64>,
    pub near_frame_outlier: bool,
    pub near_kms_delay: bool,
    pub confidence: String,
}
```

## Improve analysis

In:

```text
stutter/src/report/analysis.rs
```

split current `build_drm_fence_timing_summary()` into:

```rust
build_drm_fence_timing_summary(...)
build_cross_gpu_fence_summary(...)
```

Candidate rule:

```text
duration exists
AND display-side/importer-side role
AND exporter/importer suggests amdgpu → i915
AND event is near a frame outlier OR KMS delay for higher confidence
```

Confidence:

```text
high:
  context+seqno or timeline+seqno
  importer=i915/display
  exporter=amdgpu/render
  wait near frame outlier
  wait near KMS/presentation delay

medium:
  stable fence identity but missing one side

low:
  role/source overlap only
```

## Enrich driver/card fields

Currently `session.rs` emits DRM fence events with:

```rust
driver: None,
card: None,
comm: None,
```

Implement a userspace enrichment layer:

```text
stutter/src/display_enrichment.rs
```

or inside `session.rs` initially:

```rust
fn enrich_drm_fence_event(
    raw: DrmFenceEventRecord,
    topology: Option<&DisplayTopologySnapshot>,
    config: &MonitorConfig,
) -> DrmFenceEventRecord
```

Rules:

```text
gpu_role=render → config.drm_fence.render_card or topology.guessed_path.render_card
gpu_role=display → config.drm_fence.display_card or topology.guessed_path.scanout_card
source=amdgpu → driver=amdgpu
source=i915 → driver=i915
pid/tid present → try /proc/<tid>/comm or /proc/<pid>/comm
```

Do the same for KMS events:

```text
source=i915 → card=scanout_card
connector=config.kms_timing.connector or topology guessed connector
```

## Tests

Synthetic fixture:

```text
amdgpu signal event
i915 wait start/end event
MangoHud frame outlier nearby
KMS long flip nearby
```

Expected:

```text
cross_gpu_fence_summary.candidate_count > 0
confidence = high
display_path diagnosis includes cross_gpu_fence_wait_candidate
```

---

# 4. DMABUF format/modifier tracking

This one is big. Do it in three phases.

## Why it matters

For this user’s exact problem, the bad path is often:

```text
dGPU rendered buffer cannot be imported/scanned out efficiently by iGPU
→ compositor/copy/linearization path
→ frame pacing loss
```

You need evidence about:

```text
format
modifier
linear vs tiled
scanout-capable yes/no
allocation GPU
import GPU
copy/linearization hint
```

## Phase A: cooperative DMABUF log ingestion

Add new record in:

```text
stutter/src/recorder/event_types.rs
```

```rust
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct DmaBufEventRecord {
    pub elapsed_ms: u64,
    pub source: String,
    pub app_id: Option<String>,
    pub surface_role: Option<String>,
    pub output_name: Option<String>,

    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<String>,
    pub modifier: Option<String>,
    pub modifier_name: Option<String>,
    pub planes: Option<u32>,

    pub allocation_driver: Option<String>,
    pub import_driver: Option<String>,
    pub allocation_card: Option<String>,
    pub import_card: Option<String>,

    pub linear: Option<bool>,
    pub scanout_capable: Option<bool>,
    pub zero_copy: Option<bool>,
    pub explicit_sync: Option<bool>,
    pub copy_required: Option<bool>,

    pub reason: Option<String>,
    pub confidence: String,
}
```

Add:

```rust
ArtifactKind::DmaBufEvents
ArtifactCounter::DmaBufEvent
```

File:

```text
dmabuf_events.json
```

Update:

```text
stutter/src/artifacts.rs
stutter/src/session_io.rs
stutter/src/session_events.rs
stutter/src/session/sinks.rs
```

Add parser:

```text
stutter/src/dmabuf_log.rs
```

Modeled after:

```text
stutter/src/wayland_presentation.rs
```

CLI:

```rust
#[arg(long = "dmabuf-log", value_name = "PATH")]
pub(super) dmabuf_log: Option<PathBuf>
```

Config:

```rust
MonitorConfig {
    dmabuf: DmaBufConfig,
}

ProbeConfig {
    dmabuf_tracking: bool,
}
```

Layer/effective/config schema must be updated too:

```text
stutter/src/config/model.rs
stutter/src/config/layer.rs
stutter/src/config/effective.rs
stutter/src/config/schema.rs
stutter/src/cli/map/monitor.rs
```

## Phase B: report summary

Add:

```rust
pub struct DmaBufPathSummary {
    pub event_count: usize,
    pub linear_count: usize,
    pub scanout_capable_count: usize,
    pub copy_required_count: usize,
    pub modifier_mismatch_count: usize,
    pub cross_gpu_import_count: usize,
    pub top_reasons: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}
```

Add `build_dmabuf_path_summary()` in `report/analysis.rs`.

## Phase C: tracepoint support

Only after cooperative logs work.

Create:

```text
stutter/src/dmabuf_tracepoints.rs
```

Look for kernel tracepoints under:

```text
/sys/kernel/tracing/events/dma_buf
/sys/kernel/tracing/events/sync_file
/sys/kernel/tracing/events/drm
```

But keep language cautious. Many tracepoints will not expose DRM format modifiers. Treat them as:

```text
import/copy/sync evidence
```

not exact buffer layout proof.

## Tests

Add parser tests with NDJSON:

```json
{"elapsed_ms":1000,"source":"gamescope","surface_role":"game","format":"XRGB8888","modifier":"LINEAR","allocation_driver":"amdgpu","import_driver":"i915","scanout_capable":false,"copy_required":true,"reason":"modifier_mismatch","confidence":"medium"}
```

Expected:

```text
modifier_mismatch_count = 1
cross_gpu_import_count = 1
copy_required_count = 1
```

---

# 5. Built-in Intel + AMD engine sampling

## Existing code to build on

Current `GpuSample` is generic and one-GPU-oriented:

```rust
GpuSample {
    drm_card,
    render_node,
    gpu_busy_percent,
    vram,
    clocks,
    temp,
    power,
}
```

`MonitorSession::handle_hwmon_tick()` samples one `HwmonReader`.

That is not enough for this exact case. You need both:

```text
AMD render GPU activity
Intel iGPU render/blitter/display activity
```

## Add new record

In `stutter/src/recorder/event_types.rs`:

```rust
#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct GpuEngineSample {
    pub elapsed_ms: u64,
    pub drm_card: Option<String>,
    pub render_node: Option<String>,
    pub driver: Option<String>,
    pub engine: String,              // gfx, sdma0, rcs0, bcs0, display, blitter
    pub busy_percent: Option<f64>,
    pub client_pid: Option<u32>,
    pub client_comm: Option<String>,
    pub source: String,              // hwmon, pmu, fdinfo, debugfs
    pub confidence: String,
}
```

Add artifact:

```text
gpu_engine_samples.json
```

Add enum variants:

```rust
ArtifactKind::GpuEngineSamples
ArtifactCounter::GpuEngineSample
MonitorEvent::GpuEngineSample
```

Update:

```text
stutter/src/artifacts.rs
stutter/src/session_io.rs
stutter/src/session_events.rs
stutter/src/session/sinks.rs
```

## New sampler module

Add:

```text
stutter/src/gpu_engine.rs
```

Core trait:

```rust
trait EngineSampler {
    fn sample(&mut self, elapsed_ms: u64) -> Vec<GpuEngineSample>;
}
```

Implement in phases.

### Phase A: multi-GPU hwmon

Extend current `HwmonReader` use rather than replacing it.

Add:

```rust
pub struct MultiGpuHwmonReader {
    readers: Vec<HwmonReader>,
}
```

Discovery:

```text
render card from topology
scanout card from topology
manual --hwmon-drm-card still supported
```

In `session.rs`, change:

```rust
hwmon_reader: Option<Arc<Mutex<HwmonReader>>>
```

to:

```rust
hwmon_reader: Option<Arc<Mutex<MultiGpuHwmonReader>>>
```

or add a second sampler beside the old one to avoid breaking existing code.

### Phase B: Intel i915 engine PMU/fdinfo

For UHD630, the important engines are:

```text
Render/3D: rcs
Blitter: bcs
Video: vcs
```

Use one or more sources:

```text
/proc/<pid>/fdinfo/<fd> DRM engine accounting
/sys/bus/event_source/devices/i915/events/*-busy
/sys/kernel/debug/dri/N/i915_engine_info
```

Do not require debugfs for baseline functionality.

### Phase C: AMD engine evidence

For AMD:

```text
GFX busy
SDMA/copy activity
VRAM/GTT usage
PCIe throughput if exposed
```

Sources may include:

```text
/sys/class/drm/cardN/device/gpu_busy_percent
amdgpu hwmon
debugfs amdgpu_pm_info if available
fdinfo if kernel exposes drm-engine fields
```

## Report use

The important report logic is:

```text
AMD GFX not saturated
AND Intel Blitter/Render active during frame outliers
→ compositor/copy/display-path suspicion increases
```

Add helper in `report/analysis.rs`:

```rust
fn gpu_engine_activity_near_outliers(
    engine_samples: &[GpuEngineSample],
    frame_events: &[FrameEvent],
) -> GpuEngineActivitySummary
```

## Tests

Use fake fdinfo/debugfs samples:

```text
i915 bcs busy rises near frame outlier
amdgpu gfx busy 60%
```

Expected:

```text
display_path diagnosis includes "igpu_blitter_activity_near_outliers"
```

---

# 6. Separate compositor overhead from KMS/fence overhead

## Existing problem

Right now the project has separate summaries:

```text
kms_timing
drm_fence_timing
wayland_presentation
frame_pacing
```

But there is no single report section that says:

```text
This looks like:
  - render GPU bottleneck
  - display-side fence wait
  - KMS/pageflip delay
  - compositor/presentation queue delay
  - iGPU blitter/render work
```

## Add DisplayPathDiagnosisSummary

In `stutter/src/report/model.rs`:

```rust
#[derive(Debug, Clone, Serialize, Default)]
pub struct DisplayPathDiagnosisSummary {
    pub verdict: String,
    pub suspicion_score: f64,
    pub confidence: String,

    pub render_gpu: Option<String>,
    pub scanout_gpu: Option<String>,
    pub connector: Option<String>,
    pub is_cross_gpu: Option<bool>,

    pub direct_scanout: DirectScanoutSummary,
    pub cross_gpu_fence: CrossGpuFenceSummary,
    pub dmabuf_path: Option<DmaBufPathSummary>,
    pub gpu_engine_activity: Option<GpuEngineActivitySummary>,

    pub render_component: DisplayPathComponent,
    pub fence_component: DisplayPathComponent,
    pub kms_component: DisplayPathComponent,
    pub wayland_component: DisplayPathComponent,
    pub compositor_component: DisplayPathComponent,

    pub evidence: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub notes: Vec<String>,
}

pub struct DisplayPathComponent {
    pub status: String,       // healthy, candidate, likely, unknown
    pub score: f64,
    pub evidence: Vec<String>,
}
```

Add to `ReportAnalysisJson`:

```rust
pub display_path_diagnosis: DisplayPathDiagnosisSummary,
```

Then update:

```text
stutter/src/report/analysis.rs
```

inside `build_report_analysis_from_input()` after existing summaries:

```rust
let display_path_diagnosis = build_display_path_diagnosis_summary(
    &session,
    &artifacts,
    &frame_pacing,
    &kms_timing,
    &drm_fence_timing,
    &wayland_presentation,
);
```

## Classification rules

### Render component

Likely render bottleneck if:

```text
AMD/render GPU busy high
render-side fence waits high
game thread scheduling cluster dominates
```

### Fence component

Likely display-path issue if:

```text
display-side fence p99 > 1ms
cross-GPU candidate waits near frame outliers
waits near KMS delays
```

### KMS component

Likely scanout/pageflip issue if:

```text
KMS p99 > 1ms
long KMS flips near frame outliers
KMS delays increased in A/B comparison
```

### Wayland component

Likely compositor/presentation issue if:

```text
commit-to-present p99 high
discarded frames present
zero_copy false/missing when direct scanout expected
compositor_queue_candidate_count > 0
```

### Compositor component

Likely compositor overhead if:

```text
compositor scheduler clusters near frame outliers
iGPU Render/Blitter activity near frame outliers
direct scanout false
```

## Diagnosis integration

In `stutter/src/diagnosis.rs`, add causes:

```rust
CrossGpuDisplayPathCandidate
DisplayFenceWaitCandidate
KmsPageflipDelayCandidate
WaylandPresentationQueueCandidate
CompositorRenderCandidate
```

Add evidence kinds:

```rust
DrmFenceWait
KmsPageflipDelay
WaylandPresentationDelay
DirectScanoutStatus
GpuEngineBusy
DmaBufModifierMismatch
```

But I would make this **phase 2**. First get the report summary working. Then feed it into per-cluster diagnosis.

---

# 7. Better A/B workflow

## Existing code to build on

`stutter compare display-path` already exists and is the right foundation. The docs describe exactly the desired use case: compare a dGPU-display baseline against a UHD630/i915 scanout test, using display metadata and reporting frame-pacing, KMS, DRM-fence, Wayland-presentation, and scheduler deltas. 

The current compare code already has:

```rust
DisplayPathCompareInput {
    baseline,
    test,
    json,
}
```

and:

```rust
DisplayPathCostSummary
```

with many useful deltas. 

## Add preset

In `stutter/src/presets.rs`, add:

```rust
Preset::PrimeDisplayPath
```

Add to `VALID_PRESETS`:

```rust
"prime-display-path"
```

Extend `PresetDefaults` to include:

```rust
pub kms_timing: Option<bool>,
pub drm_fence_latency: Option<bool>,
pub wayland_presentation: Option<bool>,
pub foreground_window: Option<bool>,
pub runtime_slices: Option<bool>,
pub gpu_engine_sampling: Option<bool>,
pub display_topology: Option<bool>,
```

Then update:

```text
stutter/src/config/layer.rs
MonitorConfigLayer::from_preset_defaults()
```

Prime preset defaults:

```rust
Preset::PrimeDisplayPath => PresetDefaults {
    hwmon: Some(true),
    cpu_freq: Some(true),
    faults: Some(true),
    stat_wait: Some(true),
    runtime_slices: Some(true),
    kms_timing: Some(true),
    drm_fence_latency: Some(true),
    wayland_presentation: None, // only true if log/source configured
    foreground_window: Some(true),
    gpu_engine_sampling: Some(true),
    display_topology: Some(true),
    ...
}
```

Do not automatically force Wayland presentation unless there is a source; otherwise it will create empty evidence.

## Add guided command

I would add a lightweight workflow first:

```bash
stutter compare display-path \
  --baseline <direct-run> \
  --test <uhd630-run> \
  --expect direct-to-offload \
  --strict
```

Modify:

```text
stutter/src/cli/report.rs
stutter/src/commands/input.rs
stutter/src/commands/misc.rs
stutter/src/display_path_compare.rs
```

Add fields:

```rust
pub strict: bool,
pub expect: Option<DisplayPathExpectation>,
```

Expectation enum:

```rust
DirectToOffload
OffloadToDirect
Unknown
```

## Improve comparability validation

Current `validate_comparability()` checks general things like duration and missing data.

Add:

```text
same app/process
same frame count rough range
same duration rough range
same compositor/session type
same render GPU
same resolution/refresh if topology has it
baseline scanout GPU != test scanout GPU if expected
test is cross-GPU if expected
same MangoHud logging source
same probe availability
```

Add warnings:

```text
"comparison downgraded: test and baseline used different refresh modes"
"comparison downgraded: render GPU changed"
"comparison downgraded: missing DRM fence evidence in one run"
```

## Better output

Expand `DisplayPathCostSummary`:

```rust
pub verdict: String,
pub confidence_score: f64,
pub evidence: Vec<String>,
pub missing_evidence: Vec<String>,

pub direct_scanout_status_delta: Option<String>,
pub igpu_engine_activity_delta: Option<f64>,
pub dmabuf_copy_required_delta: Option<i64>,

pub fence_component_delta_ms: Option<f64>,
pub kms_component_delta_ms: Option<f64>,
pub wayland_component_delta_ms: Option<f64>,
pub compositor_component_delta_ms: Option<f64>,
```

Human output should become:

```text
Display-path A/B verdict:
  UHD630 scanout likely hurt this run.

Measured cost:
  avg FPS: -8.4%
  p99 frame: +2.2 ms
  p99 display fence wait: +1.4 ms
  KMS p99: +0.5 ms
  Wayland commit-to-present p99: +1.1 ms

Likely components:
  cross-GPU fence wait: likely
  compositor/presentation queue: candidate
  KMS pageflip delay: weak candidate
  render GPU bottleneck: unlikely

Confidence: medium-high
```

## Later: full wizard

After the preset is stable, add:

```bash
stutter display-path wizard
```

But that is bigger because it needs process launching, prompts, state storage, and maybe privileged re-entry. Do the preset and strict compare first.

---

# 8. Single-run suspicion score

This is very useful because users may not want to move cables immediately. But it must be explicit:

```text
This does not calculate exact FPS loss.
It detects whether this run shows signs of display-path overhead.
```

## Implement as derived report summary

Do not make it a live probe first.

In `report/analysis.rs`:

```rust
fn build_display_path_suspicion_score(
    session: &SessionFile,
    artifacts: &RunArtifacts,
    summaries: &ExistingSummaries,
) -> SuspicionScore
```

Scoring example:

```text
+0.20 render GPU != scanout GPU
+0.25 display-side fence wait p99 > 1ms
+0.25 cross-GPU waits near frame outliers
+0.15 KMS p99 > 1ms near frame outliers
+0.15 Wayland commit-to-present p99 high
+0.10 zero_copy_ratio low/false when evidence exists
+0.20 iGPU Render/Blitter activity near frame outliers
+0.10 AMD render GPU not saturated during outliers
-0.20 AMD render GPU saturated, normal GPU bottleneck likely
```

Cap:

```text
0.0 to 1.0
```

Verdict:

```text
0.00–0.25 unknown/low
0.25–0.50 possible
0.50–0.75 likely
0.75–1.00 very likely
```

Confidence should be separate from suspicion:

```text
suspicion_score = how display-path-like the symptoms are
confidence = how much evidence was available
```

That distinction matters. A run with no DRM fence/KMS/Wayland data should not get high confidence.

## Output

Add to text report:

```text
Display path diagnosis:
  Suspicion: likely
  Score: 0.68
  Confidence: medium

Evidence:
  - render GPU and scanout GPU differ: amdgpu → i915
  - display-side fence p99: 1.7 ms
  - 12 fence waits near frame outliers
  - KMS p99: 0.8 ms
  - zero-copy evidence unavailable

Missing evidence:
  - no DMABUF modifier log
  - no iGPU engine samples
```

Update:

```text
stutter/src/report/text.rs
stutter/src/report/html.rs
```

---

# Cross-cutting implementation details

## Probe registry

`DisplayPathCost` is currently `ViewOnly`, while DRM fence and Wayland presentation are optional probes with limitations. 

Add probe keys:

```rust
DisplayTopology
GpuEngineSampling
DmaBufPath
DirectScanoutStatus
```

In:

```text
stutter/src/probe_registry.rs
```

Suggested statuses:

```text
DisplayTopology: Implemented, low overhead
GpuEngineSampling: Implemented/Experimental, medium overhead
DmaBufPath: Experimental, cooperative log first
DirectScanoutStatus: ViewOnly or ExternalLog depending on implementation
```

Update:

```text
stutter/src/probe_activation.rs
```

So `prime-display-path` activates:

```text
display topology
KMS timing
DRM fence latency
hwmon
gpu engine sampling
foreground window
runtime slices
Wayland presentation only when source/log configured
```

## Artifact compatibility

Every new field should have:

```rust
#[serde(default)]
```

Every new artifact should be optional.

Old runs must still load.

Update:

```text
stutter/src/artifacts/compat_v20.rs
stutter/src/artifacts/compat_v21.rs
```

only if the compatibility tests require schema awareness.

## Docs

Add:

```text
docs/PRIME_DISPLAY_PATH.md
docs/DMABUF_PATH_LOG.md
docs/DIRECT_SCANOUT_LOG.md
```

Update:

```text
docs/ARTIFACT_SCHEMA.md
docs/PROBE_ADMISSION.md
docs/DRM_FENCE_COMPATIBILITY.md
docs/WAYLAND_PRESENTATION_LOG.md
```

The docs should keep the project’s current cautious tone:

```text
candidate attribution, not exact copy latency
missing evidence is unavailable evidence, not proof of health
A/B estimate, not photon latency
```

---

# Suggested implementation phases

## Phase 1: topology + preset

Files:

```text
stutter/src/display_topology.rs
stutter/src/system_inventory.rs
stutter/src/artifacts.rs
stutter/src/session_io.rs
stutter/src/recorder/session_files.rs
stutter/src/recorder/session.rs
stutter/src/presets.rs
stutter/src/config/model.rs
stutter/src/config/layer.rs
stutter/src/config/effective.rs
stutter/src/cli/monitor.rs
```

Deliverable:

```bash
stutter record --preset prime-display-path ...
```

produces:

```text
display_topology.json
session.core.display_path with detected render/scanout cards
```

## Phase 2: display-path diagnosis summary

Files:

```text
stutter/src/report/model.rs
stutter/src/report/analysis.rs
stutter/src/report/text.rs
stutter/src/report/html.rs
stutter/src/display_path_compare.rs
```

Deliverable:

```text
single-run display-path suspicion score
clear component split:
  render / fence / KMS / Wayland / compositor
```

## Phase 3: stronger fence attribution

Files:

```text
stutter/src/recorder/event_types.rs
stutter/src/session.rs
stutter/src/report/model.rs
stutter/src/report/analysis.rs
stutter/src/diagnosis.rs
```

Deliverable:

```text
cross-GPU fence candidate list with signal_ns, wait interval, importer/exporter, nearby frame/KMS evidence
```

## Phase 4: multi-GPU engine sampling

Files:

```text
stutter/src/gpu_engine.rs
stutter/src/hwmon.rs
stutter/src/session.rs
stutter/src/session_events.rs
stutter/src/session/sinks.rs
stutter/src/artifacts.rs
stutter/src/session_io.rs
stutter/src/report/analysis.rs
```

Deliverable:

```text
gpu_engine_samples.json
i915 Render/Blitter evidence near frame outliers
AMD GFX/SDMA evidence near frame outliers
```

## Phase 5: direct-scanout evidence

Files:

```text
stutter/src/wayland_presentation.rs
stutter/src/direct_scanout.rs
stutter/src/report/analysis.rs
docs/WAYLAND_PRESENTATION_LOG.md
docs/DIRECT_SCANOUT_LOG.md
```

Deliverable:

```text
direct_scanout: yes/no/mixed/unknown
blocking reasons when compositor/Gamescope provides them
```

## Phase 6: DMABUF/modifier evidence

Files:

```text
stutter/src/dmabuf_log.rs
stutter/src/recorder/event_types.rs
stutter/src/session_events.rs
stutter/src/session/sinks.rs
stutter/src/artifacts.rs
stutter/src/session_io.rs
stutter/src/report/model.rs
stutter/src/report/analysis.rs
docs/DMABUF_PATH_LOG.md
```

Deliverable:

```text
dmabuf_events.json
modifier/linear/copy-required summary
```

## Phase 7: stronger A/B compare

Files:

```text
stutter/src/display_path_compare.rs
stutter/src/cli/report.rs
stutter/src/commands/input.rs
stutter/src/commands/misc.rs
stutter/src/report/model.rs
```

Deliverable:

```bash
stutter compare display-path \
  --baseline direct-run \
  --test uhd630-run \
  --expect direct-to-offload \
  --strict
```

with a real verdict.

## Phase 8: validation corpus

Files:

```text
tests/
stutter/src/artifact_contract_tests.rs
docs/VALIDATION_CORPUS.md
```

Add synthetic runs:

```text
direct_gpu_clean
uhd630_cross_gpu_fence_wait
uhd630_composited_blitter
uhd630_kms_delay
wayland_zero_copy_good
dmabuf_modifier_mismatch
missing_evidence_unknown
```

Expected outputs should be snapshot-tested.
