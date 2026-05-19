

PROPOSAL 1: Add `stutter-core` as a dependency of `stutter` and migrate `ActionId` to use it
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
Until `stutter-core` is a load-bearing dependency of `stutter`, it provides no enforcement and new implementation code will use the parallel ad-hoc types by default.

CURRENT STATE:
`stutter/Cargo.toml` does not list `stutter-core` in `[dependencies]`. `stutter-core` is a workspace member and compiles correctly. `stutter-core/src/ids.rs` defines `pub struct ActionId(String)` via the `string_id!` macro with `pub fn new(value: impl Into<String>) -> Self`, `pub fn as_str(&self) -> &str`, and derives `Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize` with `#[serde(transparent)]`. The inner field is private.

`stutter/src/actions/model.rs` line 41 defines `pub struct ActionId(pub String)` — identical name, different struct. It derives `Debug, Clone, Serialize, Deserialize, PartialEq, Eq` only. The inner field is `pub`, accessed directly as `.0` in the following locations:
- `stutter/src/actions/mod.rs` line 91: `(\"pub struct\", \"ActionId\")` (architecture test)
- `stutter/src/autotune/candidate.rs`: multiple construction sites as `ActionId(string.to_owned())`
- `stutter/src/actions/fake_action.rs`: construction as `ActionId(\"fake\".to_owned())`
- Various call sites that call `.0.clone()` or `.0.as_str()`

`stutter/src/api.rs` re-exports `crate::actions::ActionId` as part of the public `api::actions` module.

PROPOSED CHANGE:
Step 1: Add `stutter-core = { path = \"../stutter-core\" }` to `[dependencies]` in `stutter/Cargo.toml`.

Step 2: In `stutter/src/actions/model.rs`, remove the `pub struct ActionId(pub String)` definition. Replace it with:
```rust
pub use stutter_core::ids::ActionId;
```

Step 3: Fix all call sites that accessed the inner field directly via `.0`:
- Replace `ActionId(some_string)` tuple construction with `ActionId::new(some_string)`.
- Replace `.0.clone()` with `.as_str().to_owned()` or `.clone()` (since `ActionId` is `Clone`).
- Replace `.0.as_str()` with `.as_str()`.
- Replace `.0 == other` with `action_id.as_str() == other`.

Step 4: Add `Hash` and `Ord` derives where call sites now require them (the `stutter-core` version has both; the old version did not, so any `HashMap<ActionId, _>` or sorted collections that previously could not use `ActionId` as a key will now be able to).

Step 5: Update `stutter/src/api.rs` re-export: change `pub use crate::actions::ActionId` to `pub use stutter_core::ids::ActionId` so the public API re-exports from the canonical source.

AFFECTED SCOPE:
- `stutter/Cargo.toml` (add dependency)
- `stutter/src/actions/model.rs` (remove definition, add re-export)
- `stutter/src/autotune/candidate.rs` (construction sites)
- `stutter/src/actions/fake_action.rs` (construction site)
- `stutter/src/api.rs` (re-export source)
- Any file that calls `.0` on an `ActionId` — scan with `grep -rn \"ActionId.*\.0\|\.0.*ActionId\" stutter/src/`
- `stutter/src/architecture_tests.rs`: the `(\"pub struct\", \"ActionId\")` entry in the public surface list must be removed or updated since the struct no longer originates in `actions/mod.rs`

This is a contained ripple: the type name is unchanged, the `Serialize`/`Deserialize` representation is unchanged (both use `#[serde(transparent)]`), so JSON on-disk format is unaffected.

DEPENDENCIES: None. This proposal is a prerequisite for any future use of `stutter-core` primitives in implementation work.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/Cargo.toml`, add `stutter-core = { path = \"../stutter-core\" }` to the `[dependencies]` section. In `stutter/src/actions/model.rs`, remove the line `pub struct ActionId(pub String);` and its `#[derive(...)]` block entirely. Add `pub use stutter_core::ids::ActionId;` as a replacement. Then scan all files in `stutter/src/` for direct `.0` field access on `ActionId` values (pattern: `action_id.0`, `ActionId(`, `\.0\.clone()` where the receiver is an `ActionId`). For each: replace `ActionId(value)` tuple construction with `ActionId::new(value)`, replace `.0.clone()` with `.clone()` (since `ActionId: Clone`), replace `.0.as_str()` or `.0.as_ref()` with `.as_str()`. In `stutter/src/api.rs`, find the `pub use crate::actions::ActionId;` re-export inside `pub mod actions` and change it to `pub use stutter_core::ids::ActionId;`. In `stutter/src/architecture_tests.rs`, find the entry `(\"pub struct\", \"ActionId\")` in the public surface export list and remove it, since `ActionId` is no longer defined in `actions/mod.rs` but re-exported from `stutter-core`. Run `cargo test -p stutter architecture_tests` to confirm the boundary tests pass.

---

PROPOSAL 2: Move `agent.rs` route handler implementations into their pre-declared sub-modules
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
The 11 `agent/` sub-module files are `pub(crate) use super::X` stubs; no implementation has moved, and every new HTTP route added during implementation will push the 5,092-line file further above its already-at-limit allowlist cap.

CURRENT STATE:
`stutter/src/agent.rs` (5,092 lines) contains all implementation. Its sub-module declarations at lines 111–121 are:
```rust
pub(crate) mod artifacts;
pub(crate) mod auth;
pub(crate) mod autotune;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod rate_limit;
pub(crate) mod recording;
pub(crate) mod routes;
pub(crate) mod server;
pub(crate) mod startup;
pub(crate) mod state;
```

Each sub-module file is a stub re-exporting from `super::`. The route handlers in `agent.rs` group naturally by domain:

**Recording handlers** (lines ~1346–1718): `start_record_handler`, `stop_record_handler`, `status_handler`, `list_runs_handler`, `get_session_handler`, `get_artifact_handler`. These use `AgentState`, `RunHandle`, and interact with the recorder.

**Autotune handlers** (lines ~1719–2290): `autotune_status_handler`, `autotune_start_handler`, `autotune_stop_handler`, `autotune_restore_handler`, `autotune_history_handler`, `autotune_config_handler`. These use `AgentState`, `AutotuneControllerHandle`, and the daemon autotune subsystem.

**Daemon handlers** (lines ~2291–2703): `daemon_status_handler`, `daemon_health_handler`, `daemon_policy_handler`, `daemon_explain_handler`, `daemon_pause_handler`, `daemon_resume_handler`, `daemon_restore_handler`, `daemon_policy_response_handler`. These use `AgentState` and daemon state mutation.

**System handlers** (lines ~2704–end): `version_handler`, `capabilities_handler` and their supporting types.

**Core infrastructure** (lines ~313–1338): `run_agent`, `serve_unix_socket`, `agent_request_guard`, rate limiter, auth guard, `AgentRateLimiter` impl, all response DTOs (`ErrorResponse`, `DaemonStatusResponse`, `DaemonHealthResponse`, `DaemonPolicyResponse`, `DaemonExplainResponse`, `DaemonControlResponse`), helper functions (`replace_agent_daemon_state`, `mark_agent_daemon_fault`, `mark_agent_daemon_policy_rejection`), and the `build_agent_router` call site.

`stutter/src/agent/routes.rs` currently contains a dead stub `build_agent_router` (marked `#[allow(dead_code)]`) that returns an empty `Router::new()`. The real route wiring lives in `agent.rs` lines 443–478.

The architecture test allowlist cap for `agent.rs` is `5_092` — the current size. The cap will be exceeded by the first new route handler added.

PROPOSED CHANGE:
Migrate the four handler groups into their pre-declared sub-module files. Each sub-module becomes a real module, not a re-export facade. `agent.rs` retains only: type definitions (`AgentConfig`, `AgentLimits`, `AgentAuth`, `AgentState`, `AutotuneControllerHandle`, `RunHandle`, `AgentRateLimiter`), constants, `run_agent` entry point, `serve_unix_socket`, `agent_request_guard`, auth guard, and route wiring. This reduces `agent.rs` to approximately 800–1,000 lines.

**`agent/recording.rs`**: move `start_record_handler`, `stop_record_handler`, `status_handler`, `list_runs_handler`, `get_session_handler`, `get_artifact_handler` and their associated response types.

**`agent/autotune.rs`**: move `autotune_status_handler`, `autotune_start_handler`, `autotune_stop_handler`, `autotune_restore_handler`, `autotune_history_handler`, `autotune_config_handler` and their associated request/response types and the `replace_agent_daemon_state`, `mark_agent_daemon_fault`, `mark_agent_daemon_policy_rejection` helpers (these are only called from autotune handlers).

**`agent/daemon.rs`**: move `daemon_status_handler`, `daemon_health_handler`, `daemon_policy_handler`, `daemon_explain_handler`, `daemon_pause_handler`, `daemon_resume_handler`, `daemon_restore_handler`, `daemon_policy_response_handler` and their associated response types (`DaemonStatusResponse`, `DaemonHealthResponse`, `DaemonPolicyResponse`, `DaemonExplainResponse`, `DaemonControlResponse`).

**`agent/routes.rs`**: replace the `#[allow(dead_code)]` stub `build_agent_router` with the real router construction, moving the `Router::new().route(...)` chain from `agent.rs` lines 443–478 into `pub(crate) fn build_agent_router(state: Arc<AgentState>, rate_limiter: Arc<AgentRateLimiter>) -> Router`.

**`agent/server.rs`**: move `serve_unix_socket` and `agent_request_guard` into this module. Keep `run_agent` in `agent.rs` or move it here — either is acceptable; if moved, `agent.rs` re-exports it.

All moved functions access `AgentState`, `AgentRateLimiter`, and other types via `use super::{AgentState, ...}` since they remain in the same crate. No public API changes.

After migration, update the architecture test allowlist: lower `agent.rs` max from `5_092` to approximately `1_100` (types + constants + entry point only). Add new entries for each sub-module file if they exceed 1,000 lines.

Remove `#![allow(unused_imports)]` and `#[allow(dead_code)]` from all `agent/` sub-module files — these were justified only by the stub state.

AFFECTED SCOPE:
- `stutter/src/agent.rs` (shrinks significantly; retains types and entry point)
- `stutter/src/agent/recording.rs` (receives recording handlers)
- `stutter/src/agent/autotune.rs` (receives autotune handlers)
- `stutter/src/agent/daemon.rs` (receives daemon handlers)
- `stutter/src/agent/routes.rs` (receives real `build_agent_router`)
- `stutter/src/agent/server.rs` (receives `serve_unix_socket`, `agent_request_guard`)
- `stutter/src/architecture_tests.rs` (update `agent.rs` allowlist cap; remove `agent.rs` from unwrap allowance if unwraps move with handlers)
- No external callers change — `run_agent` remains public, the public API surface in `api.rs` is unchanged

DEPENDENCIES: Proposal 1 must be completed first only if any handler constructs `ActionId` directly. In practice, the handlers use `ActionId` only through `AgentState` which is passed by reference — the migration can proceed in parallel with Proposal 1. No ordering dependency.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/agent.rs`, identify the four handler groups by their `async fn` names as listed above. For each group, move the `async fn` implementations plus any response-only structs (`DaemonStatusResponse`, etc.) that are used exclusively by that group into the corresponding `stutter/src/agent/{recording,autotune,daemon}.rs` file. Replace the moved code in `agent.rs` with nothing (the sub-module declaration already exists). In each receiving file, remove the existing `#![allow(unused_imports)]` header and the `pub(crate) use super::X` re-export lines. Add `use super::{AgentState, AgentRateLimiter, RunHandle, AutotuneControllerHandle};` and any other types needed from `agent.rs`. The moved functions are `pub(crate)` or `pub(super)` — match the visibility of the handler functions as they were. In `stutter/src/agent/routes.rs`, remove the `#[allow(dead_code)]` stub and replace `build_agent_router` body with the `Router::new().route(...)` chain currently at lines 443–478 of `agent.rs`, importing handler functions from `super::recording`, `super::autotune`, `super::daemon`, and defining inline any system-level handlers (`version_handler`, `capabilities_handler`) if they are short enough to stay in `routes.rs`. In `stutter/src/agent/server.rs`, move `serve_unix_socket` and `agent_request_guard` from `agent.rs`, removing the `pub(crate) use super::...` re-export. In `stutter/src/agent.rs`, update `run_agent` to call `agent::routes::build_agent_router(state, rate_limiter)` instead of the inline `Router::new()` chain. In `stutter/src/architecture_tests.rs`, lower the `agent.rs` `max_lines` allowance to reflect its reduced size after migration. Run `cargo test -p stutter architecture_tests` and `cargo test -p stutter agent` to verify.

---

PROPOSAL 3: Split `autotune/candidate.rs` into four focused sub-modules
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
The file is at its allowlist cap of 3,844 lines and contains four independently consumed responsibilities; any new action type added during implementation immediately requires a forced split under feature pressure.

CURRENT STATE:
`stutter/src/autotune/candidate.rs` (3,844 lines) is a single file. The architecture test cap is `3_844` — exactly the current size. The file contains four separable sections:

**Section 1 — Core type model** (approximately lines 1–290): `CandidateAction` enum with 10 variants, `CandidateFamily` type alias, `ExecutablePlan` type alias, `CandidatePlan` trait, `CandidateEvidence` struct, `SuggestionCandidate`, `ApplyCandidate`, `ApplyEligibility`, `try_promote_to_apply_candidate`.

**Section 2 — Action plan structs and their `CandidatePlan` impls** (approximately lines 291–950): All 13 action plan structs (`NiceActionPlan`, `IoPrioActionPlan`, `UclampActionPlan`, `CgroupPlacementActionPlan`, `IrqAffinityActionPlan`, `CpuPowerActionPlan`, `GpuPowerActionPlan`, `VmKnobActionPlan`, `FakeCandidatePlan`, `CpuAffinityProfilePlan`, `GeneratedProfileCandidatePlan`, `GeneratedCpuSetPolicy`, `GeneratedTopologyProfilePlan`) and all `impl CandidatePlan for ...` blocks including the `macro_rules! impl_system_candidate_plan!` macro and its four invocations.

**Section 3 — Safety classification and eligibility** (approximately lines 951–1,100): `CandidateAction::is_high_risk_system_adjacent()`, `CandidateAction::manual_only_reason()`, `CandidateAction::safety_class()`, `CandidateAction::conflicts_with()`, `CandidateAction::action_kind()`, `CandidateAction::descriptor()`. These are the functions the roadmap proposals (Proposals 2–4 from the production roadmap document) must modify to unlock system-wide actions.

**Section 4 — Output, suggestions, and I/O** (approximately lines 1,101–3,700): `CandidatePlanFile`, `CandidatePlanSummary`, `CandidateExecutablePlan`, `CandidateDryRunRecord`, `CandidateSuggestion`, `CandidateManualCommands`, `write_candidate_plan_file`, `apply_candidate_plan_file`, `default_candidate_plan_dir`, `candidate_plan_path`, `suggestion_from_candidate_dry_run_record`, `suggestions_from_candidates_and_dry_run_records`, `render_candidate_suggestion`, `render_candidate_suggestions`, `print_candidate_suggestions`, `required_mode_for_safety_class`, helper functions.

The existing `autotune/planning/` sub-namespace exists with: `planning/candidate.rs`, `planning/collect.rs`, `planning/denial.rs`, `planning/dry_run.rs`, `planning/executable_plan.rs`, `planning/gates.rs`, `planning/mod.rs`, `planning/plan_io.rs`, `planning/profile_candidates.rs`, `planning/ranking.rs`, `planning/suggestion.rs`. These are currently re-export facades mirroring the `candidate.rs` content, same pattern as the `agent/` sub-modules.

PROPOSED CHANGE:
Migrate each section into its corresponding `planning/` sub-module. `candidate.rs` becomes a thin re-export facade. Specifically:

**`autotune/planning/candidate.rs`**: receives Section 1 — `CandidateAction`, `CandidateFamily`, `ExecutablePlan`, `CandidatePlan` trait, `CandidateEvidence`, `SuggestionCandidate`, `ApplyCandidate`, `ApplyEligibility`, `try_promote_to_apply_candidate`. This is the file consumers import most.

**`autotune/planning/executable_plan.rs`**: receives Section 2 — all action plan structs and their `impl CandidatePlan` blocks. The `macro_rules! impl_system_candidate_plan!` stays in this file. Each new action type added during implementation adds only to this file.

**`autotune/planning/gates.rs`**: receives Section 3 — all safety classification methods. This is where Proposals 2, 3, and 4 from the production roadmap will make their changes (changing `is_high_risk_system_adjacent()` to be parameter-conditional rather than variant-membership-based). Isolating this section means the patch writer for each roadmap proposal touches only `gates.rs`, not a 3,844-line file.

**`autotune/planning/suggestion.rs`**: receives the suggestion structs and rendering functions from Section 4 (`CandidateSuggestion`, `CandidateManualCommands`, `suggestion_from_candidate_dry_run_record`, `render_candidate_suggestion`, etc.).

**`autotune/planning/plan_io.rs`**: receives the file I/O functions from Section 4 (`CandidatePlanFile`, `CandidatePlanSummary`, `write_candidate_plan_file`, `apply_candidate_plan_file`, `default_candidate_plan_dir`, `candidate_plan_path`).

**`autotune/candidate.rs`**: becomes a re-export module:
```rust
pub use crate::autotune::planning::candidate::*;
pub use crate::autotune::planning::executable_plan::*;
pub use crate::autotune::planning::gates::*;
pub use crate::autotune::planning::suggestion::*;
pub use crate::autotune::planning::plan_io::*;
```
This preserves all existing import paths (`use crate::autotune::candidate::CandidateAction`) without requiring changes to the 50+ files that import from `autotune::candidate`.

After migration, update the architecture test: lower `candidate.rs` cap to ~100 lines (re-exports only), add caps for each `planning/` sub-module file (each expected to be under 1,000 lines).

Remove `#![allow(unused_imports)]` stubs from all `planning/` sub-module files that currently re-export from `super`.

AFFECTED SCOPE:
- `stutter/src/autotune/candidate.rs` (shrinks to re-export facade, ~100 lines)
- `stutter/src/autotune/planning/candidate.rs` (receives Section 1, ~290 lines)
- `stutter/src/autotune/planning/executable_plan.rs` (receives Section 2, ~660 lines)
- `stutter/src/autotune/planning/gates.rs` (receives Section 3, ~150 lines)
- `stutter/src/autotune/planning/suggestion.rs` (receives suggestion structs and rendering, ~400 lines)
- `stutter/src/autotune/planning/plan_io.rs` (receives file I/O structs and functions, ~300 lines)
- `stutter/src/autotune/planning/mod.rs` (must re-export from sub-modules)
- `stutter/src/architecture_tests.rs` (update `candidate.rs` allowlist cap; add new entries for planning sub-modules)
- No other files change import paths if `candidate.rs` re-exports everything it currently exports.

DEPENDENCIES: None. Can be done before or after Proposal 1 and Proposal 2 independently. Must be completed before any roadmap implementation proposals that modify `is_high_risk_system_adjacent()` or add new `CandidatePlan` impls — those are the proposals that would force a mid-feature split otherwise.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/candidate.rs`, identify the four sections by the groupings described above. Move Section 1 (core types: `CandidateAction` enum, `CandidatePlan` trait, `CandidateEvidence`, `SuggestionCandidate`, `ApplyCandidate`, `ApplyEligibility`, `try_promote_to_apply_candidate`) into `stutter/src/autotune/planning/candidate.rs`, replacing its existing `pub(crate) use super::...` re-export lines. Move Section 2 (all action plan structs and all `impl CandidatePlan` blocks including the `impl_system_candidate_plan!` macro definition and its invocations) into `stutter/src/autotune/planning/executable_plan.rs`, replacing its existing content. Move Section 3 (all methods on `CandidateAction` related to safety: `is_high_risk_system_adjacent`, `manual_only_reason`, `safety_class`, `conflicts_with`, `action_kind`, `descriptor`) into `stutter/src/autotune/planning/gates.rs`. Note: these are `impl CandidateAction` blocks — they must be `impl CandidateAction` in the new file with `use super::candidate::CandidateAction` to access the type. Move Section 4a (suggestion output: `CandidateSuggestion`, `CandidateManualCommands`, `CandidateDryRunRecord`, `suggestion_from_candidate_dry_run_record`, `suggestions_from_candidates_and_dry_run_records`, `render_candidate_suggestion`, `render_candidate_suggestions`, `print_candidate_suggestions`) into `stutter/src/autotune/planning/suggestion.rs`. Move Section 4b (plan I/O: `CandidatePlanFile`, `CandidatePlanSummary`, `CandidateExecutablePlan`, `write_candidate_plan_file`, `apply_candidate_plan_file`, `default_candidate_plan_dir`, `candidate_plan_path`, `sanitize_candidate_plan_component`) into `stutter/src/autotune/planning/plan_io.rs`. Replace the body of `stutter/src/autotune/candidate.rs` with `pub use crate::autotune::planning::{candidate::*, executable_plan::*, gates::*, suggestion::*, plan_io::*};`. Update `stutter/src/autotune/planning/mod.rs` to declare each sub-module as `pub mod`. In `stutter/src/architecture_tests.rs`, lower `autotune/candidate.rs` max_lines to 50 and add new allowlist entries for any planning sub-modules that exceed 1,000 lines. Run `cargo test -p stutter` to verify no import paths are broken.







PROPOSAL 21: Decompose `focus/mod.rs` and remove legacy focus coupling

PRIORITY: MEDIUM
Focus classification is core to the watcher, but the focus module still has a large `mod.rs` hub beside newer focused submodules.

CURRENT STATE:
The focus directory contains `classify.rs`, `groups.rs`, `resolve.rs`, `score.rs`, `snapshot.rs`, and a large `mod.rs` file of about 4k lines in the uploaded tree.
`FocusResolver` and `FocusPolicy` live in `focus/resolve.rs`; focus groups and safety warnings live in `focus/groups.rs`; snapshot construction lives in `focus/snapshot.rs`.
The large `focus/mod.rs` indicates remaining mixed responsibilities and creates risk that callers bypass the newer boundaries.

PROPOSED CHANGE:
Move remaining contents of `focus/mod.rs` into explicit modules:

* `focus/provider.rs`
* `focus/foreground_match.rs`
* `focus/process_scan.rs`
* `focus/public_api.rs`
* `focus/tests.rs`

`focus/mod.rs` must only declare modules and re-export the public API.
All callers must import from the re-exported API or specific modules consistently.
No focus classification/scoring logic may remain directly in `mod.rs`.

AFFECTED SCOPE:

* `stutter/src/focus/mod.rs`
* new `stutter/src/focus/*.rs`
* imports in `stutter/src/autotune/runtime.rs`
* imports in `stutter/src/session_events.rs`
* imports in `stutter/src/tui.rs`
* imports in report/recorder modules that use focus types
* focus tests
  Medium mechanical + boundary refactor.

DEPENDENCIES:

* Can be implemented independently.
* Should happen before deeper focus provider improvements.

EDIT REQUEST FOR PATCH WRITER:
Refactor `focus/mod.rs` into small responsibility-specific modules. Leave `mod.rs` as a re-export layer only. Move process scanning, foreground matching, provider selection, and test helpers into named files. Update imports across the repo to avoid bypassing the intended focus boundaries.

---








PROPOSAL 22: Add privilege boundary for apply-capable actions

PRIORITY: HIGH
A full watcher that changes system state must separate unprivileged observation from privileged mutation.

CURRENT STATE:
The repo has `daemon/privilege.rs`, `agent.rs`, and action runner infrastructure.
The action runner performs preflight, dry-run, apply, verify, rollback, audit, timeouts, and policy checks.
Medium/high-risk action families include privileged operations such as cgroup movement, IRQ affinity, sysfs CPU/GPU power writes, and VM sysctl writes.
Current live autotune path still runs inside one process model.

PROPOSED CHANGE:
Define a strict privileged worker interface:

```rust
pub trait PrivilegedActionService {
    fn dry_run_candidate(&self, request: CandidatePlanRequest) -> Result<ActionState>;
    fn apply_candidate(&self, request: CandidateApplyRequest) -> Result<ApplyResult>;
    fn rollback(&self, request: RollbackRequest) -> Result<RollbackResult>;
}
```

The unprivileged daemon may:

* observe
* classify
* plan
* suggest
* request dry-run/apply through privileged service

The privileged worker must:

* re-check policy independently
* reject stale candidate plan timestamps
* reject candidate plans without descriptor/evidence/objective
* write audit events
* return rollback tokens

AFFECTED SCOPE:

* `stutter/src/daemon/privilege.rs`
* `stutter/src/agent.rs`
* `stutter/src/actions/runner.rs`
* `stutter/src/autotune/apply.rs`
* `stutter/src/autotune/runtime.rs`
* `stutter/src/daemon/runtime.rs`
* CLI/service docs
  Large architectural change.

DEPENDENCIES:

* Should follow PROPOSAL 8 and PROPOSAL 10.
* Required before high-risk apply is ever implemented.

EDIT REQUEST FOR PATCH WRITER:
Introduce a privileged action service boundary between the planner/runtime and action mutation. The planner must produce candidate plan requests; the privileged service must independently re-check policy and execute dry-run/apply/rollback through `actions::runner`. Update live runtime to use this service abstraction even when running in-process for tests.

---






PROPOSAL 23: Add full-system soak tests for autonomous watcher behavior

PRIORITY: HIGH
The watcher must prove it does not flap, stack conflicting actions, or fail to restore under long-running workload changes.

CURRENT STATE:
`daemon/soak.rs`, `daemon/acceptance.rs`, and `autotune/simulation.rs` exist.
The current test architecture has fake daemon scenarios and acceptance checks, but the new provider/planner architecture needs soak cases covering multi-workload transitions and candidate conflicts.

PROPOSED CHANGE:
Add soak profiles:

* game → browser → game
* compile in background while browser foreground
* recording + game
* media playback + compile
* VM load + desktop interaction
* low data quality burst
* thermal degradation during active experiment
* target disappears during active experiment
* external mutation during kept action
* repeated same candidate cooldown

Soak assertions:

* no apply without rollback token
* one active experiment maximum
* no conflicting kept actions
* no apply during low data quality
* no apply to protected tasks
* no high-risk autonomous apply
* all active changes restored on shutdown
* cooldown respected
* no focus flapping below configured confidence/margins

AFFECTED SCOPE:

* `stutter/src/daemon/soak.rs`
* `stutter/src/daemon/acceptance.rs`
* `stutter/src/autotune/simulation.rs`
* `stutter/src/autotune/replay.rs`
* `testdata/autotune/soak/*.json`
* CI workflow if soak tests are split into slow tests
  Medium-large test expansion.

DEPENDENCIES:

* Should follow PROPOSAL 13.
* Should run before enabling medium-risk apply by default.

EDIT REQUEST FOR PATCH WRITER:
Extend daemon/autotune simulation and soak testing with multi-workload scenarios that exercise planner, controller, rollback, cooldown, focus changes, data quality failures, protected tasks, and high-risk denial. Add assertions that no unsafe autonomous mutation occurs and that shutdown/startup recovery restores all active actions.

---






PROPOSAL 24: Persist and validate workload memory by stable workload identity

PRIORITY: MEDIUM
The full watcher must remember which actions helped a workload without confusing different programs or changed binaries.

CURRENT STATE:
`AutotuneObservation` contains `WorkloadIdentity` with root PID, starttime, executable device/inode, cgroup path, focus kind, class distribution, and stable hash.
`candidate_memory` and history mechanisms exist, and runtime appends history.
Planner cooldown currently uses action ID and current time, not a richer workload/action/outcome table.

PROPOSED CHANGE:
Create `WorkloadActionMemory`:

```rust
pub struct WorkloadActionMemory {
    pub workload_hash: String,
    pub action_id: ActionId,
    pub action_kind: String,
    pub objective: ObjectiveKind,
    pub last_result: CandidateMemoryResult,
    pub score_delta: Option<f64>,
    pub last_seen_unix_nanos: u128,
    pub cooldown_until_unix_nanos: Option<u128>,
}
```

Behavior:

* Successful actions become preferred for same workload hash and same situation.
* Regressed actions receive longer cooldown for same workload.
* Inconclusive actions receive short cooldown.
* Binary identity changes invalidate stale memory.
* Memory must be bounded and persisted.

AFFECTED SCOPE:

* `stutter/src/autotune/candidate_memory.rs`
* `stutter/src/autotune/history.rs`
* `stutter/src/autotune/runtime.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/observation.rs`
* `stutter/src/daemon/state.rs`
* tests/fixtures
  Medium feature addition.

DEPENDENCIES:

* Should follow PROPOSAL 5 and PROPOSAL 13.
* Helps full watcher milestone after safe local autotune is stable.

EDIT REQUEST FOR PATCH WRITER:
Implement persisted workload/action memory keyed by `WorkloadIdentity.stable_hash`, action ID, objective, and situation. Use it in planner ranking and cooldown decisions. Add invalidation when executable identity or cgroup identity changes. Add tests proving regressed actions are deprioritized for the same workload and not incorrectly blocked for unrelated workloads.

---






PROPOSAL 25: Make high-risk apply impossible until explicit future unlock

PRIORITY: CRITICAL
High-risk apply must stay impossible at the type/policy level until all system-wide verification, privilege separation, and rollback requirements are implemented.

CURRENT STATE:
`DaemonMode` includes `ApplyHighRisk`. 
`CandidateAction` includes high-risk/system-adjacent variants. 
`executor_for_candidate()` does not support IRQ, CPU power, GPU power, or VM knob apply. 
Policy includes `allow_high_risk` and `allow_system_wide_actions` fields, but the system-wide model is not split yet.

PROPOSED CHANGE:
Add a compile-time/runtime explicit guard:

```rust
pub enum HighRiskApplySupport {
    Disabled,
}
```

or equivalent constant:

```rust
pub const HIGH_RISK_APPLY_IMPLEMENTED: bool = false;
```

All code paths must reject `ApplyHighRisk` apply attempts with a stable reason code:

* `high_risk_apply_not_implemented`

High-risk candidates may be suggested manually after PROPOSAL 4 and PROPOSAL 11, but:

* no executor path may apply them
* no CLI mode may start them
* no remote agent may apply them
* no config may override this until the constant/type is changed in a dedicated future PR

AFFECTED SCOPE:

* `stutter/src/daemon/policy.rs`
* `stutter/src/cli.rs`
* `stutter/src/autotune/apply.rs`
* `stutter/src/autotune/runtime.rs`
* `stutter/src/agent.rs`
* `stutter/src/daemon/privilege.rs`
* tests
  Medium safety hardening.

DEPENDENCIES:

* Should be implemented with PROPOSAL 4 and PROPOSAL 11.
* Must remain until after PROPOSAL 22 and objective verification are complete.

EDIT REQUEST FOR PATCH WRITER:
Add an explicit high-risk apply disabled guard that no policy/config/CLI/remote path can override. Keep high-risk suggestions available only through manual-only output, but make autonomous and direct high-risk apply return a stable rejection reason. Add tests for CLI, policy, planner, agent, and apply executor paths.
















PROPOSAL 1: Replace `.unwrap()` in sysfs-reading production paths with `?`-propagated `anyhow::Result`
PRIORITY: CRITICAL
STATUS: Completed 2026-05-19.
Justifies: a panic in `emergency_restore`, `startup_recovery`, or `system_context` during an already-degraded system state terminates the daemon without completing rollback.

CURRENT STATE:
`autotune/emergency_restore.rs` line ~1003: `write_controller_journal_clean(input.journal_path.as_deref().unwrap()).unwrap()` — called unconditionally before `restore_known_autotune_actions`. If the journal path is absent or the write fails, the daemon panics before restoring any actions.

`autotune/system_context.rs`: 19 `.unwrap()` calls on sysfs string parse operations (CPU frequency reads, IRQ affinity reads, GPU power reads). Each is a potential panic if sysfs returns unexpected content, which happens on kernel upgrades, driver changes, or when a device is removed mid-read.

`autotune/active_config.rs`: 32 `.unwrap()` calls; many are on `serde_json::to_value(&snapshot).unwrap()` in production snapshot paths (lines that are NOT inside `#[cfg(test)]`). JSON serialization of a known-valid struct is actually infallible here, but the call site does not document this invariant.

`autotune/startup_recovery.rs`: 47 `.unwrap()` calls; the function `check_and_recover_on_startup` calls journal reading and action restoration — both are I/O operations that can fail legitimately.

PROPOSED CHANGE:
In `emergency_restore.rs`, replace `.unwrap()` on `journal_path.as_deref()` with an explicit `let Some(path) = input.journal_path.as_deref() else { return Ok(default_clean_outcome) }`. Replace `.unwrap()` on `write_controller_journal_clean` with `?` propagation. Function signature must return `anyhow::Result<AutotuneRestoreOutcome>` (it already does, so `?` is valid throughout).

In `system_context.rs`, replace all `.unwrap()` on parse results with `.unwrap_or_default()` for metrics that have safe zero/empty fallbacks, and `?` with `.context("reading /sys/...")` for reads that must succeed.

In `active_config.rs`, replace `.unwrap()` on `serde_json::to_value` with `.expect("WindowSnapshot is always serializable")` where the invariant is documented, and with `anyhow::bail!` for the two production snapshot paths.

In `startup_recovery.rs`, replace all I/O `.unwrap()` with `?` and surface startup recovery failures as logged warnings that degrade gracefully rather than panicking.

AFFECTED SCOPE:
- `stutter/src/autotune/emergency_restore.rs`
- `stutter/src/autotune/startup_recovery.rs`
- `stutter/src/autotune/system_context.rs`
- `stutter/src/autotune/active_config.rs`

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/emergency_restore.rs`, find every `.unwrap()` call that is not inside a `#[cfg(test)]` block. For each: if it is on an `Option` that can legitimately be `None` at runtime, replace with a guarded early return or `.unwrap_or`. If it is on an `anyhow::Result` or `std::io::Result`, replace with `?` and add a `.context("description")` qualifier. The function `restore_known_autotune_actions` must not panic on any I/O failure. Apply the same transformation to `stutter/src/autotune/startup_recovery.rs`. In `stutter/src/autotune/system_context.rs`, replace all `.unwrap()` on sysfs parse results with `.unwrap_or_default()` or `.unwrap_or(0)` as appropriate for the field type. Document each with a brief comment explaining the fallback semantics. Do not change test code.

---










PROPOSAL 2: Remove module-level `#![allow(dead_code)]` from `actions/mod.rs`, `community_rules.rs`, and `foreground.rs` by wiring or explicitly removing unused symbols
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
Justifies: these suppressors hide divergence between built API surface and actual usage, making it impossible to detect future dead code accumulation via compiler feedback.

CURRENT STATE:
`stutter/src/actions/mod.rs` has `#![allow(dead_code)]` at line 1. The suppressor hides that `RollbackRegistry`, `RollbackHandler`, `RollbackCandidate`, `discover_all()`, `preview_all()`, and `restore_all()` on the registry are never called from autotune or daemon code. The autotune rollback path goes through `emergency_restore.rs` which uses raw `libc` syscalls and action-specific match arms — not the `RollbackRegistry` abstraction.

`stutter/src/community_rules.rs` has `#![allow(dead_code)]` at line 1. `import.rs::RuleImportResult`, `importer.rs::ImportedRule`, `importer.rs::RuleImportContext`, `paths.rs::community_rules_dir` are all unused outside of tests.

`stutter/src/foreground.rs` has `#![allow(dead_code)]` at line 1. `ForegroundWindowSnapshot`, `ForegroundSource`, `ForegroundProviderStatus`, and `ForegroundEvent` are actively used in `focus/mod.rs`, `recorder/mod.rs`, and `session/ui.rs`. The suppressor is a stale artifact.

PROPOSED CHANGE:
**`foreground.rs`**: remove the `#![allow(dead_code)]` suppressor entirely. The module is genuinely used. Run `cargo check` to verify no warnings emerge.

**`community_rules.rs`**: remove the module-level suppressor. Add targeted `#[allow(dead_code)]` only to the specific items in `import.rs` and `importer.rs` that are intentionally forward-declared but not yet wired (`RuleImportResult`, `ImportedRule`, `RuleImportContext`). Each targeted suppressor must carry a `// TODO: wire into community_rules import pipeline` comment.

**`actions/mod.rs`**: remove the module-level suppressor. Evaluate `RollbackRegistry` and its trait: if `RollbackRegistry` is intended to eventually replace the match-arm dispatch in `emergency_restore.rs`, add a `// TODO: replace emergency_restore direct syscall dispatch with RollbackRegistry` comment and add targeted `#[allow(dead_code)]` to the registry API items. If it is not intended to be used, delete it. Do not suppress the whole module.

AFFECTED SCOPE:
- `stutter/src/actions/mod.rs`
- `stutter/src/community_rules.rs`
- `stutter/src/community_rules/import.rs`
- `stutter/src/community_rules/importer.rs`
- `stutter/src/foreground.rs`

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
Remove `#![allow(dead_code)]` from the file headers of `stutter/src/actions/mod.rs`, `stutter/src/community_rules.rs`, and `stutter/src/foreground.rs`. Run `cargo check` (mentally). For any item that becomes a dead_code warning after removal: in `foreground.rs`, there should be none — if there are, investigate. In `community_rules.rs`, add `#[allow(dead_code)] // TODO: wire into import pipeline` to the specific structs in `import.rs` and `importer.rs`. In `actions/mod.rs`, add `#[allow(dead_code)] // TODO: replace emergency_restore direct dispatch` to `RollbackRegistry`, `RollbackHandler`, `RollbackCandidate`, and their methods `discover_all`, `preview_all`, `restore_all`. Do not suppress any other items.

---










PROPOSAL 3: Wire `tasks.rs` dead abstractions or delete them; remove the 9 field-level suppressors
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
Justifies: `TargetRefreshPlan`, `TargetMapApplier`, and `TreeEventBuilder` are the designed API for reactive task-tree updates but are completely bypassed in the daemon tick, creating invisible architectural debt.

CURRENT STATE:
`stutter/src/tasks.rs` has `#[allow(dead_code)]` on: `TargetRefreshPlan` (line 26), `TargetRefreshValidation` (line 35), `TaskReplacement` (line 43), `TargetMapOperation` (line 51), `TargetRefreshOutcome` (line 58), `TargetMapApplier` (line 65), `TargetMapApplier::apply` (line 69), `TreeEventBuilder` (line 84), `TreeEventBuilder::events_for_plan` (line 88). The daemon tick calls `TaskTracker::handle_replacements` directly, bypassing `TargetMapApplier`. `TreeEventBuilder::events_for_plan` produces `TreeEvent` values that carry diff information the session event bus could consume, but does not.

PROPOSED CHANGE:
Decide: the `TargetMapApplier`/`TreeEventBuilder` abstraction either represents a planned migration away from the current direct `handle_replacements` call, or it is dead speculative design.

**Option A (wire):** In `autotune/runtime.rs`, replace the direct call to `task_tracker.handle_replacements(...)` with `TargetMapApplier::apply(plan, ...)`. Implement the missing body of `TargetMapApplier::apply` to call `handle_replacements` internally. Have it return `TargetRefreshOutcome`. Remove all `#[allow(dead_code)]` suppressors from `tasks.rs`.

**Option B (delete):** Remove `TargetRefreshPlan`, `TargetRefreshValidation`, `TaskReplacement`, `TargetMapOperation`, `TargetRefreshOutcome`, `TargetMapApplier`, and `TreeEventBuilder` from `tasks.rs`. Remove all their `#[allow(dead_code)]` suppressors. Keep `TaskTracker` and its methods untouched.

The patch writer must choose Option A if the `TreeEventBuilder::events_for_plan` diff events are needed for a future session event bus integration. Choose Option B if they are not.

AFFECTED SCOPE:
- `stutter/src/tasks.rs`
- `stutter/src/autotune/runtime.rs` (if Option A)

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/tasks.rs`, remove all 9 `#[allow(dead_code)]` attributes from `TargetRefreshPlan`, `TargetRefreshValidation`, `TaskReplacement`, `TargetMapOperation`, `TargetRefreshOutcome`, `TargetMapApplier`, `TargetMapApplier::apply`, `TreeEventBuilder`, and `TreeEventBuilder::events_for_plan`. Then choose: either implement `TargetMapApplier::apply` (currently body-less stub) as a thin wrapper over `TaskTracker::handle_replacements` and update `stutter/src/autotune/runtime.rs` to use it, or delete all those types and their `impl` blocks entirely. Document the choice in a `// ARCH:` comment at the top of `tasks.rs`.

---











PROPOSAL 4: Replace hardcoded `ScoreComparisonConfig` constants with variance-weighted thresholds
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
Justifies: the 12.5% minimum improvement threshold is invariant across baseline variance, sample count, window duration, and workload type; it misclassifies real improvements as `Inconclusive` when baselines are noisy, and it misclassifies lucky sample-timing as `Improved` when baselines are stable.

CURRENT STATE:
`stutter/src/autotune/comparison.rs` declares:
```rust
pub(crate) const DEFAULT_SCORE_COMPARISON_CONFIG: ScoreComparisonConfig = ScoreComparisonConfig {
    min_improvement_percent: 12.5,
    max_regression_percent: 7.5,
    max_frame_p99_regression_ms: 2.0,
    max_over_5ms_regression: 0,
};
```
`compare_scores_with_config` receives `data_quality: ExperimentDataQuality` (High/Medium/Low). Low quality → immediate `Regressed`. High/Medium → same fixed threshold applied regardless of how many samples were collected, how long the window ran, or how variable the baseline was. `WindowScore` carries `interval_count` and `scored_samples` which could inform threshold tightening or loosening, but are not used by the comparison function.

`autotune/quality.rs` has `DEFAULT_MIN_SCORED_INTERVALS: usize = 5` and `DEFAULT_MIN_SCORED_SAMPLES: u64 = 100`. These guard entry into High/Medium quality. A window with exactly 100 samples gets the same threshold as one with 10,000 samples.

PROPOSED CHANGE:
Add a `ThresholdPolicy` struct to `autotune/comparison.rs` with three threshold tiers keyed on sample count:

```rust
pub struct ThresholdTier {
    pub min_scored_samples: u64,
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
}

pub struct ThresholdPolicy {
    pub tiers: Vec<ThresholdTier>,  // sorted ascending by min_scored_samples
}
```

Default tiers (empirically conservative starting points that can be tuned without changing the struct):
- `< 200 samples`: min_improvement=15.0%, max_regression=5.0% (tighter, fewer samples)
- `200–999 samples`: min_improvement=12.5%, max_regression=7.5% (current behaviour preserved)
- `≥ 1000 samples`: min_improvement=10.0%, max_regression=8.5% (more evidence → accept smaller signals)

Modify `compare_scores_with_config` to accept an optional `ThresholdPolicy`. When provided, select the `ScoreComparisonConfig` based on `input.baseline.scored_samples`. When absent, use the existing default. This is additive — all existing callers continue to work with `None`.

In `autotune/objective.rs`, fix all `ExperimentResult::Regressed { regression_percent: 0.0 }` returns in the objective-level veto functions to instead carry the actual regression magnitude computed from the window scores:

```rust
// Replace:
return Some(ExperimentResult::Regressed { regression_percent: 0.0 });
// With (where baseline and candidate are in scope):
return Some(ExperimentResult::Regressed {
    regression_percent: regression_percent(baseline.score.total, candidate.score.total),
});
```

`regression_percent` is already a free function in `comparison.rs`; make it `pub(crate)` or move it to a shared location accessible from `objective.rs`.

AFFECTED SCOPE:
- `stutter/src/autotune/comparison.rs` (add `ThresholdTier`, `ThresholdPolicy`; modify `compare_scores_with_config` signature)
- `stutter/src/autotune/objective.rs` (fix zero regression_percent in all veto returns)
- `stutter/src/autotune/live_experiment.rs` (pass `ThresholdPolicy` through `compare_keep_result` if it calls `compare_for_objective`)
- Callers in `autotune/runtime.rs` and `autotune/comparison.rs` tests must be updated to pass `None` for backward compat

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/comparison.rs`, add two new public structs: `ThresholdTier { pub min_scored_samples: u64, pub min_improvement_percent: f64, pub max_regression_percent: f64 }` and `ThresholdPolicy { pub tiers: Vec<ThresholdTier> }`. Add `impl ThresholdPolicy { pub fn default_tiers() -> Self { ... } pub fn config_for_samples(&self, scored_samples: u64) -> ScoreComparisonConfig { ... } }`. The default tiers must be: below 200 samples → min=15.0 max=5.0, 200-999 → min=12.5 max=7.5, 1000+ → min=10.0 max=8.5. Modify `compare_scores_with_config` to accept an additional parameter `threshold_policy: Option<&ThresholdPolicy>`; when `Some`, call `policy.config_for_samples(input.baseline.scored_samples)` to select the config and ignore the passed `config` parameter. All existing call sites must pass `None` so the existing behaviour is preserved by default. Then, in `stutter/src/autotune/objective.rs`, make the `regression_percent` free function from `comparison.rs` visible (add `pub(crate)` to it), and replace every `ExperimentResult::Regressed { regression_percent: 0.0 }` in `objective.rs` with `ExperimentResult::Regressed { regression_percent: crate::autotune::comparison::regression_percent(baseline.score.total, candidate.score.total) }`, using whatever baseline/candidate are in scope at that point in the call stack.

---











PROPOSAL 5: Add `AutotuneObservationBuilder` unit tests and planner-integration golden cases
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
Justifies: `AutotuneObservationBuilder` is the only untested bridge between raw rolling window data and planner decisions; a bug here silently passes wrong observations to every provider.

CURRENT STATE:
`stutter/src/autotune/observation_builder.rs` (436 lines) transforms `AutotuneObservationBuilderInput` (which contains a reference to `RollingWindow` state, task info, system health, etc.) into `AutotuneObservation`. It is called from `autotune/runtime.rs` on every tick. It has zero tests. There are no golden cases that verify "given this rolling window state and task tree, the observation has these properties."

`stutter/src/autotune/planner.rs` has 20 golden cases, but all of them use `build_fixture_observation` which constructs observations manually. The path from `RollingWindow::objective_signals()` → `ObservationBuilder` → `AutotuneObservation` → planner is not tested end-to-end.

PROPOSED CHANGE:
Add a `#[cfg(test)] mod tests` block to `autotune/observation_builder.rs` with at minimum:

1. **`observation_from_game_window_with_high_cpu_latency`**: construct a `RollingWindow` with synthetic `IntervalRecord` values representing >5ms runnable latency on game thread TIDs, verify that the resulting `AutotuneObservation` has `focus_confidence >= DEFAULT_MIN_FOCUS_CONFIDENCE` and `situation == SituationKind::GameCpuSchedulerPressure`.

2. **`observation_preserves_irq_signals`**: push synthetic `IrqEventRecord` entries with nonzero `duration_ns` into a `RollingWindow`, call `objective_signals()`, verify `irq_overlap_count` and `irq_worst_overlap_ns` are populated in the resulting observation.

3. **`observation_builder_focus_falls_back_to_unknown_when_confidence_below_threshold`**: construct an observation with a task tree containing only `TaskClass::Service` processes, verify `focus_is_idle_or_unknown()` returns true.

4. **`observation_builder_protected_tasks_exclude_compositor`**: verify that a `TaskClass::Compositor` process appears in `protected_tasks` of the resulting observation.

Add three planner integration golden cases in `testdata/autotune/planner/` that exercise the `observation_signals` path:
- `game_irq_pressure_signals_present.json`: has `irq_overlap_count` and `irq_worst_overlap_ns` set; expects `irq_affinity` candidate eligible
- `game_gpu_power_limited.json`: has `gpu_power_limited=true` and `gpu_busy_percent` high; expects `gpu_power` candidate
- `browser_memory_pressure.json`: has `memory_pressure_some_avg10_percent` nonzero; expects a relevant candidate or explicit denial

AFFECTED SCOPE:
- `stutter/src/autotune/observation_builder.rs` (add tests)
- `testdata/autotune/planner/` (add 3 JSON fixtures)
- `stutter/src/autotune/planner.rs` `expected_names` list must be updated to include the 3 new fixtures

DEPENDENCIES: None. Self-contained.

EDIT REQUEST FOR PATCH WRITER:
Add a `#[cfg(test)] mod tests` block at the bottom of `stutter/src/autotune/observation_builder.rs`. Add 4 unit tests as described above. Each test must construct a `RollingWindow` using `RollingWindow::new(Duration::from_secs(30))`, push synthetic events via the `push_*` methods, call `RollingWindow::objective_signals()` or `RollingWindow::score()` as appropriate, and assert specific fields on the resulting `ObjectiveSignals` or `AutotuneObservation`. Use the existing `test_fixture_builder.rs` helpers where available; do not introduce new test dependencies. Then add 3 new JSON files to `testdata/autotune/planner/` following the exact schema of existing fixtures (`game_cpu_scheduler_pressure.json` is the reference). Update the `expected_names` vec in the `planner_golden_cases` test in `stutter/src/autotune/planner.rs` to include the 3 new fixture names in alphabetical order.

---











PROPOSAL 6: Add a `ControllerStateMachine` integration test covering the full Observing→Apply→Keep/Revert→Cooldown cycle
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
Justifies: the controller state machine's 20+ inline unit tests each test one transition in isolation; no test exercises the full cycle with a real `AutotuneRuntime` tick sequence.

CURRENT STATE:
`autotune/controller.rs` has 20+ `#[test]` blocks each calling `decide_autotune_transition` with a manually-constructed `ControllerRuntimeState`. These are correct and well-written. However, the higher-level `AutotuneRuntime::on_tick` path — which calls `decide_autotune_transition` using state derived from a real `RollingWindow`, `CandidatePlanner`, and `apply_candidate_with_audit` — has no comparable test. `autotune/runtime.rs` has several inline tests (at lines 2076–2808) but they use `simulate_action_effects: true` which bypasses the actual apply/rollback logic. No test drives the runtime through a full experiment lifecycle.

`daemon/soak.rs:run_fake_daemon_soak` is a pure simulation with hardcoded per-tick increments, not a real runtime.

PROPOSED CHANGE:
Add a standalone integration test file `stutter/tests/autotune_lifecycle.rs`. This file must:

1. Construct an `AutotuneRuntimeConfig` with `simulate_action_effects: true`, `daemon_config.mode = DaemonMode::ApplyLowRisk`, a `Fake` candidate with `SafetyClass::ReversibleLowRisk`, and a temporary directory for journal/history paths.

2. Construct an `AutotuneRuntime` and feed it synthetic `MonitorEvent` ticks via the mpsc channel.

3. Assert the following state sequence:
   - After baseline window fills: `controller_state.phase == ControllerPhase::Observing`
   - After planner selects the fake candidate: `phase == ControllerPhase::Applying` or equivalent
   - After candidate measurement window: experiment result is `Improved` (since `simulate_action_effects: true` freezes windows)
   - After keep decision: `kept_candidate.current.is_some()`
   - After a forced revert trigger: rollback token is consumed and phase returns to `Observing`

4. Assert that the journal file contains a clean record after the full cycle.

AFFECTED SCOPE:
- New file: `stutter/tests/autotune_lifecycle.rs`

DEPENDENCIES: Proposal 1 (the test must not panic on I/O errors in startup_recovery, which runs on construction).

EDIT REQUEST FOR PATCH WRITER:
Create the file `stutter/tests/autotune_lifecycle.rs`. It must import from `stutter::{autotune::runtime::{AutotuneRuntime, AutotuneRuntimeConfig}, daemon::{DaemonConfig, DaemonMode}, actions::{ActionSource, SafetyClass, ActionId}, autotune::candidate::CandidateAction, session_events::MonitorEvent}`. Construct an `AutotuneRuntimeConfig` with `simulate_action_effects: true`, `daemon_config.mode = DaemonMode::ApplyLowRisk`, `simulated_candidates` containing one `CandidateAction::Fake { action_id: ActionId("test-fake".to_owned()), safety_class: SafetyClass::ReversibleLowRisk }`, and temp paths for journal/history. Drive the runtime through at least 60 synthetic tick events by calling `runtime.on_tick(MonitorEvent::Tick { ... })` in a loop (use `tokio::test`). After the loop, assert: `runtime.controller_state().phase` is not `ControllerPhase::Faulted`, the history log exists and contains at least one entry, and the journal is in a clean state. The test must complete without panicking.

---











PROPOSAL 7: Add targeted `#[allow(dead_code)]` with wiring TODOs to `diagnosis.rs` dead evidence fields
PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
Justifies: suppressed fields on `DiagnosisCandidate` and `LiveDiagnosisEntry` hide that the advisor does not consume all diagnosis evidence, which creates silent information loss in the recommendation pipeline.

CURRENT STATE:
`stutter/src/diagnosis.rs` lines 68, 78, 253, 263 suppress dead fields. Specifically, `DiagnosisCandidate.evidence_details: Vec<String>` (line 68 area) and `LiveDiagnosisEntry.raw_latencies: Vec<u64>` (line 263 area) are never read by `advisor.rs` or `recommend.rs`. These carry per-event evidence strings and latency samples that the advisor would need to produce specific recommendations (e.g., "IRQ 44 on CPU 2 caused 3ms stutter 7 times").

PROPOSED CHANGE:
Remove the four `#[allow(dead_code)]` attributes from `diagnosis.rs`. For fields that become warnings, add targeted `#[allow(dead_code)] // TODO: consumed by advisor when evidence-detail recommendations are implemented` inline. Add a comment in `advisor.rs` at the top: `// TODO: consume DiagnosisCandidate::evidence_details for specific actionable recommendations`.

AFFECTED SCOPE:
- `stutter/src/diagnosis.rs`
- `stutter/src/advisor.rs` (comment only)

DEPENDENCIES: None.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/diagnosis.rs`, remove the 4 `#[allow(dead_code)]` attributes at lines 68, 78, 253, and 263. For each field that emits a dead_code warning after removal, add `#[allow(dead_code)] // TODO: consumed by advisor in evidence-detail recommendation pass` immediately before that field. Do not remove or change any field values. Add a block comment at the top of `stutter/src/advisor.rs` reading `// TODO: DiagnosisCandidate::evidence_details and LiveDiagnosisEntry::raw_latencies are not yet consumed here. When implementing specific actionable recommendations, read these fields to produce per-IRQ/per-process evidence strings.`

---











PROPOSAL 8: Rename `rolling_window::WindowScore` to `RollingWindowScore` to eliminate namespace collision with `experiment::WindowScore`
PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
Justifies: two public structs named `WindowScore` in sibling modules of the same crate will cause import confusion as the codebase grows; one is already causing implicit type-hiding in `rolling_window.rs` line 20.

CURRENT STATE:
`stutter/src/autotune/rolling_window.rs` line 20 declares `pub struct WindowScore` with fields: `duration_ms: u64`, `interval_count: usize`, `scored_task_count: usize`, `scored_samples: u64`, `score_total: u64`, `over_1ms: u64`, `over_2ms: u64`, `over_5ms: u64`, `max_latency_ns: u64`, `frame_count: usize`, `frame_p99_ms: f64`, `frame_max_ms: f64`, `data_quality: OnlineDataQuality`.

`stutter/src/autotune/experiment.rs` line 23 declares `pub struct WindowScore` with fields: `started_unix_nanos: u128`, `finished_unix_nanos: u128`, `interval_count: usize`, `scored_samples: u64`, `scored_task_count: usize`, `score: StutterScore`.

`comparison.rs` imports `experiment::WindowScore`. `rolling_window.rs` defines its own. Callers who `use crate::autotune::rolling_window::*` and `use crate::autotune::experiment::*` will have a silent collision resolved by whichever import is last.

PROPOSED CHANGE:
Rename `rolling_window::WindowScore` to `rolling_window::RollingWindowScore`. Update all references within `rolling_window.rs` and any callers that import specifically from `rolling_window`. The `experiment::WindowScore` retains its name as it is the canonical type used by `comparison.rs`, `objective.rs`, `live_experiment.rs`, and `controller.rs`.

AFFECTED SCOPE:
- `stutter/src/autotune/rolling_window.rs` (rename struct, all references)
- Any file that imports `WindowScore` from `rolling_window` — check by grepping `rolling_window::WindowScore` and `use.*rolling_window.*WindowScore`

DEPENDENCIES: None.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/rolling_window.rs`, rename the struct `WindowScore` to `RollingWindowScore`. Update every use of `WindowScore` within that file, including the `#[allow(dead_code)]` annotation at line 20 (which should be retargeted to `RollingWindowScore`), the return type of `score_with_quality_policy`, and all internal references. Then grep for any file in `stutter/src/` that imports `WindowScore` from `rolling_window` or uses the fully-qualified path `rolling_window::WindowScore`, and update those to `rolling_window::RollingWindowScore`. Do not rename `experiment::WindowScore`.

---











PROPOSAL 9: Wire `StutterError` at actual error-origination callsites or delete it
PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
Justifies: the typed error enum in `error.rs` exists solely for documentation value right now; it adds maintenance overhead for zero runtime benefit.

CURRENT STATE:
`stutter/src/error.rs` defines `StutterError` with `#[from]` impls for `ConfigError`, `TargetError`, `EbpfError`, `ProbeError`, `RecordingError`, `ArtifactError`, `ReportError`, `RemoteError`. None of these are used at callsites. The entire production codebase returns `anyhow::Result<_>`. `StutterError` is not the return type of `main()` or any public API boundary.

PROPOSED CHANGE:
**Option A (wire, correct approach):** Change `fn main() -> anyhow::Result<()>` to `fn main() -> Result<(), StutterError>`. Change the top-level command dispatch functions in `commands/` to return `Result<(), StutterError>`. This gives the `#[from]` impls a purpose and ensures errors are typed at the API boundary. Internal functions continue using `anyhow::Result`.

**Option B (delete):** Delete `error.rs` entirely. Remove `pub mod error` from `lib.rs`. This is the correct choice if typed errors at the CLI boundary are not valuable for this tool.

AFFECTED SCOPE:
- `stutter/src/error.rs`
- `stutter/src/main.rs` (if Option A)
- `stutter/src/commands/mod.rs` and submodules (if Option A)
- `stutter/src/lib.rs`

DEPENDENCIES: None.

EDIT REQUEST FOR PATCH WRITER:
Choose Option B unless there is a concrete downstream consumer (Prometheus exporter, machine-readable error output, library consumer) that requires typed errors. If choosing Option B: delete `stutter/src/error.rs`, remove `pub mod error;` from `stutter/src/lib.rs`, and remove any `use crate::error::*` imports. Confirm no file other than `error.rs` itself references `StutterError`, `ConfigError`, `TargetError`, `EbpfError`, `ProbeError`, `RecordingError`, `ArtifactError`, `ReportError`, or `RemoteError` in production code (only in test code or the file itself). If other files do reference these, document them before deleting.

---











PROPOSAL 10: Add `AutotuneObservationBuilder` → planner end-to-end scenario tests for the 3 signal paths not covered by golden cases
PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
Justifies: IRQ affinity, GPU power, and memory pressure candidates are selected based on `ObjectiveSignals` fields that come from `rolling_window::objective_signals()`, a path not exercised by any current test.

CURRENT STATE:
The 20 planner golden cases in `testdata/autotune/planner/` construct `AutotuneObservation` via `build_fixture_observation`, which hardcodes `gpu_power_evidence: bool` and similar fields directly — bypassing `rolling_window::objective_signals()` entirely. The providers `irq_affinity.rs`, `gpu_power.rs`, and `vm_knob.rs` read `ObjectiveSignals` from the observation's `hardware_signals` field. If `rolling_window::objective_signals()` returns wrong values (wrong thresholds, missing fields), the planner will silently not generate candidates without any test failing.

PROPOSED CHANGE:
See Proposal 5. This proposal specifically calls out that the 3 new golden cases in Proposal 5 must be constructed by calling `rolling_window::objective_signals()` in their fixture builder, not by hardcoding signal values in the JSON. The fixture builder in `planner.rs::build_fixture_observation` must be extended to accept an `objective_signals: Option<ObjectiveSignals>` field in `PlannerGoldenCase`, and use it if present.

AFFECTED SCOPE:
- `stutter/src/autotune/planner.rs` (extend `PlannerGoldenCase` struct and `build_fixture_observation`)
- `testdata/autotune/planner/` (3 new fixtures from Proposal 5)

DEPENDENCIES: Proposal 5 must be completed first; this extends the infrastructure created there.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/planner.rs`, extend the `PlannerGoldenCase` struct (in the `#[cfg(test)]` section) with an optional field `hardware_signals: Option<serde_json::Value>`. In `build_fixture_observation`, if `hardware_signals` is `Some`, deserialize it as `ObjectiveSignals` and assign it to `observation.hardware_signals`. Update the `PlannerGoldenCase` JSON schema comment. Then, for the 3 new fixture files added in Proposal 5, include a `hardware_signals` JSON object that directly populates the relevant `ObjectiveSignals` fields (e.g., `irq_overlap_count`, `gpu_busy_percent`, `memory_pressure_some_avg10_percent`) rather than relying on the boolean flag shortcuts.

---











PROPOSAL 10.5: Implement active CPU-affinity profile matching
PRIORITY: CRITICAL
STATUS: Completed 2026-05-19.
CPU-affinity profiles are the core currently mature apply family, but live no-op/external-mutation detection still cannot verify them.

CURRENT STATE:
`stutter/src/autotune/active_config.rs` implements `CandidateAction::matches_active_config(&ActiveConfigSnapshot)`. It compares active config for nice, ionice, uclamp, cgroup, IRQ, CPU power, GPU power, and VM knobs, but for `CandidateAction::CpuAffinityProfile` it returns `ActiveConfigMatch::Unknown` with the exact summary text: `"active per-profile CPU affinity matching is not implemented"`. 
`CandidateAction::CpuAffinityProfile` now stores a `CpuAffinityProfilePlan` with `profile_name`, `profile`, and `tree_pid`; `CpuAffinityProfilePlan` declares action kind `cpu_affinity_profile`, effect scope `LocalProcessTree`, objective `StutterScore`, and conflict group `CpuPlacement`. 
Planner no-op detection depends on `candidate.matches_active_config(snapshot)`: if it returns `Matches`, the planner adds `CandidateDenyReason::NoEffectiveChange`. Because CPU-affinity profiles return `Unknown`, the planner cannot deny no-op CPU-affinity candidates or detect kept CPU-affinity drift. 

PROPOSED CHANGE:
Implement CPU-affinity profile matching in `active_config.rs` by evaluating the planned profile rules against `ActiveConfigSnapshot.affinity.per_tid` and `AutotuneObservation.active_tasks`. Add a reusable function:

```rust
pub fn cpu_affinity_profile_match(
    plan: &CpuAffinityProfilePlan,
    snapshot: &ActiveConfigSnapshot,
    active_tasks: &[ActiveTaskSnapshot],
) -> ActiveConfigMatch
```

Because `matches_active_config(&self, snapshot)` currently has no access to active task snapshots, replace it with either:

```rust
pub struct ActiveConfigMatchInput<'a> {
    pub snapshot: &'a ActiveConfigSnapshot,
    pub active_tasks: &'a [ActiveTaskSnapshot],
}

pub fn matches_active_config(&self, input: ActiveConfigMatchInput<'_>) -> ActiveConfigMatch
```

or add a CPU-affinity-specific path in planner where the observation is available. The matcher must:

* Compile/apply profile rule matching using the same semantics as `CpuAffinityProfileAction::dry_run()` / `apply()`.
* For each mutable target task matched by the profile, compare the requested mask to `snapshot.affinity.per_tid[tid]`.
* Return `Matches` only if every planned affected task already has the requested mask.
* Return `Differs` if any planned affected task has a different active mask.
* Return `Unknown` if no target task information is available or required active affinity data is missing.
* Preserve profile rule order and first-match-wins behavior.
* Add tests for exact match, one differing TID, missing affinity data, no matched tasks, excluded/protected tasks, and broad fallback rules.

AFFECTED SCOPE:

* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/candidate.rs`
* `stutter/src/actions/cpu_affinity.rs`
* `stutter/src/profiles.rs`
* planner tests and active-config tests
  This is a medium ripple because the matching API must receive active task snapshots.

DEPENDENCIES:

* Must be done before PROPOSAL 11, PROPOSAL 12, and any broader autonomous apply expansion.

EDIT REQUEST FOR PATCH WRITER:
Implement CPU-affinity profile active-state matching. Currently `CandidateAction::CpuAffinityProfile` returns `ActiveConfigMatch::Unknown` in `stutter/src/autotune/active_config.rs`. Replace that placeholder with rule-accurate matching against active task snapshots and `ActiveConfigSnapshot.affinity.per_tid`. Update the planner call sites so CPU-affinity matching has access to active task snapshots. Add tests proving no-op CPU-affinity profiles are denied and externally changed kept CPU-affinity profiles are detected.

---











PROPOSAL 11: Unify CPU-affinity rule evaluation between profile apply, dry-run, candidate generation, and active matching
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
CPU-affinity logic must not be duplicated across apply, dry-run, generation, and active-state comparison because small semantic drift will make rollback/no-op detection incorrect.

CURRENT STATE:
`CandidateAction::CpuAffinityProfile` wraps a full `Profile` inside `CpuAffinityProfilePlan`. 
`ActiveConfigSnapshot` currently cannot match that profile to active affinity state. 
Planner uses `NoEffectiveChange`, `ExternalMutationDetected`, and `KeptActionNoLongerActive` based on `matches_active_config()`, so any divergence between apply semantics and active matching semantics will directly corrupt planner decisions. 

PROPOSED CHANGE:
Create a shared module:

```rust
stutter/src/profiles/evaluate.rs
```

or:

```rust
stutter/src/actions/cpu_affinity/profile_eval.rs
```

with:

```rust
pub struct ProfileEvaluationInput<'a> {
    pub profile: &'a Profile,
    pub active_tasks: &'a [ActiveTaskSnapshot],
    pub topology: Option<&'a TopologyModel>,
}

pub struct ProfileTaskPlan {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub class: TaskClass,
    pub requested_mask: String,
    pub matched_rule_index: usize,
    pub matched_rule_name: Option<String>,
}

pub fn evaluate_profile_for_tasks(input: ProfileEvaluationInput<'_>) -> Vec<ProfileTaskPlan>
```

Migrate CPU-affinity apply/dry-run and new active matching to call this shared evaluator. Remove duplicated rule matching where it exists. The evaluator must preserve existing profile semantics:

* file-order first matching rule wins
* `match_comm` literal/regex behavior
* `match_class`
* explicit masks
* any existing priority/nice/ionice side effects must remain separate from CPU mask evaluation

AFFECTED SCOPE:

* `stutter/src/actions/cpu_affinity.rs`
* `stutter/src/profiles.rs`
* possible new `stutter/src/profiles/evaluate.rs`
* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/candidate.rs`
* `stutter/src/autotune/providers/cpu_affinity.rs`
* CPU-affinity tests/profile tests
  This is a medium-to-large refactor but is required to make CPU-affinity autonomous behavior trustworthy.

DEPENDENCIES:

* Should be implemented immediately before or together with PROPOSAL 10.
* Blocks safe expansion of apply-low-risk profile retention.

EDIT REQUEST FOR PATCH WRITER:
Extract CPU-affinity profile rule evaluation into one shared evaluator. Replace profile matching logic in apply/dry-run/candidate active matching with this evaluator. The evaluator must produce per-task planned affinity decisions with matched rule metadata. Add regression tests proving dry-run affected tasks and active-config matching use identical task/rule/mask decisions.

---











PROPOSAL 12: Make candidate plan files executable for CPU-affinity profiles or explicitly non-plan-based
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
The project now writes candidate plan files, but CPU-affinity profile candidates have no executable payload in that schema even though they are the primary mature action family.

CURRENT STATE:
`CandidatePlanFile::from_candidate()` writes a plan with `descriptor`, `objective`, `evidence`, and `executable: CandidateExecutablePlan::from_candidate(candidate)`. 
`CandidateExecutablePlan` supports `Nice`, `IoPrio`, `Uclamp`, and `CgroupPlacement`. It returns `None` for `CpuAffinityProfile`, IRQ, CPU power, GPU power, VM knob, and fake candidates. 
CPU-affinity profiles still have separate legacy/manual paths through `apply-profile`, while generic candidate plan files cannot represent the profile payload. This creates two manual-apply paths: profile file/manual command for CPU affinity, JSON executable plan for process-local medium-risk actions.

PROPOSED CHANGE:
Choose one of these two designs and enforce it consistently:

Option A, preferred:
Add CPU-affinity profile executable support:

```rust
CandidateExecutablePlan::CpuAffinityProfile {
    profile: Profile,
    tree_pid: u32,
}
```

Update `CandidateExecutablePlan::from_candidate()` and `into_candidate()` accordingly. Ensure the plan file includes enough data to reconstruct the profile exactly. Add policy validation to reject stale/root PID-invalid plans.

Option B:
Make CPU-affinity candidate plan files explicitly non-executable and add stable fields:

```rust
manual_apply_command: "stutter apply-profile ..."
executable: null
manual_only_reason: "cpu-affinity profiles use apply-profile, not candidate-plan apply"
```

Then generic candidate-plan apply must reject CPU-affinity plan files with a stable reason code rather than silently writing `executable: None`.

AFFECTED SCOPE:

* `stutter/src/autotune/candidate.rs`
* `stutter/src/autotune/human_output.rs`
* `stutter/src/commands/autotune.rs`
* `stutter/src/cli.rs`
* `stutter/src/autotune/apply.rs`
* docs for candidate plan files
  Self-contained if Option B; medium ripple if Option A.

DEPENDENCIES:

* Should follow PROPOSAL 10 and PROPOSAL 11 if Option A needs profile evaluation.
* Should precede any user-facing candidate-plan workflow expansion.

EDIT REQUEST FOR PATCH WRITER:
Resolve the split between CPU-affinity profile suggestions and generic candidate plan files. Currently `CandidateExecutablePlan` excludes CPU-affinity profiles. Either add `CpuAffinityProfile` to `CandidateExecutablePlan` and make plan files executable for CPU-affinity candidates, or make CPU-affinity plan files explicitly manual-only with a stable rejection reason in candidate-plan apply. Add tests for serialization, deserialization, apply rejection/apply reconstruction, and stale PID handling.

---











PROPOSAL 13: Populate missing live objective signals instead of leaving them as `None`
PRIORITY: CRITICAL
STATUS: Completed 2026-05-19.
Objective verification exists, but several critical live signals remain unpopulated, so providers and keep/revert logic cannot reach full-system-tuner reliability.

CURRENT STATE:
`ObjectiveSignals` defines fields for block I/O, IRQ, thermal, CPU power, GPU power, render node, memory pressure, swap activity, dirty writeback, frame p99, and foreground latency. 
`RollingWindow` owns interval, frame, diagnosis, IRQ, block I/O, GPU, CPU frequency, and foreground event queues. 
`RollingWindow::objective_signals()` currently sets `gpu_active_render_node: None`, `memory_pressure_some_avg10_percent: None`, and `swap_activity_events: None`; it only sets dirty writeback when detected, and GPU render-node identity is not connected to focus/workload routing. 

PROPOSED CHANGE:
Extend live signal collection so these fields are populated:

* `gpu_active_render_node`
* `memory_pressure_some_avg10_percent`
* `swap_activity_events`
* `dirty_writeback_events` with actual count, not only optional presence
* CPU power limit source and affected policy
* GPU power limit reason if available
* block I/O overlap basis/trust level
* IRQ overlap basis/trust level

Add `ObjectiveSignalSourceQuality`:

```rust
pub enum ObjectiveSignalQuality {
    Direct,
    Derived,
    Approximate,
    Missing,
}
```

and include quality in either `ObjectiveSignals` or a parallel `ObjectiveSignalQualitySnapshot`.

Update missing-signal behavior:

* Providers must refuse or lower confidence when required signals are `Missing`.
* `compare_for_objective()` must distinguish “missing required signal” from “signal says no improvement.”
* Status output must show missing objective signals as structured diagnostic reasons.

AFFECTED SCOPE:

* `stutter/src/autotune/objective.rs`
* `stutter/src/autotune/rolling_window.rs`
* `stutter/src/recorder.rs`
* `stutter/src/session_events.rs`
* `stutter/src/hwmon.rs`
* `stutter/src/autotune/providers/gpu_power.rs`
* `stutter/src/autotune/providers/vm_knob.rs`
* `stutter/src/autotune/providers/cpu_power.rs`
* `stutter/src/autotune/providers/irq_affinity.rs`
* report/analysis JSON if objective signals are exported
  Large telemetry/verification ripple.

DEPENDENCIES:

* Should be implemented before trusting high-risk suggestions or medium-risk keep/revert decisions.
* Blocks PROPOSAL 14, PROPOSAL 15, PROPOSAL 16, and PROPOSAL 17.

EDIT REQUEST FOR PATCH WRITER:
Populate all currently missing `ObjectiveSignals` fields from live telemetry. `RollingWindow::objective_signals()` must no longer hardcode `gpu_active_render_node`, memory pressure, and swap activity to `None` when the required source data exists. Add signal quality metadata, propagate it into providers and objective comparison, and add tests proving missing required signals block or lower-confidence candidates.

---











PROPOSAL 14: Implement focused-GPU ownership resolution for multi-GPU systems
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
GPU power tuning must never select the wrong GPU on multi-GPU systems.

CURRENT STATE:
`GpuPowerProvider::selected_gpu()` uses `objective_signals.gpu_active_render_node` if present; otherwise it only selects a GPU when there is exactly one DRM device. With multiple DRM devices and no render-node signal, it returns `None`. 
`RollingWindow::objective_signals()` currently sets `gpu_active_render_node: None`. 
Therefore, on multi-GPU systems, GPU power suggestions depend on a signal that is not currently populated.

PROPOSED CHANGE:
Create a module:

```rust
stutter/src/autotune/gpu_focus.rs
```

with:

```rust
pub struct FocusGpuResolver;
pub struct FocusGpuResolution {
    pub render_node: Option<String>,
    pub drm_card: Option<String>,
    pub pci_id: Option<String>,
    pub confidence: f32,
    pub source: FocusGpuSource,
}
```

Resolution sources:

* target process open FDs under `/proc/<pid>/fd` pointing to `/dev/dri/renderD*`
* MangoHud/GPU sample device identity if present
* hwmon/DRM card selected by monitor flags
* explicit config override
* fallback only if single GPU exists

Update `ObjectiveSignals.gpu_active_render_node` from this resolver. Update `GpuPowerProvider` to require `FocusGpuResolution.confidence >= policy threshold` for multi-GPU systems.

AFFECTED SCOPE:

* new `stutter/src/autotune/gpu_focus.rs`
* `stutter/src/autotune/rolling_window.rs`
* `stutter/src/autotune/observation_builder.rs`
* `stutter/src/system_inventory.rs`
* `stutter/src/autotune/providers/gpu_power.rs`
* monitor config/docs for explicit GPU override
  Medium-large hardware routing change.

DEPENDENCIES:

* Requires PROPOSAL 13.
* Must be done before GPU power suggestions are considered trustworthy.

EDIT REQUEST FOR PATCH WRITER:
Add focused-GPU resolution. Populate `ObjectiveSignals.gpu_active_render_node` by inspecting focused target process DRM render-node usage and configured GPU selection. Update `GpuPowerProvider` so multi-GPU systems require a resolved focused GPU. Add tests for one GPU, two GPUs with focused render node, two GPUs without render node, and explicit override.

---











PROPOSAL 15: Add AC/battery and thermal-headroom gates to CPU power provider
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
CPU power tuning must not push performance governor/EPP when the machine lacks power or thermal headroom.

CURRENT STATE:
`CpuPowerProvider` builds `CpuPowerCandidateEvidence` with `ac_power: Option<bool>`, but `cpu_power_evidence()` always sets `ac_power: None`. 
The provider only checks `input.system_health.ok_for_apply`, `objective_signals.cpu_power_limited == Some(true)`, available governors, related CPUs, and whether the current governor/EPP is already performance. 
The confidence calculation includes thermal headroom and CPU limit evidence, but cannot incorporate AC power because it is never collected. 

PROPOSED CHANGE:
Add power-source telemetry:

* AC online status from `/sys/class/power_supply/*/online`
* battery discharging/charging status from `/sys/class/power_supply/*/status`
* optional config override for desktop systems without batteries

Add to `SystemInventory` or `SystemContextSnapshot`:

```rust
pub struct PowerSourceSnapshot {
    pub ac_online: Option<bool>,
    pub battery_present: bool,
    pub battery_discharging: Option<bool>,
}
```

Update CPU power provider:

* Do not propose performance governor/EPP when battery is discharging unless explicit config allows it.
* Require thermal headroom.
* Include AC/battery status in evidence and confidence.
* Add policy config: `allow_cpu_power_on_battery = false` default.

AFFECTED SCOPE:

* `stutter/src/system_inventory.rs`
* `stutter/src/autotune/system_context.rs`
* `stutter/src/autotune/providers/cpu_power.rs`
* `stutter/src/daemon/config.rs`
* `stutter/src/config/model.rs`
* `stutter/src/config/schema.rs`
* docs and provider tests
  Medium system-context/provider ripple.

DEPENDENCIES:

* Should follow PROPOSAL 13.
* Required before CPU power suggestions are trusted.

EDIT REQUEST FOR PATCH WRITER:
Collect AC/battery state and feed it into `CpuPowerProvider`. Replace `ac_power: None` with real power-source evidence. Block CPU power performance candidates while on battery by default. Add config override, evidence output, confidence integration, and tests for AC online, battery discharging, no battery desktop, thermal degraded, and already-performance policy.

---











PROPOSAL 16: Expand VM knob provider beyond fixed swappiness and add knob-specific policies
PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
The VM provider currently represents the entire VM tuning surface as one fixed `vm.swappiness=10` proposal.

CURRENT STATE:
`VmKnobProvider` only constructs one candidate named `"vm-swappiness-investigate-10"`, writing `proc/sys/vm/swappiness` to `"10"`. 
It requires at least one of memory pressure, swap activity, or dirty writeback evidence, and refuses if current swappiness is already `10`. 
`ObjectiveSignals` has fields for memory pressure, swap activity, and dirty writeback, but some are currently unpopulated by rolling-window live signals.

PROPOSED CHANGE:
Add a VM tuning policy table:

```rust
pub struct VmKnobPolicy {
    pub knob: &'static str,
    pub safe_values: Vec<String>,
    pub trigger: VmKnobTrigger,
    pub objective: ObjectiveKind,
    pub safety_class: SafetyClass,
    pub manual_only: bool,
}
```

Initial supported suggestions:

* `vm.swappiness` for swap-heavy interactive workloads
* `vm.dirty_background_ratio` or `dirty_background_bytes` for dirty writeback stalls
* `vm.dirty_ratio` or `dirty_bytes` for writeback pressure
* no transparent hugepage changes unless separately modeled

Rules:

* Do not suggest mutually exclusive ratio/bytes knobs simultaneously.
* Do not suggest sysctl changes without direct evidence.
* Do not apply VM knobs autonomously.
* Include current value, proposed value, trigger evidence, and rollback value in evidence.

AFFECTED SCOPE:

* `stutter/src/autotune/providers/vm_knob.rs`
* `stutter/src/actions/vm_knobs.rs`
* `stutter/src/autotune/objective.rs`
* `stutter/src/system_inventory.rs`
* docs/config for VM tuning
  Medium provider/action expansion.

DEPENDENCIES:

* Requires PROPOSAL 13 for memory/writeback/swap signals.
* Must remain manual-only until high-risk apply support exists.

EDIT REQUEST FOR PATCH WRITER:
Refactor `VmKnobProvider` from a single hardcoded swappiness proposal into a knob-policy-driven provider. Add knob-specific triggers, evidence, rollback value capture, and mutual-exclusion rules. Keep all VM knob candidates manual-only/high-risk. Add tests for swap pressure, dirty writeback pressure, already-target-value no-op, missing evidence, and conflicting ratio/bytes knobs.

---











PROPOSAL 17: Add IRQ CPU-placement policy that avoids moving IRQs onto protected/focused CPUs blindly
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
IRQ affinity suggestions currently choose the least-busy CPU from IRQ counters, but a full tuner must account for focused workload placement and protected CPU roles.

CURRENT STATE:
`IrqAffinityProvider` selects a hot IRQ from structured objective signals, looks up current IRQ affinity from active config, finds an IRQ line in inventory, then calls `least_busy_cpu(irq_line)` or falls back to `signals.irq_hot_cpu`; it converts that CPU to a single-CPU mask. 
The provider does not consult focused workload CPU placement, CPU topology role, isolated/reserved cores, compositor/audio CPU reservations, or current CPU-affinity profile intent. 

PROPOSED CHANGE:
Add:

```rust
pub struct CpuPlacementMap {
    pub focused_workload_cpus: BTreeSet<u32>,
    pub compositor_cpus: BTreeSet<u32>,
    pub audio_realtime_cpus: BTreeSet<u32>,
    pub housekeeping_cpus: BTreeSet<u32>,
    pub reserved_cpus: BTreeSet<u32>,
    pub candidate_irq_cpus: BTreeSet<u32>,
}
```

Use it in IRQ provider:

* Never suggest moving IRQs to audio realtime CPUs.
* Prefer housekeeping CPUs when available.
* Avoid focused render/game CPUs unless IRQ belongs to the focused device and overlap evidence says current placement is worse.
* Consider SMT siblings if topology is available.
* Do not suggest single-CPU mask if target CPU is outside allowed IRQ candidate set.
* Include placement rationale in evidence.

AFFECTED SCOPE:

* `stutter/src/autotune/providers/irq_affinity.rs`
* `stutter/src/autotune/system_context.rs`
* `stutter/src/topology.rs`
* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/observation.rs`
* provider tests
  Medium provider/topology change.

DEPENDENCIES:

* Requires PROPOSAL 10 or equivalent CPU-placement visibility for current profile interactions.
* Should follow PROPOSAL 13.

EDIT REQUEST FOR PATCH WRITER:
Add CPU placement awareness to `IrqAffinityProvider`. Replace `least_busy_cpu()` as the only target selector with a policy that accounts for focused workload CPUs, protected classes, housekeeping CPUs, topology, and reserved CPUs. Add tests proving IRQ suggestions do not target audio/compositor/protected CPUs and include structured placement evidence.

---








PROPOSAL 18: Remove unsafe fallback root task selection for apply-capable modes
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
When active task snapshots are missing, target selection manufactures a mutable helper-class root task, which is dangerous for autonomous mutation.

CURRENT STATE:
`mutable_task_targets_for_observation()` and `mutable_task_snapshots_for_observation()` use `fallback_root_snapshot(observation)` when `observation.active_tasks` is empty. 
`fallback_root_snapshot()` builds an `ActiveTaskSnapshot` using `target_root_pid`, sets `tid = root_pid`, `process_pid = root_pid`, and assigns `class: TaskClass::Helper`. 
Protected-task filtering then sees the fallback as `Helper`, not as `Unknown`, so process-local providers can target it in apply-capable paths.

PROPOSED CHANGE:
Replace fallback behavior with explicit mode-sensitive behavior:

* In suggest mode, fallback root may be used only for display/suggestions and must add evidence/deny message: `target_selection_fallback_root`.
* In apply modes, empty `active_tasks` must return no mutable targets.
* Add `CandidateDenyReason::TargetSnapshotMissing` or provider deny reason.
* `fallback_root_snapshot()` must classify fallback as `Unknown` unless the workload identity has a validated class.

AFFECTED SCOPE:

* `stutter/src/autotune/target_selection.rs`
* `stutter/src/autotune/providers/nice.rs`
* `stutter/src/autotune/providers/ioprio.rs`
* `stutter/src/autotune/providers/uclamp.rs`
* `stutter/src/autotune/providers/cgroup.rs`
* `stutter/src/autotune/planner.rs`
* tests
  Medium safety change.

DEPENDENCIES:

* Should be implemented before broad medium-risk apply is used.
* Related to PROPOSAL 19.

EDIT REQUEST FOR PATCH WRITER:
Remove apply-capable fallback targeting from `target_selection.rs`. When `active_tasks` is empty, process-local providers must not produce apply-eligible targets in apply modes. Classify fallback roots as unknown unless explicitly validated. Add tests proving nice/ionice/uclamp/cgroup providers emit no apply-eligible candidates when active task snapshots are missing.

---











PROPOSAL 19: Add target identity revalidation immediately before apply
PRIORITY: CRITICAL
STATUS: Completed 2026-05-19.
A candidate selected from one observation must not mutate a PID/TID that has exited and been reused.

CURRENT STATE:
`target_selection.rs` converts `ActiveTaskSnapshot` into `TaskIdentity` with `tid`, `process_pid`, `comm`, and `starttime_ticks` from task/process snapshot fields. 
`LiveExperimentManager` starts experiments by applying the selected candidate later through `RuntimeLiveExperimentActionExecutor`, using a candidate cloned from planning time. 
`PrivilegedActionService::validate_candidate_plan_request()` validates plan age, descriptor match, objective match, and evidence count, but it does not re-read `/proc/<tid>/stat` to confirm all target identities are still the same before apply. 

PROPOSED CHANGE:
Add target identity revalidation to the privileged apply path:

* For every candidate containing process/task targets, enumerate `TaskIdentity` targets.
* Re-read `/proc/<tid>/stat` or equivalent under configured proc root.
* Confirm `starttime_ticks` matches if provided.
* Confirm process PID matches if provided.
* Confirm `comm` mismatch is either rejected or downgraded depending on policy.
* Reject the whole candidate if any target is stale, unless the action supports partial safe apply and policy explicitly allows it.

Add:

```rust
pub enum TargetRevalidationError {
    MissingTid,
    StarttimeMismatch,
    ProcessPidMismatch,
    CommMismatch,
}
```

AFFECTED SCOPE:

* `stutter/src/daemon/privilege.rs`
* `stutter/src/actions/mod.rs`
* `stutter/src/actions/nice.rs`
* `stutter/src/actions/ioprio.rs`
* `stutter/src/actions/uclamp.rs`
* `stutter/src/actions/cgroup.rs`
* `stutter/src/actions/cpu_affinity.rs`
* `stutter/src/procfs_utils.rs` or new module
* tests with fake proc roots
  Medium-large apply safety change.

DEPENDENCIES:

* Should follow PROPOSAL 18.
* Required before medium-risk process-local apply is default-trustworthy.

EDIT REQUEST FOR PATCH WRITER:
Add pre-apply task identity revalidation to the privileged action service. Before applying any candidate with task targets, re-read procfs and verify each target’s TID/process/starttime identity still matches the selected observation. Reject stale or reused targets with stable error codes. Add fake-proc tests for missing TID, reused TID, comm mismatch, and valid target.

---











PROPOSAL 20: Convert `InProcessPrivilegedActionService` into a real IPC-backed privileged worker
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
The privilege boundary is currently an in-process abstraction; a full system-wide tuner needs an actual separated privileged mutator.

CURRENT STATE:
`daemon/privilege.rs` defines `PrivilegedActionService` with `dry_run_candidate`, `apply_candidate`, and `rollback`. It also defines roles, transports, operations, request authorization, and an allowlist. 
The only concrete service implementation is `InProcessPrivilegedActionService`, which directly calls `executor_for_candidate()`, `executor.dry_run()`, `executor.apply_with_audit()`, and `executor.rollback()`. 
`LiveExperimentManager` instantiates `InProcessPrivilegedActionService` directly for medium-risk apply and rollback. 

PROPOSED CHANGE:
Add an IPC-backed worker:

* Unix socket transport for local control plane → privileged worker.
* Request/response JSON or bincode schema.
* Authentication/authorization token or filesystem permission model.
* Worker process mode: `stutter privileged-worker --socket <path>`.
* Control-plane client implementing `PrivilegedActionService`.
* In-process service allowed only for tests and explicit unsafe dev mode.

Update `LiveExperimentManagerInput` to receive `Box<dyn PrivilegedActionService>` or a service handle instead of constructing `InProcessPrivilegedActionService` internally.

AFFECTED SCOPE:

* `stutter/src/daemon/privilege.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/cli.rs`
* `stutter/src/commands/daemon.rs`
* `stutter/src/agent.rs`
* `contrib/openrc/stutter`
* `contrib/systemd/*.service` if present
* docs/install/safety/daemon contract
  Large architectural change.

DEPENDENCIES:

* Should follow PROPOSAL 19.
* Required before autonomous system-wide apply is acceptable.

EDIT REQUEST FOR PATCH WRITER:
Implement a real privileged worker process over a local Unix socket. Replace direct construction of `InProcessPrivilegedActionService` in live experiment code with dependency injection. Keep in-process service only for tests. Add command-line worker mode, IPC request/response schema, authentication/authorization checks, audit logging, and integration tests using a temporary Unix socket.

---











PROPOSAL 21: Audit privileged boundary decisions, not only action execution
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
The privilege module defines audit event helpers, but the service path does not audit allow/deny decisions at the boundary.

CURRENT STATE:
`privileged_operation_audit_event()` builds an `AuditEvent` with command `"daemon_privilege"` and stable privilege action IDs. 
`InProcessPrivilegedActionService::dry_run_candidate()`, `apply_candidate()`, and `rollback()` validate the candidate and execute the action, but the shown implementation does not append a privilege-boundary audit event for request allowed/denied decisions. 
Action execution itself uses audited action runners, but that is not the same as auditing the boundary request and authorization decision.

PROPOSED CHANGE:
Add boundary audit writes for:

* privilege request received
* allowlist decision
* policy validation result
* stale plan rejection
* descriptor mismatch
* objective mismatch
* missing evidence
* apply started
* apply completed
* rollback requested
* rollback completed
* rollback failed

Add:

```rust
pub struct PrivilegeAuditSink { ... }
```

and pass it into `PrivilegedActionService` implementations.

AFFECTED SCOPE:

* `stutter/src/daemon/privilege.rs`
* `stutter/src/audit.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/actions/runner.rs`
* tests for audit output
  Medium privilege/audit change.

DEPENDENCIES:

* Should be implemented before PROPOSAL 20 or as part of it.
* Required before remote/local privileged worker is trusted.

EDIT REQUEST FOR PATCH WRITER:
Add explicit audit logging for every privilege boundary decision in `daemon/privilege.rs`. Boundary audit must record allow/deny, reason code, caller role, transport, operation, policy intent, action ID, and error category. Add tests proving denied stale plans, descriptor mismatches, missing evidence, and successful apply/rollback all write privilege audit events.

---











PROPOSAL 22: Replace fake soak harness with scenario-driven live planner/controller simulation
PRIORITY: HIGH
STATUS: Completed 2026-05-19.
The current soak harness tests synthetic counters, not real planner/controller safety behavior.

CURRENT STATE:
`DaemonSoakProfile` only has `ObserveOnly` and `ApplyLowRiskFake`. 
`run_fake_daemon_soak()` simulates ticks, memory/disk/history growth, fake action counts, and fake rollback counts; it does not run actual planner proposals, controller decisions, live experiment transitions, active-config matching, target selection, privilege validation, or provider logic. 

PROPOSED CHANGE:
Create scenario-driven soak tests:

```rust
pub struct SoakScenario {
    pub name: String,
    pub ticks: Vec<SoakTick>,
    pub assertions: Vec<SoakAssertion>,
}
```

Required scenarios:

* game → browser → game
* compile background while browser foreground
* recording + game
* media playback + compile
* VM load + desktop interaction
* thermal degradation during experiment
* target disappears during experiment
* external mutation while kept action is active
* repeated candidate cooldown
* low data quality burst
* high-risk suggestion in suggest mode
* high-risk candidate in apply mode denied

Assertions:

* one active experiment maximum
* no high-risk autonomous apply
* no apply during low data quality
* no protected task mutation
* rollback token exists before apply
* shutdown restores active actions
* cooldown respected
* focus flapping does not cause action flapping

AFFECTED SCOPE:

* `stutter/src/daemon/soak.rs`
* `stutter/src/autotune/simulation.rs`
* `stutter/src/autotune/replay.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* new `testdata/autotune/soak/*.json`
  Large test harness expansion.

DEPENDENCIES:

* Should follow PROPOSAL 10, PROPOSAL 13, PROPOSAL 18, and PROPOSAL 19.
* Required before medium-risk apply is made a normal user workflow.

EDIT REQUEST FOR PATCH WRITER:
Replace the fake soak harness with scenario-driven planner/controller simulation. Add JSON soak scenarios that feed observations through provider registry, planner, controller, and live experiment manager using fake executors. Assert no unsafe apply, correct rollback behavior, cooldowns, protected-task safety, and high-risk manual-only behavior.

---











PROPOSAL 23: Rename and generalize `LiveLowRiskExperiment`
PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
The live experiment manager now handles medium-risk apply through the privileged service, but its primary state type still encodes “low risk” in its name.

CURRENT STATE:
`live_experiment.rs` defines `LiveLowRiskExperiment` with experiment ID, candidate, baseline score/signals, applied time, washout/measurement deadlines, and rollback token. 
`LiveExperimentManager` can start medium-risk candidates when `input.mode == DaemonMode::ApplyMediumRisk`; it uses `InProcessPrivilegedActionService` for medium-risk apply and the legacy low-risk path otherwise. 
`validate_start_candidate()` explicitly handles both `ApplyLowRisk` and `ApplyMediumRisk`. 

PROPOSED CHANGE:
Rename:

* `LiveLowRiskExperiment` → `LiveExperiment`
* `ActiveLowRiskActionRegistry` references, if still generic, to `ActiveAutotuneActionRegistry`
* low-risk-specific variable names where they now include medium-risk candidates

Add field:

```rust
pub safety_class: SafetyClass
pub mode: DaemonMode
```

to the live experiment state. Ensure daemon status, rollback state, journal metadata, and history output include actual mode and safety class.

AFFECTED SCOPE:

* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/autotune/shutdown.rs`
* `stutter/src/autotune/runtime.rs`
* `stutter/src/daemon/state.rs`
* `stutter/src/autotune/history.rs`
* tests
  Mostly mechanical but medium ripple.

DEPENDENCIES:

* Should follow medium-risk apply path stabilization.
* Helps PROPOSAL 22 and PROPOSAL 24.

EDIT REQUEST FOR PATCH WRITER:
Rename live experiment state from low-risk-specific names to generic names. Store safety class and daemon mode in the experiment state. Update daemon status, rollback state, journal records, history records, and tests so medium-risk experiments are not represented as low-risk experiments.

---








PROPOSAL 24: Add multi-action kept-state model with explicit conflict resolution

PRIORITY: HIGH
STATUS: Completed 2026-05-19.
A full system-wide tuner eventually needs multiple compatible kept actions, but current kept-state handling appears to model only one current kept candidate.

CURRENT STATE:
Planner checks one kept action through `active_profile_state.current.as_ref()` and denies candidates that conflict with that single kept candidate. 
`LiveExperimentManager` receives `ActiveProfileState` and writes daemon rollback state for the current experiment, but the shown planner interaction does not support a set of compatible kept actions.
`CandidateAction::conflict_group()` exists, and `conflicts_with()` compares conflict groups. 

PROPOSED CHANGE:
Replace single kept action state with:

```rust
pub struct KeptActionSet {
    pub actions: BTreeMap<ActionConflictGroup, KeptCandidateState>,
}
```

Rules:

* One kept action per conflict group by default.
* Compatible conflict groups may coexist.
* New candidate replaces an existing kept action only through explicit replace/rollback sequence.
* Shutdown must restore all non-persistent kept actions.
* Status must list all kept actions.
* Planner must check candidate against every kept action.

AFFECTED SCOPE:

* `stutter/src/autotune/kept.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/autotune/shutdown.rs`
* `stutter/src/daemon/state.rs`
* `stutter/src/autotune/status.rs`
* tests
  Large state model change.

DEPENDENCIES:

* Should follow PROPOSAL 23.
* Needed before combining CPU affinity + process-local priority + cgroup kept actions.

EDIT REQUEST FOR PATCH WRITER:
Replace single kept-action state with a conflict-group-indexed kept action set. Update planner conflict checks, live experiment keep/revert logic, shutdown restore, daemon status, and history to support multiple compatible kept actions while still preventing conflicting actions from stacking.

---








PROPOSAL 25: Add real external mutation recovery workflow

PRIORITY: HIGH
STATUS: Completed 2026-05-19.
Planner can detect external mutation, but the system needs a recovery path that tells the daemon whether to restore, resync, or abandon state.

CURRENT STATE:
Planner adds `ExternalMutationDetected` when an active experiment’s candidate conflicts with the proposal and the active candidate’s `matches_active_config()` returns `Differs`. It adds `KeptActionNoLongerActive` when a kept candidate differs from live state. 
No complete recovery workflow is visible in the planner code: it only denies new candidates and emits messages such as “restore or resync before planning new candidates.” 

PROPOSED CHANGE:
Add daemon recovery decisions:

* `RestoreExpectedState`
* `AcceptExternalMutationAndResync`
* `AbandonKeptAction`
* `FaultRequireManualRestore`

Add config:

```toml
external_mutation_policy = "fault" | "restore" | "resync"
```

Default:

* active experiment mutation → rollback/fault
* kept action mutation → observe-only/fault unless explicit resync configured

Add command:

```bash
stutter daemon resync-state --dry-run
stutter daemon resync-state
```

AFFECTED SCOPE:

* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/autotune/kept.rs`
* `stutter/src/daemon/state.rs`
* `stutter/src/commands/daemon.rs`
* `stutter/src/cli.rs`
* docs
  Medium-large behavior addition.

DEPENDENCIES:

* Requires PROPOSAL 10 for CPU-affinity external mutation.
* Should follow PROPOSAL 24.

EDIT REQUEST FOR PATCH WRITER:
Implement an explicit external mutation recovery workflow. Planner denial must lead to a daemon state transition: restore, resync, abandon kept action, or fault. Add config and CLI commands for safe resync. Add tests for active experiment mutation, kept action mutation, restore success, restore failure, and manual resync.

---








PROPOSAL 26: Make high-risk manual suggestions produce dry-run evidence without enabling apply

PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
High-risk candidates are manual-only, but users still need safe dry-run diagnostics for why a high-risk suggestion exists and what it would touch.

CURRENT STATE:
Planner marks high-risk/system-adjacent candidates with `ManualOnlyHighRisk` before dry-run. 
`dry_run_candidate_if_still_eligible()` skips dry-run when mode is `Suggest` and the candidate is high-risk/system-adjacent. 
High-risk providers emit evidence, but dry-run affected state is not collected in normal suggest mode.

PROPOSED CHANGE:
Add a separate safe high-risk dry-run mode:

* `suggest`: no high-risk dry-run by default
* `suggest --high-risk-dry-run`: run high-risk dry-run only, never apply
* dry-run must use policy intent `DryRun`
* dry-run output must include affected scope and rollback availability
* apply command must remain absent/manual-blocked

Add planner field:

```rust
pub high_risk_dry_run: bool
```

or policy/config equivalent.

AFFECTED SCOPE:

* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/candidate.rs`
* `stutter/src/cli.rs`
* `stutter/src/commands/autotune.rs`
* `stutter/src/autotune/status.rs`
* docs
  Medium CLI/planner addition.

DEPENDENCIES:

* Must keep PROPOSAL 20 and high-risk apply guards intact.
* Should follow PROPOSAL 21 audit improvements.

EDIT REQUEST FOR PATCH WRITER:
Add an explicit high-risk dry-run-only suggestion mode. Keep high-risk candidates manual-only and non-applyable, but allow users to request audited dry-run diagnostics for high-risk candidates. Update planner, CLI, status output, and tests so high-risk dry-run never produces a live `StartExperiment`.

---








PROPOSAL 27: Add confidence calibration per provider family

PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
Provider confidence currently uses local completeness formulas, but there is no central calibration to make scores comparable across action families.

CURRENT STATE:
`CandidateProposal` includes `confidence`, and planner uses policy thresholds and ranking by confidence.
IRQ confidence is `situation.confidence * completeness`; GPU confidence is `situation.confidence * completeness`; CPU power confidence is also `situation.confidence * completeness`.
These formulas use different completeness dimensions and therefore are not calibrated across provider families.

PROPOSED CHANGE:
Add:

```rust
pub struct ProviderConfidenceCalibration {
    pub family: String,
    pub min_required_signals: Vec<String>,
    pub direct_signal_weight: f32,
    pub inferred_signal_weight: f32,
    pub max_without_direct_signal: f32,
    pub max_without_active_config: f32,
}
```

Add per-family calibration defaults:

* CPU affinity: high if dry-run affects targets and focus confidence high
* nice/ionice/uclamp/cgroup: require active task snapshots
* IRQ: cap if no stable IRQ identity or no current mask
* GPU: cap if no focused render node on multi-GPU
* CPU power: cap if no AC/battery state
* VM: cap if no direct memory/swap/writeback signal

Apply calibration centrally in planner or provider registry after proposal creation.

AFFECTED SCOPE:

* `stutter/src/autotune/providers/mod.rs`
* `stutter/src/autotune/planner.rs`
* all provider files
* `stutter/src/daemon/config.rs`
* docs/config
  Medium provider-policy change.

DEPENDENCIES:

* Should follow PROPOSAL 13, PROPOSAL 14, and PROPOSAL 15.

EDIT REQUEST FOR PATCH WRITER:
Add centralized provider confidence calibration. Do not let providers return uncalibrated confidence directly into planner ranking. Implement per-family caps and required signals. Add tests proving missing focused GPU, missing AC power, missing active config, and missing IRQ identity cap confidence below apply thresholds.

---








PROPOSAL 28: Add hardware allowlists for CPU/GPU/IRQ/VM system-adjacent suggestions

PRIORITY: HIGH
System-wide suggestions must be constrained to user-approved devices and knobs before the project moves toward automated system-wide tuning.

CURRENT STATE:
`DaemonPolicy` has system-wide suggestion/apply flags, high-risk flags, enabled/denied action families, and cgroup targets. 
High-risk providers can emit IRQ, CPU power, GPU power, and VM knob candidates using inventory and active config.
There is no cited equivalent of `cgroup_targets` for allowed DRM cards, CPU policies, IRQ devices, or sysctl knobs.

PROPOSED CHANGE:
Add policy config:

```toml
[system_wide_allowlist]
cpu_policies = ["policy0", "policy1"]
gpu_cards = ["card0"]
gpu_pci_ids = ["1002:*"]
irq_devices = ["amdgpu", "xhci_hcd"]
vm_knobs = ["proc/sys/vm/swappiness"]
```

Planner/provider behavior:

* High-risk providers must not emit candidates outside allowlist unless in diagnostic-only mode.
* Status must report deny reason `SystemWideTargetNotAllowlisted`.
* Empty allowlist means no system-wide targets are allowed by default for apply; suggestions may be diagnostic-only depending on config.

AFFECTED SCOPE:

* `stutter/src/daemon/config.rs`
* `stutter/src/daemon/policy.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/providers/irq_affinity.rs`
* `stutter/src/autotune/providers/cpu_power.rs`
* `stutter/src/autotune/providers/gpu_power.rs`
* `stutter/src/autotune/providers/vm_knob.rs`
* docs/config
  Medium policy/provider change.

DEPENDENCIES:

* Should follow PROPOSAL 14, PROPOSAL 15, and PROPOSAL 16.
* Required before any future high-risk apply work.

EDIT REQUEST FOR PATCH WRITER:
Add system-wide target allowlists for CPU policies, GPUs, IRQ devices, and VM knobs. Enforce them in high-risk providers or planner. Add structured denial reasons and tests proving non-allowlisted cards, policies, IRQs, and sysctls are not suggested or applied.

---








PROPOSAL 29: Add rollback verification after every rollback operation

PRIORITY: HIGH
Rollback success must be verified against active config, not trusted only because the rollback function returned `Ok`.

CURRENT STATE:
`LiveExperimentManager` stores a rollback token and calls the privileged service’s `rollback()` during rollback paths.
`InProcessPrivilegedActionService::rollback()` checks policy, calls `executor.rollback(&request.token)`, and returns affected task count. 
No cited rollback path re-collects `ActiveConfigSnapshot` and verifies the system returned to the expected baseline state.

PROPOSED CHANGE:
Capture pre-apply baseline active config for the candidate conflict group. Store it with live experiment state and rollback token. After rollback:

* Recollect active config.
* Compare conflict-group-relevant state to baseline.
* If rollback did not restore expected state, enter fault state and expose manual restore command.
* Audit rollback verification result.

Add:

```rust
pub struct RollbackVerification {
    pub verified: bool,
    pub expected: String,
    pub actual: String,
    pub reason_code: String,
}
```

AFFECTED SCOPE:

* `stutter/src/autotune/live_experiment.rs`
* `stutter/src/daemon/privilege.rs`
* `stutter/src/autotune/active_config.rs`
* `stutter/src/autotune/shutdown.rs`
* `stutter/src/actions/*`
* tests
  Large safety change.

DEPENDENCIES:

* Requires PROPOSAL 10 and PROPOSAL 13.
* Should precede any autonomous medium-risk default workflow.

EDIT REQUEST FOR PATCH WRITER:
Add rollback verification. Capture conflict-group-specific active config before apply, verify active config after rollback, and fault if rollback does not restore expected state. Add audit events and tests for successful rollback, incomplete rollback, missing target after rollback, and rollback verification unavailable.

---








PROPOSAL 30: Add build/test/CI gate for the full watcher safety matrix

PRIORITY: HIGH
The project now has enough safety-sensitive architecture that CI must enforce the invariants automatically.

CURRENT STATE:
The workspace has Rust crates `stutter`, `stutter-common`, and `stutter-ebpf`, with default members `stutter` and `stutter-common`. 
The codebase contains planner, privilege, provider, objective, rolling-window, and soak tests in source modules, but the review environment did not verify a full build/test run.
Safety invariants include high-risk apply disabled, medium-risk unlock required, manual-only high-risk suggestions, no protected task selection, and policy gates in planner. These are distributed across `planner.rs`, `policy.rs`, `target_selection.rs`, `privilege.rs`, and provider modules.

PROPOSED CHANGE:
Add CI jobs:

* `cargo fmt --all --check`
* `cargo build --all-targets`
* `cargo clippy --all-targets -- -D warnings`
* `cargo test --all-targets`
* focused safety tests:

  * planner golden fixtures
  * high-risk apply disabled
  * protected task selection
  * privilege boundary
  * rollback verification
  * soak scenarios

Add a script:

```bash
scripts/check-autotune-safety.sh
```

that runs only the safety matrix tests for local iteration.

AFFECTED SCOPE:

* `.github/workflows/*.yml`
* `scripts/check-autotune-safety.sh`
* test modules across `autotune`, `daemon`, `actions`
* docs contributing/test instructions
  Medium repository/CI change.

DEPENDENCIES:

* Should be maintained continuously.
* Add new safety tests from proposals as they land.

EDIT REQUEST FOR PATCH WRITER:
Add CI and local scripts that enforce formatting, build, clippy, full tests, and an explicit autotune safety matrix. Include tests for high-risk apply disabled, protected task exclusion, medium-risk unlock, privilege boundary denial, planner no-op detection, rollback verification, and soak scenarios.

---








PROPOSAL 31: Decompose large architecture hubs after safety gates are stable

PRIORITY: MEDIUM
Large files are now the main maintainability risk and will slow future full-system tuning work.

CURRENT STATE:
`candidate.rs` owns candidate enum, plan file schema, executable plan schema, evidence, candidate-plan trait, all plan structs, CPU-affinity plan metadata, and much more.
`planner.rs` owns deny reasons, evaluation DTOs, planner summaries, sorting, grouping, provider input construction, static evaluation, active-config checks, dry-run gating, and tests.
`live_experiment.rs` owns experiment state, privileged apply, rollback, journal side effects, controller state mutation, keep/revert decisions, deadline computation, objective comparison, and history contexts.

PROPOSED CHANGE:
Split modules:

`candidate.rs` into:

* `candidate/mod.rs`
* `candidate/action.rs`
* `candidate/plan.rs`
* `candidate/evidence.rs`
* `candidate/plan_file.rs`
* `candidate/executable.rs`
* `candidate/manual_commands.rs`

`planner.rs` into:

* `planner/mod.rs`
* `planner/deny.rs`
* `planner/evaluation.rs`
* `planner/summary.rs`
* `planner/sort.rs`
* `planner/static_gates.rs`
* `planner/dry_run.rs`

`live_experiment.rs` into:

* `live_experiment/mod.rs`
* `live_experiment/state.rs`
* `live_experiment/apply.rs`
* `live_experiment/rollback.rs`
* `live_experiment/journal.rs`
* `live_experiment/keep_revert.rs`

No behavior changes in the split PR. Move tests with their modules.

AFFECTED SCOPE:

* `stutter/src/autotune/candidate.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/live_experiment.rs`
* imports across autotune/provider/runtime/status modules
* tests
  Large mechanical refactor.

DEPENDENCIES:

* Should happen after PROPOSAL 10, PROPOSAL 18, and PROPOSAL 19 to avoid merge conflicts.
* Should happen before adding high-risk apply implementation.

EDIT REQUEST FOR PATCH WRITER:
Perform a behavior-preserving module split of `candidate.rs`, `planner.rs`, and `live_experiment.rs`. Keep public APIs stable through `mod.rs` re-exports. Do not change runtime behavior. Move tests into the closest new module. Run full fmt/build/clippy/test after the split.

---








PROPOSAL 32: Add automated workload policy validation and linting

PRIORITY: MEDIUM
Configurable workload policy is powerful, but invalid policy can accidentally allow dangerous autonomous actions.

CURRENT STATE:
Planner uses `workload_policy.rule_for(observation.primary_situation)` and enforces `allows_candidate`, `allows_autonomous_candidate`, and `allows_objective`. 
`DaemonPolicy` has enabled/denied action families, allowed scopes, confidence config, and system-wide flags. 
System-wide/high-risk action families can be suggested but must remain blocked for autonomous apply.

PROPOSED CHANGE:
Add policy linter:

```rust
pub struct WorkloadPolicyLint {
    pub severity: LintSeverity,
    pub reason_code: String,
    pub message: String,
}
```

Lint rules:

* high-risk families must not appear in `autonomous_families` while high-risk apply disabled
* system-wide families must not be autonomous unless system-wide apply enabled
* objective must match family capability
* empty autonomous list must be explicit, not accidental
* denied family must not also be autonomous
* apply-low-risk presets must not enable medium/high-risk autonomous families

Expose:

```bash
stutter daemon policy-lint
stutter daemon policy-lint --json
```

AFFECTED SCOPE:

* `stutter/src/autotune/workload_policy.rs`
* `stutter/src/daemon/policy.rs`
* `stutter/src/commands/daemon.rs`
* `stutter/src/cli.rs`
* config docs/tests
  Medium config safety addition.

DEPENDENCIES:

* Should follow system-wide allowlist work from PROPOSAL 28.
* Helps before user-editable full-system policy.

EDIT REQUEST FOR PATCH WRITER:
Implement workload policy linting. Add CLI and JSON output. Enforce lints in tests for default policies and fail config loading on critical policy contradictions. Ensure high-risk/system-wide families cannot become autonomous through config while high-risk apply remains disabled.

---








PROPOSAL 33: Add real end-to-end “autotune dry-run daemon” mode

PRIORITY: HIGH
Users need a mode that exercises the full watcher stack without mutating system state.

CURRENT STATE:
`DaemonMode` has `Observe`, `Suggest`, and apply modes. Policy allows `Suggest` when mode is not observe and system-wide suggestions are permitted depending on policy. 
Planner can produce evaluations and summaries. 
High-risk dry-run is skipped by default in suggest mode. 

PROPOSED CHANGE:
Add mode or flag:

```bash
stutter autotune --mode suggest --dry-run-all-safe
```

Behavior:

* Run provider registry.
* Run static gates.
* Run dry-run for candidates whose safety class and effect scope allow dry-run under policy.
* Never start experiments.
* Output candidate plan files, summaries, dry-run affected tasks, and deny reasons.
* Include high-risk dry-run only if explicitly requested per PROPOSAL 26.

AFFECTED SCOPE:

* `stutter/src/cli.rs`
* `stutter/src/autotune/planner.rs`
* `stutter/src/autotune/runtime.rs`
* `stutter/src/autotune/status.rs`
* docs
  Medium feature addition.

DEPENDENCIES:

* Should follow PROPOSAL 26.
* Useful before medium-risk apply rollout.

EDIT REQUEST FOR PATCH WRITER:
Add an end-to-end dry-run daemon/suggest mode that exercises planning and safe dry-run without mutation. It must produce structured summaries and candidate plan files. It must never call live experiment start/apply. Add tests proving no rollback token is created and no action apply path runs.

---










PROPOSAL 1: Implement privileged worker subprocess lifecycle management in daemon startup
PRIORITY: CRITICAL
STATUS: Completed 2026-05-19.
The daemon silently fails to perform medium-risk apply because no code path starts the privileged worker subprocess before connecting to its socket.

CURRENT STATE:
`stutter/src/autotune/runtime.rs` lines 930–965 construct a `UnixSocketPrivilegedActionService::new(socket_path)` when `DaemonMode::ApplyMediumRisk` is selected and `unsafe_in_process_privileged_worker = false`. The socket path is computed via `default_privileged_worker_socket_path()` (returns `$XDG_RUNTIME_DIR/stutter-privileged-worker.sock`). The `UnixSocketPrivilegedActionService::send_request()` calls `UnixStream::connect(&self.socket_path)` at apply time — if the socket does not exist (worker not running), this returns `ConnectionRefused` which surfaces as an `anyhow::Error`. The daemon logs this as an action failure and the experiment is marked `ExperimentResult::ActionFailed`, triggering cooldown. The daemon recovers but the medium-risk apply is permanently broken for the session.

`stutter/src/daemon/privilege.rs:run_privileged_worker` is a synchronous blocking function: `UnixListener::bind` → loop accepting connections → `handle_privileged_worker_connection`. It cannot be called from within the async Tokio runtime thread without `spawn_blocking`. It is currently exposed only as a top-level CLI command (`stutter privileged-worker --socket <path>`) via `commands/daemon.rs` line 217.

`stutter/src/daemon/policy.rs`: `DaemonConfig` has `privileged_worker_socket: Option<PathBuf>` and `unsafe_in_process_privileged_worker: bool`. These are the only worker configuration fields. There is no `manage_privileged_worker: bool` field and no worker process handle type.

PROPOSED CHANGE:
Add a `PrivilegedWorkerHandle` struct to `daemon/privilege.rs`:

```rust
pub struct PrivilegedWorkerHandle {
    child: std::process::Child,
    socket_path: PathBuf,
    restart_count: u32,
}

impl PrivilegedWorkerHandle {
    pub fn spawn(socket_path: &Path) -> anyhow::Result<Self>;
    pub fn is_alive(&mut self) -> bool;
    pub fn restart(&mut self) -> anyhow::Result<()>;
    pub fn shutdown_gracefully(&mut self, timeout_ms: u64) -> anyhow::Result<()>;
}

impl Drop for PrivilegedWorkerHandle {
    fn drop(&mut self) { self.child.kill().ok(); }
}
```

`spawn` uses `std::process::Command::new(std::env::current_exe()?)` with args `["privileged-worker", "--socket", socket_path]` and `Stdio::null()` for stdin/stdout/stderr. After spawning, polls `socket_path.exists()` every 50ms for up to 2000ms. Returns error if socket never appears.

`restart` kills existing child, removes stale socket, calls spawn again, increments `restart_count`.

`is_alive` calls `child.try_wait()` → returns true if child is still running.

Add `manage_privileged_worker: bool` to `DaemonConfig`. Default: `true` when `mode == DaemonMode::ApplyMediumRisk && !unsafe_in_process_privileged_worker`.

In `autotune/runtime.rs`, in `run_autotune_controller_with_monitor`: before the event loop begins, if `manage_privileged_worker` is true, call `PrivilegedWorkerHandle::spawn(socket_path)` and store the handle. Add a health-check every 30 ticks: call `handle.is_alive()`; if false, call `handle.restart()` and log `privileged_worker_restarted`. Add `privileged_worker_restart_limit: u32` (default 3) to `DaemonConfig`. If `restart_count > limit` within a 5-minute window, transition to `DaemonPhase::Faulted`. On clean exit, call `handle.shutdown_gracefully(3000)` which sends `PrivilegedWorkerRequest::Shutdown` over socket then waits for child exit.

AFFECTED SCOPE:
- `stutter/src/daemon/privilege.rs` (add `PrivilegedWorkerHandle`)
- `stutter/src/daemon/policy.rs` (add `manage_privileged_worker`, `privileged_worker_restart_limit` to `DaemonConfig`)
- `stutter/src/autotune/runtime.rs` (spawn handle; health check; shutdown)
- `stutter/src/commands/daemon.rs` (pass config through)
- `stutter/tests/autotune_lifecycle.rs` (extend with medium-risk socket test)

DEPENDENCIES: None. Must be completed before Proposals 2, 3, 4, 8, 9.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/daemon/privilege.rs`, add `pub struct PrivilegedWorkerHandle { child: std::process::Child, socket_path: PathBuf, restart_count: u32 }`. Implement `pub fn spawn(socket_path: &Path) -> anyhow::Result<Self>` using `std::process::Command::new(std::env::current_exe()?)` with args `["privileged-worker", "--socket", socket_path.to_str().context("socket path is not UTF-8")?]`, stdin/stdout/stderr all `Stdio::null()`. After spawn, poll `socket_path.exists()` every 50ms up to 2000ms; return `Err` if socket never appears. Implement `pub fn is_alive(&mut self) -> bool` as `self.child.try_wait().map(|s| s.is_none()).unwrap_or(false)`. Implement `pub fn shutdown_gracefully(&mut self, timeout_ms: u64) -> anyhow::Result<()>`: attempt `UnixStream::connect` and send `PrivilegedWorkerRequest::Shutdown`, then `self.child.wait()` with a timeout fallback to `kill`. Implement `Drop` calling `self.child.kill().ok()`. In `stutter/src/daemon/policy.rs`, add `pub manage_privileged_worker: bool` defaulting to computed value and `pub privileged_worker_restart_limit: u32` defaulting to `3` on `DaemonConfig`. In `stutter/src/autotune/runtime.rs`, in `run_autotune_controller_with_monitor`, after computing `socket_path`, if `daemon_config.manage_privileged_worker` call `PrivilegedWorkerHandle::spawn(&socket_path)?` and store as `let mut worker_handle: Option<PrivilegedWorkerHandle>`. Every 30 event-loop iterations, if `handle.is_alive()` is false, call `handle.restart()`, increment a local counter, and if counter exceeds `restart_limit`, emit `DaemonRuntimeEvent::Fault("privileged_worker_crash_loop".into())`. Before returning, call `handle.shutdown_gracefully(3000)`.

---








PROPOSAL 2: Add IRQ device safety classification model and unlock `IrqAffinityRisk::ReversibleMediumRisk` for known-safe devices
PRIORITY: CRITICAL
IRQ affinity is the highest-impact system-wide tuning action and is permanently blocked by a hardcoded `HighRisk` assignment that ignores device identity.

CURRENT STATE:
`stutter/src/autotune/providers/irq_affinity.rs` line 77: `IrqAffinityRisk::HighRisk` is hardcoded in the `CandidateAction::IrqAffinity` construction regardless of `evidence_model.known_device_mapping`. `known_device_mapping` at line 151 is `stable_identity && !irq_line.kind.trim().is_empty()` — any IRQ with a non-empty kind field is considered "mapped", which is too coarse for safety tiers.

`stutter/src/irq_inspect.rs:IrqLine` has `kind: String` (e.g., `"PCI-MSI"`) and `name: String` (e.g., `"amdgpu"`, `"ahci"`, `"igc"`). No enum maps these to safety tiers.

`stutter/src/autotune/candidate.rs` line 200–210: `is_high_risk_system_adjacent()` returns `true` for the `IrqAffinity` variant unconditionally. `IrqAffinityRisk::ReversibleMediumRisk` exists and `actions/irq_affinity.rs` line 192 maps it to `SafetyClass::ReversibleMediumRisk` — the action-level distinction is complete; the provider and candidate layer are the gap.

`daemon/policy.rs` line 16: `HIGH_RISK_APPLY_IMPLEMENTED = false`. IRQ affinity with `ReversibleMediumRisk` maps to `SafetyClass::ReversibleMediumRisk` which is gated by `ApplyMediumRisk`, not `ApplyHighRisk` — this is the correct path and does not require flipping the constant.

PROPOSED CHANGE:
Add `IrqDeviceClass` enum to `irq_inspect.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrqDeviceClass {
    Gpu, DisplayController, Usb, Network,
    StorageController, Audio, Unknown, ExplicitHighRisk,
}

impl IrqDeviceClass {
    pub fn from_irq_name(name: &str) -> Self;
    pub fn default_risk(self) -> IrqAffinityRisk;
}
```

`from_irq_name` matches lowercase `name` substrings:
- `amdgpu|nvidia|i915|radeon|nouveau` → `Gpu`
- `xhci|uhci|ehci|ohci` → `Usb`
- `ahci|nvme|scsi|sata|mpt` → `StorageController`
- `igc|ixgbe|e1000|r8169|rtw|iwl|ath|brcm|mt76` → `Network`
- `snd|audio|hda|ac97` → `Audio`
- empty or unrecognized → `Unknown`

`default_risk` maps: `Gpu|Usb|Network|Audio` → `ReversibleMediumRisk`; `StorageController|Unknown|ExplicitHighRisk` → `HighRisk`.

In `autotune/providers/irq_affinity.rs`, replace line 77 hardcoded `IrqAffinityRisk::HighRisk` with `crate::irq_inspect::classify_irq_device(irq_line).default_risk()`.

In `autotune/candidate.rs`, modify `is_high_risk_system_adjacent()` for `IrqAffinity` arm to: `matches!(plan.action.risk, IrqAffinityRisk::HighRisk)`.

AFFECTED SCOPE:
- `stutter/src/irq_inspect.rs` (add `IrqDeviceClass` enum and `classify_irq_device`)
- `stutter/src/autotune/providers/irq_affinity.rs` (replace hardcoded `HighRisk`)
- `stutter/src/autotune/candidate.rs` (make `is_high_risk_system_adjacent()` conditional for IRQ)
- New planner fixture: `testdata/autotune/planner/game_irq_gpu_medium_risk.json`

DEPENDENCIES: Proposal 1 required. Must be complete before medium-risk apply is attempted.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/irq_inspect.rs`, add public enum `IrqDeviceClass` with variants `Gpu, Usb, Network, StorageController, Audio, Unknown, ExplicitHighRisk`. Implement `pub fn from_irq_name(name: &str) -> Self` matching on `name.to_ascii_lowercase()` for the substring patterns listed above. Implement `pub fn default_risk(self) -> crate::actions::irq_affinity::IrqAffinityRisk` mapping `Gpu|Usb|Network|Audio` → `ReversibleMediumRisk`, `StorageController|Unknown|ExplicitHighRisk` → `HighRisk`. Add `pub fn classify_irq_device(line: &IrqLine) -> IrqDeviceClass` calling `IrqDeviceClass::from_irq_name(&line.name)`. In `stutter/src/autotune/providers/irq_affinity.rs`, replace `IrqAffinityRisk::HighRisk` in the `CandidateAction::IrqAffinity` construction with `crate::irq_inspect::classify_irq_device(irq_line).default_risk()`. In `stutter/src/autotune/candidate.rs`, in `is_high_risk_system_adjacent()`, for the `Self::IrqAffinity { plan }` arm, return `matches!(plan.action.risk, IrqAffinityRisk::HighRisk)` instead of `true`. Add tests in `irq_inspect.rs` asserting `classify_irq_device` returns `Gpu` for name `"amdgpu"`, `StorageController` for `"ahci"`, `Unknown` for empty name. Add planner fixture `testdata/autotune/planner/game_irq_gpu_medium_risk.json` with a GPU IRQ signal and verify the resulting candidate is not denied with `ManualOnly`.

---








PROPOSAL 3: Lower GPU power profile switch and CPU EPP to `ReversibleMediumRisk`; unlock via `ApplyMediumRisk`
PRIORITY: HIGH
`pp_power_profile_mode` switches and EPP writes are structurally reversible and rollback paths exist; `HighRisk` classification is conservative policy blocking a safe and high-value autonomous operation.

CURRENT STATE:
`stutter/src/actions/gpu_power.rs` line 248: `fn safety_class(&self) -> SafetyClass { SafetyClass::HighRisk }` — unconditional for all GPU power operations including `pp_power_profile_mode` switches. The action has rollback via `GpuPowerRestoreRecord`. `GpuPowerPolicy::allow_power_profile_mode` exists as a gate (default `false`).

`stutter/src/actions/cpu_power.rs` line 186: `fn safety_class(&self) -> SafetyClass { SafetyClass::HighRisk }` — unconditional. `energy_performance_preference` writes are immediately reversible. `CpuPowerPolicy::allow_epp_changes` gates the operation.

`stutter/src/daemon/policy.rs` line 16: `HIGH_RISK_APPLY_IMPLEMENTED = false`. `ActionEffectScope::GpuPower` and `CpuPower` are in `ApplyHighRisk` allowed scopes only. Must add to `ApplyMediumRisk` for profile-switch operations.

`autotune/candidate.rs` line 200: `is_high_risk_system_adjacent()` returns `true` for `GpuPower` and `CpuPower` unconditionally.

PROPOSED CHANGE:
Modify `GpuPowerAction::safety_class()`:
```rust
fn safety_class(&self) -> SafetyClass {
    if self.pp_power_profile_mode.is_some() && self.power_dpm_force_performance_level.is_none() {
        SafetyClass::ReversibleMediumRisk
    } else {
        SafetyClass::HighRisk
    }
}
```
Profile-only switch → medium risk. DPM level change → high risk.

Modify `CpuPowerAction::safety_class()`:
```rust
fn safety_class(&self) -> SafetyClass {
    if self.energy_performance_preference.is_some() && self.scaling_governor.is_none() {
        SafetyClass::ReversibleMediumRisk
    } else {
        SafetyClass::HighRisk
    }
}
```
EPP-only write → medium risk. Governor change → high risk.

In `daemon/policy.rs`, add `ActionEffectScope::GpuPower` and `ActionEffectScope::CpuPower` to `ApplyMediumRisk` allowed effect scopes. Add `DaemonConfig` field `pub allow_gpu_power_in_autotune: bool` (default `false`). Add check in `DaemonPolicy::check_action` rejecting `GpuPower`/`CpuPower` unless `allow_gpu_power_in_autotune`.

In `autotune/candidate.rs`, modify `is_high_risk_system_adjacent()` for `GpuPower` and `CpuPower` arms to delegate to `self.safety_class() == SafetyClass::HighRisk`.

In `autotune/providers/gpu_power.rs`, limit all proposals to `pp_power_profile_mode` only (no DPM level change), ensuring they qualify for `ReversibleMediumRisk`.

AFFECTED SCOPE:
- `stutter/src/actions/gpu_power.rs` (conditional `safety_class()`)
- `stutter/src/actions/cpu_power.rs` (conditional `safety_class()`)
- `stutter/src/daemon/policy.rs` (add scopes to `ApplyMediumRisk`; add `allow_gpu_power_in_autotune`)
- `stutter/src/autotune/candidate.rs` (conditional `is_high_risk_system_adjacent()`)
- `stutter/src/autotune/providers/gpu_power.rs` (profile-switch-only proposals)
- `stutter/src/autotune/providers/cpu_power.rs` (EPP-only proposals)
- New planner fixture: `testdata/autotune/planner/game_gpu_profile_switch_medium_risk.json`

DEPENDENCIES: Proposal 1 required. Proposal 2 recommended first (establishes the conditional-risk pattern in `candidate.rs`).

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/actions/gpu_power.rs`, replace the unconditional `SafetyClass::HighRisk` return in `safety_class()` with: return `SafetyClass::ReversibleMediumRisk` if `self.pp_power_profile_mode.is_some() && self.power_dpm_force_performance_level.is_none()`, else `SafetyClass::HighRisk`. In `stutter/src/actions/cpu_power.rs`, same change: `SafetyClass::ReversibleMediumRisk` if `self.energy_performance_preference.is_some() && self.scaling_governor.is_none()`, else `SafetyClass::HighRisk`. In `stutter/src/autotune/candidate.rs`, for `Self::GpuPower { plan }` and `Self::CpuPower { plan }` arms in `is_high_risk_system_adjacent()`, return `matches!(plan.action.safety_class(), SafetyClass::HighRisk)`. In `stutter/src/daemon/policy.rs`, add `ActionEffectScope::GpuPower` and `ActionEffectScope::CpuPower` to the `ApplyMediumRisk` allowed effect scope set. Add `pub allow_gpu_power_in_autotune: bool` to `DaemonConfig` with default `false`. In `DaemonPolicy::check_action`, reject these scopes unless `allow_gpu_power_in_autotune`. In `stutter/src/autotune/providers/gpu_power.rs`, ensure all proposals have `power_dpm_force_performance_level: None` set. Add unit tests for the new conditional `safety_class()` in both action files. Add planner fixture `game_gpu_profile_switch_medium_risk.json`.

---








PROPOSAL 4: Unlock `vm.swappiness` for autonomous apply at `ReversibleMediumRisk`
PRIORITY: HIGH
`vm.swappiness` lowered to 10 is a well-understood, reversible, rollback-safe operation with a concrete trigger and safe-value list already defined; the only blockers are `manual_only: true` and `HighRisk` in the policy struct.

CURRENT STATE:
`stutter/src/autotune/providers/vm_knob.rs` line 172–180: the `vm.swappiness` policy has `safety_class: SafetyClass::HighRisk` and `manual_only: true` hardcoded. The `VmKnobPolicy.manual_only` check at line 132 prevents the provider from returning a candidate. `safe_values: vec!["10".to_owned()]` and `trigger: VmKnobTrigger::SwapPressure` are already correctly defined.

`actions/vm_knobs.rs:VmKnobAction::safety_class()` returns the value from the action's `safety_class` field, populated from `evidence_model.safety_class` via the provider. The action reads and restores the original value via `VmKnobRestoreRecord`. Rollback is structural.

`autotune/candidate.rs` line 200: `is_high_risk_system_adjacent()` returns `true` for `VmKnob` unconditionally.

`daemon/policy.rs`: `ActionEffectScope::VmKnob` is in `ApplyHighRisk` allowed scopes only.

PROPOSED CHANGE:
In `autotune/providers/vm_knob.rs`, change `vm.swappiness` policy to `safety_class: SafetyClass::ReversibleMediumRisk` and `manual_only: false`. Leave `vm.dirty_background_ratio` and `vm.dirty_ratio` as `HighRisk` and `manual_only: true`.

Add value bounds guard in `actions/vm_knobs.rs::VmKnobAction::apply_at()`: reject writes not in `safe_values` list. For `vm.swappiness`, additionally reject writes outside `[1, 60]`.

In `daemon/policy.rs`, add `ActionEffectScope::VmKnob` to `ApplyMediumRisk` allowed effect scopes. Add `pub allow_vm_knobs_in_autotune: bool` (default `false`). Check in `DaemonPolicy::check_action`.

In `autotune/candidate.rs`, modify `is_high_risk_system_adjacent()` for `VmKnob` to return `matches!(plan.action.safety_class(), SafetyClass::HighRisk)`.

AFFECTED SCOPE:
- `stutter/src/autotune/providers/vm_knob.rs` (change swappiness policy)
- `stutter/src/actions/vm_knobs.rs` (add value bounds guard)
- `stutter/src/daemon/policy.rs` (add VmKnob to ApplyMediumRisk; add config field)
- `stutter/src/autotune/candidate.rs` (conditional `is_high_risk_system_adjacent()` for VmKnob)
- New planner fixture: `testdata/autotune/planner/memory_pressure_swappiness_medium_risk.json`

DEPENDENCIES: Proposal 1 required. Proposal 6 (PSI delta) recommended before this — `mem_stall_spike_count` from Proposal 6 improves trigger evidence quality.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/autotune/providers/vm_knob.rs`, in `vm_knob_policies()`, change the `vm.swappiness` entry to `safety_class: SafetyClass::ReversibleMediumRisk` and `manual_only: false`. Do not change the other two entries. In `stutter/src/actions/vm_knobs.rs`, in the apply method, before writing the value, check: if the proposed value string is not in `action.safe_values`, return `Err(anyhow::anyhow!("vm knob write refused: {} not in safe_values", value))`. For `vm.swappiness` specifically, parse the value as `u32` and reject if outside `[1, 60]`. In `stutter/src/autotune/candidate.rs`, for `Self::VmKnob { plan }` in `is_high_risk_system_adjacent()`, return `matches!(plan.action.safety_class(), SafetyClass::HighRisk)`. In `stutter/src/daemon/policy.rs`, add `ActionEffectScope::VmKnob` to `ApplyMediumRisk` allowed scopes. Add `pub allow_vm_knobs_in_autotune: bool` to `DaemonConfig` defaulting to `false`. Add rejection in `DaemonPolicy::check_action`. Add planner fixture `testdata/autotune/planner/memory_pressure_swappiness_medium_risk.json` with `swap_activity_events` set and verify candidate is not `ManualOnly`.

---








PROPOSAL 5: Implement `HyprlandForegroundProvider` and wire into `auto_foreground_provider()`
PRIORITY: HIGH
Hyprland is the most rapidly growing Wayland compositor after Sway; Hyprland sessions currently fall through to `UnsupportedForegroundProvider` despite a complete JSON parser and tests existing.

CURRENT STATE:
`stutter/src/foreground.rs` lines 303–365: `struct HyprlandActiveWindow`, `struct HyprlandWorkspace`, and `fn hyprland_snapshot_from_activewindow_json` all carry `#[allow(dead_code)]`. Test `parse_hyprland_activewindow_extracts_pid_class_workspace` at line 1180 passes.

`auto_foreground_provider()` lines 253–272: checks `SwayForegroundProvider::is_detected()` then `is_generic_wayland_without_supported_foreground_api()` (returns `false` for Hyprland due to `HYPRLAND_INSTANCE_SIGNATURE` check) then `X11ForegroundProvider::is_detected()` then falls through to generic unsupported. Hyprland sessions reach `UnsupportedForegroundProvider`.

`SwayForegroundProvider` at line 367 is the reference: `is_detected()` checks env var, `sample()` runs shell command, parses JSON via `sample_from_tree_json()`. The Hyprland equivalent is `hyprctl activewindow -j`.

PROPOSED CHANGE:
Add `pub struct HyprlandForegroundProvider { hyprctl: String }` with `Default`, `new()`, `with_hyprctl()`, `pub fn is_detected() -> bool { std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() }`, and `impl ForegroundProvider` with `sample()` running `hyprctl activewindow -j` and calling `hyprland_snapshot_from_activewindow_json`.

In `auto_foreground_provider()`, add `if HyprlandForegroundProvider::is_detected() { return Box::new(HyprlandForegroundProvider::new()); }` between the Sway check and the generic Wayland check.

Remove `#[allow(dead_code)]` from `HyprlandActiveWindow`, `HyprlandWorkspace`, and `hyprland_snapshot_from_activewindow_json`.

AFFECTED SCOPE:
- `stutter/src/foreground.rs` only. Fully self-contained.

DEPENDENCIES: None. Can be implemented independently at any point.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/foreground.rs`, after the `SwayForegroundProvider` impl block, add `pub struct HyprlandForegroundProvider { hyprctl: String }`. Implement `Default` calling `Self::new()`. Implement `pub fn new() -> Self { Self { hyprctl: "hyprctl".to_owned() } }`, `pub fn with_hyprctl(mut self, h: impl Into<String>) -> Self { self.hyprctl = h.into(); self }`, `pub fn is_detected() -> bool { std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() }`. Implement `ForegroundProvider for HyprlandForegroundProvider`: `sample()` runs `std::process::Command::new(&self.hyprctl).args(["activewindow", "-j"]).output()`, measures elapsed_ms, on success calls `hyprland_snapshot_from_activewindow_json(elapsed_ms, &String::from_utf8_lossy(&out.stdout))`, on failure returns `ForegroundWindowSnapshot::unavailable(elapsed_ms, format!("hyprctl activewindow failed: {err}"))`. Make `hyprland_snapshot_from_activewindow_json` `pub(crate)` and remove its `#[allow(dead_code)]`. Remove `#[allow(dead_code)]` from `HyprlandActiveWindow` and `HyprlandWorkspace`. In `auto_foreground_provider()`, add `if HyprlandForegroundProvider::is_detected() { return Box::new(HyprlandForegroundProvider::new()); }` immediately after the `SwayForegroundProvider::is_detected()` block. Add 3 tests: one for `is_detected()` returning true when env var is set, one for `sample()` using `with_hyprctl(echo_command_that_outputs_known_json)`, one for `auto_foreground_provider()` returning Hyprland provider when env var is set.

---








PROPOSAL 6: Add per-interval PSI delta tracking and memory stall spike detection
PRIORITY: HIGH
The current `PsiReader` stores only avg10 per interval, preventing detection of brief but severe memory stall events that cause latency spikes without sustained background pressure.

CURRENT STATE:
`stutter/src/psi.rs:PsiReader::read()` returns `PsiSnapshot` with avg10 and monotonic `total_us` fields. `PsiReader` has no `prev_snapshot` field. At each interval (`session.rs` line 1491), `psi_reader.read().ok()` is called and only avg10 values are stored in `IntervalRecord`. The monotonic `total_us` fields are discarded. `rolling_window.rs` line 345 computes `memory_pressure_some_avg10_percent` by averaging `mem_psi_some` across all intervals — a doubly smoothed trailing average.

`IntervalRecord` in `metrics.rs` has `cpu_psi_some: f64`, `mem_psi_some: f64`, `mem_psi_full: f64`, `io_psi_some: f64`, `io_psi_full: f64`. No `mem_psi_delta_us: u64` or `mem_psi_spike: bool`.

PROPOSED CHANGE:
Add `prev_snapshot: Option<PsiSnapshot>` to `PsiReader`. Add `pub struct PsiDelta { pub snapshot: PsiSnapshot, pub mem_stall_delta_us: Option<u64>, pub cpu_stall_delta_us: Option<u64>, pub io_stall_delta_us: Option<u64>, pub mem_stall_spike: bool }` with `const MEM_STALL_SPIKE_THRESHOLD_US: u64 = 100_000`.

Add `pub fn read_with_delta(&mut self) -> anyhow::Result<PsiDelta>`: call `self.read()`, if `prev_snapshot` is set compute `mem_stall_delta_us = snapshot.mem_some_total_us.saturating_sub(prev.mem_some_total_us)`, set `mem_stall_spike = delta > THRESHOLD`, update `self.prev_snapshot`, return `PsiDelta`.

Add `pub mem_psi_delta_us: u64` and `pub mem_psi_spike: bool` to `IntervalRecord` in `metrics.rs`. Change `session.rs` to call `psi_reader.read_with_delta()`. Populate new fields.

In `rolling_window.rs`, add `mem_stall_spike_count: u64` accumulation from `IntervalRecord.mem_psi_spike`. Expose in `ObjectiveSignals` as `pub mem_stall_spike_count: Option<u64>`.

In `autotune/providers/vm_knob.rs`, add `mem_stall_spike_count` as secondary trigger for `vm.swappiness` candidate.

AFFECTED SCOPE:
- `stutter/src/psi.rs` (add `PsiDelta`, `read_with_delta`, `prev_snapshot`)
- `stutter/src/metrics.rs` (add fields to `IntervalRecord`)
- `stutter/src/session.rs` (change to `read_with_delta`)
- `stutter/src/autotune/rolling_window.rs` (add `mem_stall_spike_count` to `ObjectiveSignals`)
- `stutter/src/autotune/providers/vm_knob.rs` (use `mem_stall_spike_count`)

DEPENDENCIES: None. Self-contained. Do before Proposal 4 for best signal quality.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/psi.rs`, add `prev_snapshot: Option<PsiSnapshot>` to `PsiReader`. Add `pub struct PsiDelta { pub snapshot: PsiSnapshot, pub mem_stall_delta_us: Option<u64>, pub cpu_stall_delta_us: Option<u64>, pub io_stall_delta_us: Option<u64>, pub mem_stall_spike: bool }` and `const MEM_STALL_SPIKE_THRESHOLD_US: u64 = 100_000`. Implement `pub fn read_with_delta(&mut self) -> anyhow::Result<PsiDelta>`: call `self.read()`, if `self.prev_snapshot.is_some()` compute monotonic deltas via `saturating_sub`, set `mem_stall_spike = mem_stall_delta_us.unwrap_or(0) > MEM_STALL_SPIKE_THRESHOLD_US`, store `self.prev_snapshot = Some(snapshot.clone())`, return. In `stutter/src/metrics.rs`, add `pub mem_psi_delta_us: u64` and `pub mem_psi_spike: bool` to `IntervalRecord`. In `stutter/src/session.rs`, change `self.runtime.probes.psi_reader.read().ok()` to `self.runtime.probes.psi_reader.read_with_delta().ok()` and populate both new fields when constructing `IntervalRecord`. In `stutter/src/autotune/rolling_window.rs`, accumulate `mem_stall_spike_count` by summing `interval.mem_psi_spike as u64` across `self.intervals`. Add `pub mem_stall_spike_count: Option<u64>` to `ObjectiveSignals` and populate it. Add a unit test in `psi.rs` verifying that two `read_with_delta` calls with increasing `mem_some_total_us` produce the correct delta and correct spike classification.

---








PROPOSAL 7: Implement workload activity state machine for idle-suppression in the autotune planner
PRIORITY: HIGH
Without a per-moment activity model, the planner may select tuning candidates for a game on a pause menu or browser between tabs, wasting experiment window time on idle workloads.

CURRENT STATE:
`stutter/src/focus/score.rs:score_focus_group()` computes `cpu_score`, `io_score`, and `interactivity_score` as per-tick scalars with no history. `RollingWindow` has per-interval `scored_samples` and `frame_count` VecDeques but does not track their recent rate of change. `situation.rs:classify_situation()` transitions from `GameFocused` to `GameCpuSchedulerPressure` only when `StutterCause` diagnosis is present — a game idle on a pause menu stays `GameFocused` indefinitely. No activity gate exists in the planner.

PROPOSED CHANGE:
Create `stutter/src/autotune/activity.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityLevel { Active, SlowingDown, Idle }

pub struct ActivityClassifier {
    window: VecDeque<u64>,  // scored_samples per interval
    window_size: usize,
}

impl ActivityClassifier {
    pub fn new(window_size: usize) -> Self;
    pub fn push_interval(&mut self, scored_samples: u64);
    pub fn classify(&self) -> ActivityLevel;
}
```

`classify()`: fewer than 3 samples → `Active`. Last 3 all zero → `Idle`. Last 3 monotonically decreasing and peak-to-last drop > 50% → `SlowingDown`. Otherwise → `Active`.

Add `ActivityClassifier` field to `AutotuneRuntime`. Push on each `MonitorEvent::Interval`. Add `activity_level: ActivityLevel` to `AutotuneObservation`. Populate in `AutotuneObservationBuilder`.

In `autotune/planner.rs`, add guard: if `activity_level == Idle && !matches!(situation, SituationKind::Idle)` → push `CandidateDenyReason::WorkloadIdle`, skip candidate selection.

Add `CandidateDenyReason::WorkloadIdle` to `candidate.rs`.

AFFECTED SCOPE:
- New: `stutter/src/autotune/activity.rs`
- `stutter/src/autotune/mod.rs` (add module)
- `stutter/src/autotune/runtime.rs` (add classifier field; push on interval)
- `stutter/src/autotune/observation.rs` (add `activity_level`)
- `stutter/src/autotune/observation_builder.rs` (populate `activity_level`)
- `stutter/src/autotune/planner.rs` (add activity gate)
- `stutter/src/autotune/candidate.rs` (add `WorkloadIdle` deny reason)
- New planner fixture: `testdata/autotune/planner/game_idle_suppressed.json`

DEPENDENCIES: None. Recommend implementing after Proposals 2–4 so the gate is tested against real unlocked candidates.

EDIT REQUEST FOR PATCH WRITER:
Create `stutter/src/autotune/activity.rs`. Define `pub enum ActivityLevel { Active, SlowingDown, Idle }` with `Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq`. Define `pub struct ActivityClassifier { window: std::collections::VecDeque<u64>, window_size: usize }`. Implement `pub fn new(window_size: usize) -> Self`, `pub fn push_interval(&mut self, scored_samples: u64)` (push, pop front if len > window_size), `pub fn classify(&self) -> ActivityLevel` with the three-tier logic described above. Add `pub mod activity;` to `stutter/src/autotune/mod.rs`. In `stutter/src/autotune/runtime.rs`, add `activity_classifier: ActivityClassifier::new(5)` to `AutotuneRuntime`. In `on_event(MonitorEvent::Interval { records, .. })`, compute total `scored_samples` across records and call `self.activity_classifier.push_interval(total)`. In `stutter/src/autotune/observation.rs`, add `pub activity_level: ActivityLevel`. In `stutter/src/autotune/observation_builder.rs`, populate it from `self.runtime.activity_classifier.classify()`. In `stutter/src/autotune/candidate.rs`, add `WorkloadIdle` to `CandidateDenyReason`. In `stutter/src/autotune/planner.rs`, before candidate evaluation, if `observation.activity_level == ActivityLevel::Idle && !matches!(observation.primary_situation, SituationKind::Idle)`, produce a deny entry with `WorkloadIdle` for all providers and return early. Add tests in `activity.rs` for all three `classify()` outcomes. Add planner fixture `testdata/autotune/planner/game_idle_suppressed.json` with zero `scored_samples` across intervals and verify all candidates carry `WorkloadIdle` deny.

---








PROPOSAL 8: Integration tests for all newly unlocked autonomous apply paths
PRIORITY: CRITICAL
The lifecycle integration test covers only low-risk apply; the three newly unlocked action families and the activity suppression gate each require automated test coverage before being considered production-safe.

CURRENT STATE:
`stutter/tests/autotune_lifecycle.rs` (196 lines) covers `Observing → Measuring → Keep → Rollback` with `SafetyClass::ReversibleLowRisk` and `simulate_action_effects: true`. No test exercises: medium-risk through Unix socket; `IrqAffinityRisk::ReversibleMediumRisk`; GPU profile switch; VM swappiness; activity idle suppression.

PROPOSED CHANGE:
Extend `stutter/tests/autotune_lifecycle.rs` with 5 additional tests:

**Test 1: `medium_risk_apply_through_unix_socket_lifecycle`**
Spawn `run_privileged_worker_with_service` in `std::thread::spawn` with temp socket path. Drive `AutotuneRuntime` in `ApplyMediumRisk` mode with `CandidateAction::Fake { safety_class: ReversibleMediumRisk }`. Assert: experiment reaches `kept`; journal is clean; worker thread exits cleanly on shutdown.

**Test 2: `irq_affinity_gpu_device_is_medium_risk_not_manual_only`**
Construct `CandidateAction::IrqAffinity` with `IrqAffinityRisk::ReversibleMediumRisk`. Assert: `is_high_risk_system_adjacent() == false`; `manual_only_reason() == None`; `safety_class() == ReversibleMediumRisk`.

**Test 3: `gpu_power_profile_switch_only_is_medium_risk`**
Construct `GpuPowerAction { pp_power_profile_mode: Some("3"), power_dpm_force_performance_level: None, .. }`. Assert `safety_class() == ReversibleMediumRisk`. Construct `CandidateAction::GpuPower`. Assert `is_high_risk_system_adjacent() == false`.

**Test 4: `vm_swappiness_provider_produces_medium_risk_non_manual_candidate`**
Construct `CandidateProviderInput` with `swap_activity_events: Some(100)`. Run `VmKnobProvider::propose()`. Assert at least one proposal has `manual_only: false`, `safety_class: ReversibleMediumRisk`, and knob `"vm.swappiness"`.

**Test 5: `activity_classifier_suppresses_idle_game_candidates`**
Drive `AutotuneRuntime` with 6 `Interval { scored_samples: 0 }` steps preceded by `FocusGame { confidence: 0.9 }`. Assert: no `candidate_started` decision; `WorkloadIdle` appears in deny reasons.

AFFECTED SCOPE:
- `stutter/tests/autotune_lifecycle.rs` (extend with 5 tests)
- `stutter/src/test_fixture_builder.rs` (add helpers for `IrqAffinityActionPlan` and `GpuPowerActionPlan` construction if needed)

DEPENDENCIES: Proposals 1, 2, 3, 4, 7 must be complete for tests to pass. Tests 2–5 can be written as failing tests before proposals to drive development.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/tests/autotune_lifecycle.rs`, add 5 new test functions. Test 1 must use `std::thread::spawn` to run `stutter::daemon::privilege::run_privileged_worker_with_service` with `InProcessPrivilegedActionService::default()` and a `TempDir` socket path; the main test drives the async runtime. Test 2 directly constructs `CandidateAction::IrqAffinity` with `IrqAffinityRisk::ReversibleMediumRisk` and asserts properties without involving the runtime. Tests 3 and 4 directly construct action types and assert properties. Test 5 uses the `FakeDaemonScenario` infrastructure from `stutter::autotune::simulation` with 6 zero-sample interval steps plus a `FocusGame` step and asserts decisions contain no `candidate_started` and contain `WorkloadIdle` deny reasons.

---








PROPOSAL 9: Privileged worker socket IPC integration test in isolation
PRIORITY: CRITICAL
A dedicated socket IPC test is needed because the worker subprocess lifecycle introduces failure modes (socket not ready, worker crash mid-apply) that in-process tests cannot exercise.

CURRENT STATE:
No test performs a `UnixStream::connect` → `PrivilegedWorkerRequest::Apply` → `PrivilegedWorkerResponse::Apply` round trip. `handle_privileged_worker_connection` is only exercised implicitly through `unsafe_in_process_privileged_worker = true` in simulation.

PROPOSED CHANGE:
Add `stutter/tests/privileged_worker_socket.rs` with 3 tests:

**Test 1: `worker_socket_apply_and_rollback_roundtrip`**
Spawn `run_privileged_worker_with_service` in a thread with `InProcessPrivilegedActionService` and temp socket. Create `UnixSocketPrivilegedActionService`. Call `plan_candidate` → `apply_candidate` → `rollback`. Assert each step returns `Ok`. Send shutdown. Join thread.

**Test 2: `worker_connection_refused_surfaces_as_error`**
Call `plan_candidate` on `UnixSocketPrivilegedActionService::new("/nonexistent/path.sock")`. Assert `Err` with message containing `"failed to connect"`.

**Test 3: `worker_handle_restart_recovers_connectivity`**
Start worker via `PrivilegedWorkerHandle::spawn()`. Kill child. Assert `!is_alive()`. Call `restart()`. Assert `is_alive()`. Complete a `plan_candidate` round trip successfully.

AFFECTED SCOPE:
- New: `stutter/tests/privileged_worker_socket.rs`

DEPENDENCIES: Proposal 1 (`PrivilegedWorkerHandle`) must be complete.

EDIT REQUEST FOR PATCH WRITER:
Create `stutter/tests/privileged_worker_socket.rs`. Import `stutter::daemon::privilege::{run_privileged_worker_with_service, UnixSocketPrivilegedActionService, PrivilegedWorkerHandle, InProcessPrivilegedActionService, CandidatePlanRequest}`. Add 3 tests as described. Test 1 uses `std::thread::spawn` for the worker; main thread uses `UnixSocketPrivilegedActionService`; asserts plan/apply/rollback sequence succeeds; sends `PrivilegedWorkerRequest::Shutdown` and joins. Test 2 directly calls `plan_candidate` on a nonexistent socket path and asserts `Err`. Test 3 calls `PrivilegedWorkerHandle::spawn`, kills via `handle.child.kill()`, asserts `!handle.is_alive()`, calls `handle.restart()`, asserts `handle.is_alive()`, completes a `plan_candidate` call successfully.

---








PROPOSAL 10: Daemon startup error diagnostics for medium-risk configuration problems
PRIORITY: MEDIUM
STATUS: Completed 2026-05-19.
When `ApplyMediumRisk` is configured without the worker running, the daemon fails silently at apply time rather than reporting misconfiguration at startup.

CURRENT STATE:
`commands/daemon.rs` daemon startup does not validate Unix socket reachability when `ApplyMediumRisk` mode is selected and `manage_privileged_worker = false`. First apply fails with `Connection refused` logged as a warning. `stutter daemon status` does not report worker connectivity. `stutter daemon doctor` does not check socket reachability.

PROPOSED CHANGE:
In daemon startup, after config construction, if `mode == ApplyMediumRisk && !unsafe_in_process_privileged_worker && !manage_privileged_worker`: check socket existence and print a warning to stderr and log at `warn!` level with the manual start command.

Add `privileged_worker_socket_reachable: Option<bool>` to `DaemonCapabilities`. Populate by checking `socket_path.exists()` in `assess_daemon_capabilities`. Surface in `stutter daemon status` and `stutter daemon doctor` output.

Add structured log lines in `PrivilegedWorkerHandle::spawn()`: `privileged_worker_started socket=... pid=...`. In `restart()`: `privileged_worker_restarted socket=... restart_count=... pid=...`.

AFFECTED SCOPE:
- `stutter/src/commands/daemon.rs` (startup warning)
- `stutter/src/daemon/policy.rs` (add `privileged_worker_socket_reachable` to `DaemonCapabilities`)
- `stutter/src/daemon/privilege.rs` (add structured log in `spawn` and `restart`)

DEPENDENCIES: Proposal 1 (`PrivilegedWorkerHandle`) must be complete.

EDIT REQUEST FOR PATCH WRITER:
In `stutter/src/commands/daemon.rs`, in the main daemon startup function after config construction, add: if `daemon_config.mode == DaemonMode::ApplyMediumRisk && !daemon_config.autotune.unsafe_in_process_privileged_worker && !daemon_config.manage_privileged_worker`, call `default_privileged_worker_socket_path()` and if path does not exist, emit `eprintln!` with the manual `stutter privileged-worker --socket <path>` command and `log::warn!` at the same message. In `stutter/src/daemon/policy.rs`, add `pub privileged_worker_socket_reachable: Option<bool>` to `DaemonCapabilities` and populate in `assess_daemon_capabilities` by checking socket path existence. In `stutter/src/daemon/privilege.rs`, in `PrivilegedWorkerHandle::spawn()` after socket appears, emit `log::info!("privileged_worker_started socket={} pid={}", socket_path.display(), child.id())`. In `restart()`, emit `log::info!("privileged_worker_restarted socket={} restart_count={} pid={}", socket_path.display(), self.restart_count, child.id())`.

---








PROPOSAL 11: Production Gentoo ebuild with stable eBPF object build path
PRIORITY: HIGH
The ebuild is explicitly marked skeleton-only; the eBPF build path using nightly and `-Z build-std` is incompatible with offline Portage vendoring.

CURRENT STATE:
`packaging/gentoo/stutter-9999.ebuild`:
- `EGIT_BRANCH="review/big-daemon-packaging-work"` — non-main branch
- `KEYWORDS=""` — unkeworded
- `src_compile() { export RUSTC_BOOTSTRAP=1; RUSTUP_TOOLCHAIN=nightly cargo_src_compile -p stutter }` — fails in Portage sandbox
- `dev-util/bpf-linker` in `BDEPEND` — not in main Gentoo tree
- No `src_test()`, no bash completion, no man page, no udev rule

eBPF build requires `RUSTUP_TOOLCHAIN=nightly cargo build --target bpfel-unknown-none -Z build-std=core`. Incompatible with offline `cargo.eclass`.

PROPOSED CHANGE:
**Phase A — Prebuilt eBPF object support in build system:**
Add to `stutter/build.rs` (or `stutter-ebpf/build.rs`):
```rust
if let Ok(path) = std::env::var("STUTTER_PREBUILT_BPF_OBJECT") {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("stutter.bpf.o");
    std::fs::copy(&path, &out)?;
    println!("cargo:rerun-if-env-changed=STUTTER_PREBUILT_BPF_OBJECT");
} else {
    // existing aya-build path
}
```

**Phase B — CI release workflow:**
Create `.github/workflows/release.yml`: on tag push, install nightly + rust-src + bpf-linker, build eBPF object, upload as `stutter-${VERSION}.bpf.o` release asset.

**Phase C — Updated ebuild:**
```bash
SRC_URI="https://github.com/P2949/stutter/releases/download/${PV}/stutter-${PV}.bpf.o"
KEYWORDS="~amd64"
EGIT_BRANCH="main"

src_configure() {
    export STUTTER_PREBUILT_BPF_OBJECT="${DISTDIR}/stutter-${PV}.bpf.o"
}

src_compile() {
    cargo_src_compile -p stutter
}

src_test() {
    cargo_src_test -p stutter
}
```

Update `stutter/Cargo.toml` `version` to `0.2.0`. Create `CHANGELOG.md`.

AFFECTED SCOPE:
- `stutter/build.rs` or `stutter-ebpf/build.rs` (prebuilt object support)
- `.github/workflows/release.yml` (new)
- `packaging/gentoo/stutter-9999.ebuild` (major update)
- `stutter/Cargo.toml` (version bump)
- New: `CHANGELOG.md`

DEPENDENCIES: All other proposals must be merged before tagging a release. Build system change (Phase A) can be drafted in parallel.

EDIT REQUEST FOR PATCH WRITER:
In the `stutter-ebpf` crate, add `build.rs` if absent or modify existing. Add: if env var `STUTTER_PREBUILT_BPF_OBJECT` is set, copy the file at that path to `$OUT_DIR/stutter.bpf.o` and emit `cargo:rerun-if-env-changed=STUTTER_PREBUILT_BPF_OBJECT`, skipping the aya-build compilation. Create `.github/workflows/release.yml` with a job triggered `on.push.tags: ['v*']` that: checks out repository, installs nightly Rust with `rust-src` component via `dtolnay/rust-toolchain`, installs `bpf-linker` via `cargo install`, runs `RUSTUP_TOOLCHAIN=nightly cargo build -p stutter-ebpf --target bpfel-unknown-none -Z build-std=core --release`, and uploads `target/bpfel-unknown-none/release/stutter-ebpf` as a release asset named `stutter-${GITHUB_REF_NAME}.bpf.o` using `actions/upload-release-asset`. In `packaging/gentoo/stutter-9999.ebuild`, replace the full `src_compile()` block with `cargo_src_compile -p stutter`, remove `RUSTC_BOOTSTRAP=1` and `RUSTUP_TOOLCHAIN=nightly`, add `src_configure()` exporting `STUTTER_PREBUILT_BPF_OBJECT`, add `SRC_URI` for the prebuilt object, change `EGIT_BRANCH` to `"main"`, set `KEYWORDS="~amd64"`, add `src_test()` calling `cargo_src_test -p stutter`. Update `stutter/Cargo.toml` version to `"0.2.0"`. Create `CHANGELOG.md` with a `## [0.2.0] — Production release` section listing all implemented proposals.

















# 0. Shared foundation before implementing any probe

STATUS: Completed 2026-05-19.

Do this first. Do **not** start by writing eBPF tracepoint code.

## 0.1 Add explicit names for the new measurement families

Add these probe keys to `stutter/src/probe_registry.rs`:

```rust
KmsPageflipTiming,
WaylandPresentationTiming,
DisplayPathCost,
```

Keep the existing:

```rust
DrmFenceLatency
```

Do **not** create a second fence probe. The current registry already has `DrmFenceLatency` as a planned probe answering whether GPU queue/fence delay caused frame stutter. 

Recommended probe mapping:

| Feature                       | Probe key                   | Type                                                   |
| ----------------------------- | --------------------------- | ------------------------------------------------------ |
| DRM/KMS pageflip timing       | `KmsPageflipTiming`         | eBPF/tracepoint                                        |
| dGPU → iGPU fence/copy timing | existing `DrmFenceLatency`  | eBPF/tracepoint                                        |
| Wayland presentation timing   | `WaylandPresentationTiming` | external log / Wayland client / compositor cooperation |
| UHD630 display-cable cost     | `DisplayPathCost`           | view-only comparison                                   |

## 0.2 Add CLI flags, but make them no-op first

Add fields to `ProbeConfig` in `stutter/src/config/model.rs`:

```rust
pub kms_timing: bool,
pub drm_fence_latency: bool,
pub wayland_presentation: bool,
```

Add supporting config structs:

```rust
pub struct KmsTimingConfig {
    pub drm_card: Option<String>,      // "card0", "card1"
    pub connector: Option<String>,     // "DP-1", "HDMI-A-1"
    pub crtc: Option<u32>,
}

pub struct DrmFenceConfig {
    pub render_card: Option<String>,   // likely amdgpu
    pub display_card: Option<String>,  // likely i915
    pub driver_filter: Option<String>, // "amdgpu", "i915", "auto"
}

pub struct WaylandPresentationConfig {
    pub log_path: Option<PathBuf>,
    pub source: WaylandPresentationSource,
}
```

Add CLI flags in `stutter/src/cli/monitor.rs`:

```text
--kms-timing
--kms-card <cardN>
--kms-connector <NAME>
--kms-crtc <ID>

--drm-fence-latency
--drm-fence-render-card <cardN>
--drm-fence-display-card <cardN>
--drm-fence-driver <amdgpu|i915|auto>

--wayland-presentation
--wayland-presentation-log <PATH>
--wayland-presentation-source <external-log|gamescope|self-test>
```

At this stage, the flags should parse and appear in effective config, but record nothing.

## 0.3 Add artifact names

Update `stutter/src/artifacts.rs`.

Add `ArtifactKind` variants:

```rust
KmsFlipEvents,
DrmFenceEvents,
WaylandPresentationEvents,
```

Add `ArtifactCounter` variants:

```rust
KmsFlipEvent,
DrmFenceEvent,
WaylandPresentationEvent,
```

Add specs:

```rust
ArtifactSpec {
    kind: ArtifactKind::KmsFlipEvents,
    file_name: "kms_flip_events.json",
    encoding: ArtifactEncoding::Ndjson,
    required: false,
    legacy_aliases: &[],
    counter_field: Some(ArtifactCounter::KmsFlipEvent),
}

ArtifactSpec {
    kind: ArtifactKind::DrmFenceEvents,
    file_name: "drm_fence_events.json",
    encoding: ArtifactEncoding::Ndjson,
    required: false,
    legacy_aliases: &[],
    counter_field: Some(ArtifactCounter::DrmFenceEvent),
}

ArtifactSpec {
    kind: ArtifactKind::WaylandPresentationEvents,
    file_name: "wayland_presentation_events.json",
    encoding: ArtifactEncoding::Ndjson,
    required: false,
    legacy_aliases: &[],
    counter_field: Some(ArtifactCounter::WaylandPresentationEvent),
}
```

Do **not** add a `display_path_cost.json` artifact yet. The UHD630 cable cost should be a **derived comparison between two runs**, not a raw artifact.

The artifact docs explicitly say artifact changes must update the artifact registry, session writing/loading, validation, data-quality behavior, and probe registry together. 

## 0.4 Add recorder event structs

In `stutter/src/recorder/event_types.rs`, add:

```rust
pub struct KmsFlipEventRecord { ... }
pub struct DrmFenceEventRecord { ... }
pub struct WaylandPresentationEventRecord { ... }
```

Start with fields that are easy to validate and useful later.

### `KmsFlipEventRecord`

```rust
pub struct KmsFlipEventRecord {
    pub elapsed_ms: u64,
    pub timestamp_ns: u64,

    pub source: String,        // "drm_tracepoint", "i915_tracepoint", etc.
    pub card: Option<String>,  // "card0"
    pub driver: Option<String>,// "i915", "amdgpu"
    pub crtc_id: Option<u32>,
    pub connector: Option<String>,

    pub event_kind: String,    // "commit", "pageflip_done", "vblank", "flip_interval"
    pub sequence: Option<u64>,

    pub request_ns: Option<u64>,
    pub done_ns: Option<u64>,
    pub duration_ns: Option<u64>,

    pub flags: Vec<String>,
    pub confidence: String,    // "high", "medium", "low"
}
```

### `DrmFenceEventRecord`

```rust
pub struct DrmFenceEventRecord {
    pub elapsed_ms: u64,
    pub timestamp_ns: u64,

    pub source: String,          // "dma_fence", "drm_sched", "amdgpu", "i915"
    pub event_kind: String,      // "wait_start", "wait_done", "signal", "wait_interval"

    pub driver: Option<String>,
    pub card: Option<String>,
    pub gpu_role: Option<String>, // "render", "display", "unknown"

    pub pid: Option<u32>,
    pub tid: Option<u32>,
    pub comm: Option<String>,

    pub context: Option<u64>,
    pub seqno: Option<u64>,
    pub timeline_hash: Option<u64>,

    pub wait_start_ns: Option<u64>,
    pub wait_done_ns: Option<u64>,
    pub duration_ns: Option<u64>,

    pub exporter_driver: Option<String>, // e.g. "amdgpu"
    pub importer_driver: Option<String>, // e.g. "i915"
    pub correlation_basis: String,       // "context_seqno", "driver_only", "unknown"
    pub confidence: String,
}
```

### `WaylandPresentationEventRecord`

```rust
pub struct WaylandPresentationEventRecord {
    pub elapsed_ms: u64,

    pub source: String,          // "external_log", "gamescope", "self_test"
    pub app_id: Option<String>,
    pub surface_role: Option<String>,

    pub commit_ns: Option<u64>,
    pub presented_ns: Option<u64>,
    pub commit_to_present_ns: Option<u64>,

    pub output_name: Option<String>,
    pub refresh_ns: Option<u64>,
    pub sequence: Option<u64>,

    pub zero_copy: Option<bool>,
    pub discarded: bool,
    pub flags: Vec<String>,

    pub confidence: String,
}
```

## 0.5 Add session counters

In `stutter/src/recorder/live.rs`, extend `RecordingCounters`:

```rust
pub kms_flip_event_count: u64,
pub drm_fence_event_count: u64,
pub wayland_presentation_event_count: u64,
```

In `stutter/src/recorder/session_files.rs`, add matching count fields to session metadata/core structs with `#[serde(default)]` so old recordings still load.

Also update:

```rust
metadata.json
session.json
validate
strict validate
artifacts summary
```

## 0.6 Add `MonitorEvent` variants

In `stutter/src/session_events.rs`, add:

```rust
KmsFlipEvent {
    event: Box<recorder::KmsFlipEventRecord>,
},

DrmFenceEvent {
    event: Box<recorder::DrmFenceEventRecord>,
},

WaylandPresentationEvent {
    event: Box<recorder::WaylandPresentationEventRecord>,
},
```

Update:

```rust
MonitorEvent::kind()
MonitorEvent::elapsed_ms()
MonitorEvent::delivery_class()
```

## 0.7 Add sink handling

In `stutter/src/session/sinks.rs`, add branches to `RecorderSink::on_event()`:

```rust
MonitorEvent::KmsFlipEvent { event } => {
    push_artifact_event(
        ctx.recorder,
        ArtifactKind::KmsFlipEvents,
        event.as_ref(),
        "kms_flip_events",
        |c| c.kms_flip_event_count += 1,
    );
}
```

Same pattern for:

```rust
DrmFenceEvents
WaylandPresentationEvents
```

This mirrors the existing `IrqEvent`, `IoEvent`, `GpuSample`, and `Frame` handling. 

## 0.8 Add artifact loading

In `stutter/src/session_io.rs`, extend `RunArtifacts`:

```rust
pub kms_flip_events: Vec<KmsFlipEventRecord>,
pub drm_fence_events: Vec<DrmFenceEventRecord>,
pub wayland_presentation_events: Vec<WaylandPresentationEventRecord>,
```

Update:

```rust
load_run_artifacts()
load_artifact_stream()
check_consistency()
expected_artifact_count_for_counter()
count_artifact_kind()
load_correlations()
```

For correlation windows, include only events near frame spikes / scheduler clusters:

```text
KMS: event.timestamp_ns or duration interval overlaps window
Fence: wait interval overlaps window
Wayland: presented_ns or commit_to_present interval overlaps window
```

## 0.9 Add empty report summaries first

In `stutter/src/report/model.rs`, add:

```rust
pub kms_timing: KmsTimingSummary,
pub drm_fence_timing: DrmFenceTimingSummary,
pub wayland_presentation: WaylandPresentationSummary,
```

Each summary should be safe when no artifact exists:

```rust
pub struct KmsTimingSummary {
    pub event_count: usize,
    pub duration_count: usize,
    pub median_flip_ms: Option<f64>,
    pub p95_flip_ms: Option<f64>,
    pub p99_flip_ms: Option<f64>,
    pub max_flip_ms: Option<f64>,
    pub notes: Vec<String>,
}
```

Do equivalent summaries for fence and Wayland.

In `stutter/src/report/analysis.rs`, return empty summaries initially.

This gives you a clean, testable scaffold before touching tracepoints.

## 0.10 Add docs and fixtures immediately

Update:

```text
docs/ARTIFACT_SCHEMA.md
docs/PROBE_ADMISSION.md
docs/examples/artifacts/vXX/
```

Add minimal NDJSON examples:

```text
docs/examples/artifacts/vXX/kms_flip_events.json
docs/examples/artifacts/vXX/drm_fence_events.json
docs/examples/artifacts/vXX/wayland_presentation_events.json
```

Add tests that:

```text
load empty optional files
load valid example files
reject malformed present files
allow missing optional files
include counts in analysis JSON
do not mark missing optional probes as proof of no issue
```

This follows the project’s existing artifact contract and optional-stream behavior. 

---

# 1. Implement DRM/KMS pageflip / scanout timing

Goal:

```text
Measure when a frame/presentation request reaches KMS and when the pageflip/vblank completion event happens.
```

Important limitation:

```text
This is not true photon timing.
It is pageflip / vblank / KMS presentation timing.
```

The Linux DRM/KMS API supports page-flip completion events, and the DRM docs state that drivers must wait for rendering to the new framebuffer before executing the flip, including rendering from other drivers when the buffer is shared through dma-buf. That makes this probe directly relevant to a 9070 XT render → UHD630 scanout setup. ([Kernel Documentation][1])

## 1.1 Add a tracepoint discovery layer

STATUS: Completed 2026-05-19.

Do this before eBPF.

Create:

```text
stutter/src/drm_tracepoints.rs
```

Add structs:

```rust
pub struct DrmTracepointField {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub signed: bool,
}

pub struct DrmTracepointFormat {
    pub category: String,
    pub name: String,
    pub fields: Vec<DrmTracepointField>,
}

pub struct KmsTracepointAvailability {
    pub pageflip_request: Option<TracepointFormat>,
    pub pageflip_done: Option<TracepointFormat>,
    pub vblank_event: Option<TracepointFormat>,
    pub atomic_commit: Option<TracepointFormat>,
    pub provider: KmsTracepointProvider,
}
```

Add provider enum:

```rust
pub enum KmsTracepointProvider {
    GenericDrm,
    I915,
    Amdgpu,
    Mixed,
    Unavailable,
}
```

Reuse the parser style already used in `ebpf_loader.rs` for block I/O tracepoint format validation.

Minimum discovery paths:

```text
/sys/kernel/tracing/events/drm/
/sys/kernel/tracing/events/i915/
/sys/kernel/tracing/events/amdgpu/
```

Do not assume field names yet. Discovery should print what exists.

## 1.2 Add a doctor/preflight output

STATUS: Completed 2026-05-19.

Extend `stutter doctor`:

```bash
stutter doctor --kms-timing
```

Expected output style:

```text
KMS timing:
  generic drm tracepoints: unavailable
  i915 pageflip tracepoints: available
  amdgpu pageflip tracepoints: unavailable
  selected provider: i915
  usable fields:
    crtc_id: yes
    sequence: yes
    timestamp: yes
  status: usable with medium confidence
```

Failure should be non-fatal:

```text
KMS timing unavailable: no supported pageflip/vblank tracepoints found
```

## 1.3 Add common eBPF event type

STATUS: Completed 2026-05-19.

In `stutter-common/src/lib.rs`, add:

```rust
pub const EVENT_KMS_FLIP: u32 = 8;
```

Add C-compatible struct:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KmsFlipEvent {
    pub kind: u32,

    pub event_kind: u32, // request, done, interval, vblank
    pub provider: u32,   // generic, i915, amdgpu
    pub flags: u32,

    pub pid: u32,
    pub tid: u32,
    pub cpu: u32,

    pub card_minor: u32,
    pub crtc_id: u32,
    pub pipe: u32,

    pub sequence: u64,
    pub request_ns: u64,
    pub done_ns: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
}
```

Add flags:

```rust
pub const KMS_FLIP_HAS_REQUEST_NS: u32 = 1 << 0;
pub const KMS_FLIP_HAS_DONE_NS: u32 = 1 << 1;
pub const KMS_FLIP_HAS_DURATION_NS: u32 = 1 << 2;
pub const KMS_FLIP_HAS_SEQUENCE: u32 = 1 << 3;
pub const KMS_FLIP_HAS_CRTC: u32 = 1 << 4;
```

Add `aya::Pod` impl and compile-time size assertion like existing event structs. The current code uses compile-time struct-size assertions to keep eBPF and userspace layouts aligned. 

## 1.4 Add eBPF map for pending flips

STATUS: Completed 2026-05-19.

In `stutter-ebpf/src/main.rs`, add a map:

```rust
#[map]
static mut KMS_FLIP_STARTS: HashMap<KmsFlipKey, u64> = HashMap::with_max_entries(4096, 0);
```

Key:

```rust
#[repr(C)]
pub struct KmsFlipKey {
    pub card_minor: u32,
    pub crtc_id: u32,
    pub pipe: u32,
}
```

Use whichever identity the tracepoint provider exposes.

## 1.5 Add eBPF tracepoint handlers, one provider at a time

Do not implement all providers at once.

### 1.5.1 First provider: i915-only

STATUS: Completed 2026-05-19.

Start with the display GPU in your target system: UHD630 uses i915.

Add tracepoint handlers only when doctor confirms usable fields.

Pattern:

```rust
#[tracepoint]
pub fn i915_flip_request(ctx: TracePointContext) -> u32 {
    try_i915_flip_request(ctx).unwrap_or(0)
}

#[tracepoint]
pub fn i915_flip_done(ctx: TracePointContext) -> u32 {
    try_i915_flip_done(ctx).unwrap_or(0)
}
```

First handler:

```text
read crtc/pipe
record timestamp in KMS_FLIP_STARTS
```

Second handler:

```text
read crtc/pipe/sequence
look up start timestamp
compute duration
emit EVENT_KMS_FLIP
delete map entry
```

If no start exists, emit a completion-only event with low confidence.

### 1.5.2 Second provider: generic DRM

STATUS: Completed 2026-05-19.

After i915 works, add generic DRM tracepoints if present.

### 1.5.3 Third provider: amdgpu

STATUS: Completed 2026-05-19.

Add later only if needed for displays physically attached to AMD GPUs.

## 1.6 Extend `ebpf_loader.rs`

STATUS: Completed 2026-05-19.

Add tracepoint availability fields:

```rust
pub struct TracepointAvailability {
    ...
    pub kms: KmsTracepointAvailability,
}
```

Add validation:

```rust
validate_kms_tracepoint_offsets()
```

Add BPF global offset overrides if tracepoint fields vary by kernel.

Current block-I/O code already has the pattern: userspace validates tracepoint field offsets, then writes offsets into BPF globals before attaching programs. Reuse that approach.

## 1.7 Extend `ProbeActivationPlan`

STATUS: Completed 2026-05-19.

In `stutter/src/probe_activation.rs`:

```rust
ProbeKey::KmsPageflipTiming => config.probes.kms_timing
```

Add unavailable reasons:

```rust
"kms_timing_requested_but_no_supported_tracepoints"
"kms_timing_requested_but_missing_required_fields"
```

Add required artifact:

```rust
ArtifactKind::KmsFlipEvents
```

## 1.8 Decode events in the session loop

STATUS: Completed 2026-05-19.

In `MonitorSession::drain_bpf_events`, add branch:

```rust
EVENT_KMS_FLIP => {
    let event = ptr::read_unaligned(data.as_ptr() as *const KmsFlipEvent);
    ...
}
```

Convert to `KmsFlipEventRecord`.

Dispatch:

```rust
MonitorEvent::KmsFlipEvent { event: Box::new(record) }
```

## 1.9 Add report summary

STATUS: Completed 2026-05-19.

In `report/analysis.rs`, add:

```rust
build_kms_timing_summary(&artifacts.kms_flip_events, &artifacts.frame_events)
```

Compute:

```text
event_count
duration_count
median_flip_ms
p95_flip_ms
p99_flip_ms
max_flip_ms
long_flip_count
events_near_frame_outliers
```

Add notes:

```text
no KMS timing events present
KMS timing requested but unavailable
only completion events present, request-to-done duration unavailable
KMS flip p99 rose near frame outliers
```

## 1.10 Add “scanout estimate” only after pageflip timing works

STATUS: Completed 2026-05-19.

Do **not** implement this first.

Once pageflip completion timing works, add derived estimates:

```text
estimated_top_of_screen_visible_ns = pageflip_done_ns
estimated_bottom_of_screen_visible_ns = pageflip_done_ns + refresh_period_ns
```

This is only an estimate. It assumes conventional scanout behavior and does not include monitor processing or pixel response.

Call it:

```text
scanout_window_estimate
```

Not:

```text
photon_latency
```

---

# 2. Implement dGPU → iGPU fence/copy timing

Goal:

```text
Find whether frame spikes are caused by GPU fence waits / cross-GPU synchronization rather than CPU scheduler delay.
```

This is the hardest part.

Linux’s dma-buf/dma-fence system is the relevant mechanism: dma-buf lets drivers share buffers, dma-fence signals completion of asynchronous hardware work, and dma-resv manages fences associated with shared buffers. The docs also describe implicit-fence polling/export behavior, which is useful background for validating what the kernel is doing. ([Kernel Documentation][2])

## 2.1 Narrow the initial target

STATUS: Completed 2026-05-19.

Do not try to support every GPU combination.

Initial target:

```text
render GPU:  amdgpu / RX 9070 XT
display GPU: i915 / UHD630
mode:        Gamescope DRM session if possible
monitor:     single output
```

This matches the user’s setup and avoids designing for NVIDIA, mixed compositors, capture stacks, and multi-output complexity on day one.

## 2.2 Promote existing planned probe

STATUS: Completed 2026-05-19.

In `probe_registry.rs`, update existing `ProbeKey::DrmFenceLatency`.

Current status:

```rust
status: ProbeStatus::Planned
artifacts: &[]
cli_flags: &[]
```

Change eventually to:

```rust
status: ProbeStatus::Implemented
cli_flags: &["--drm-fence-latency"]
artifacts: &[ArtifactKind::DrmFenceEvents]
required_capabilities: &[Ebpf, Tracepoint]
overhead: ProbeOverhead::High
```

But do **not** mark implemented until artifact writing, validation, and report analysis exist.

## 2.3 Add tracepoint discovery first

STATUS: Completed 2026-05-19.

Create:

```text
stutter/src/drm_fence_tracepoints.rs
```

Search tracefs for usable categories:

```text
/sys/kernel/tracing/events/dma_fence/
/sys/kernel/tracing/events/dma_buf/
/sys/kernel/tracing/events/sync_file/
/sys/kernel/tracing/events/drm_sched/
/sys/kernel/tracing/events/amdgpu/
/sys/kernel/tracing/events/i915/
```

Add command:

```bash
stutter inspect-drm-tracepoints
```

Initial output:

```text
DRM fence tracepoint discovery:
  dma_fence: unavailable
  drm_sched: available
  amdgpu: available
  i915: available
  supported profile: amdgpu+i915 partial
```

This command should not require a running game.

## 2.4 Build a compatibility matrix file

STATUS: Completed 2026-05-19.

Add:

```text
docs/DRM_FENCE_COMPATIBILITY.md
```

Record:

```text
kernel version
driver
tracepoint category
tracepoint name
fields needed
fields present
confidence
```

Example:

```text
i915 display wait: supported if i915 tracepoint X has context/seqno/duration
amdgpu scheduler: supported if amdgpu tracepoint Y has job id/timeline/seqno
generic dma_fence: preferred if available
```

This follows the project’s own admission gate for DRM fence latency: it explicitly asks for stable tracepoint research and a vendor compatibility matrix before enabling the probe. 

## 2.5 Add common eBPF event type

STATUS: Completed 2026-05-19.

In `stutter-common/src/lib.rs`:

```rust
pub const EVENT_DRM_FENCE: u32 = 9;
```

Add:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmFenceEvent {
    pub kind: u32,

    pub event_kind: u32,  // wait_start, wait_done, signal, interval
    pub provider: u32,    // dma_fence, drm_sched, amdgpu, i915
    pub flags: u32,

    pub pid: u32,
    pub tid: u32,
    pub cpu: u32,

    pub driver_id: u32,   // userspace maps to string
    pub gpu_role: u32,    // unknown/render/display

    pub context: u64,
    pub seqno: u64,
    pub timeline_hash: u64,

    pub wait_start_ns: u64,
    pub wait_done_ns: u64,
    pub signal_ns: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
}
```

Add validity flags:

```rust
DRM_FENCE_HAS_CONTEXT
DRM_FENCE_HAS_SEQNO
DRM_FENCE_HAS_TIMELINE
DRM_FENCE_HAS_DURATION
DRM_FENCE_HAS_PID
DRM_FENCE_IS_IMPORTER_SIDE
DRM_FENCE_IS_EXPORTER_SIDE
```

## 2.6 Add BPF maps for fence intervals

STATUS: Completed 2026-05-19.

Add maps:

```rust
FENCE_WAIT_STARTS: HashMap<FenceKey, FenceWaitStart>
FENCE_SIGNAL_TIMES: HashMap<FenceKey, u64>
```

Key:

```rust
#[repr(C)]
pub struct FenceKey {
    pub context: u64,
    pub seqno: u64,
}
```

If context/seqno is not available, use a weaker key:

```rust
timeline_hash + seqno
```

If no stable key is available, do not emit high-confidence intervals.

## 2.7 First implementation: fence wait intervals only

STATUS: Completed 2026-05-19.

Start with the simplest useful question:

```text
Did any GPU/display fence wait last unusually long near a frame spike?
```

Do not attempt copy attribution yet.

Tracepoint logic:

```text
wait_start:
  store FenceKey -> timestamp, pid, tid, provider

wait_done:
  lookup FenceKey
  compute duration
  emit DrmFenceEvent interval
  delete map entry
```

If only signal events exist:

```text
emit signal-only events
confidence = low
```

## 2.8 Second implementation: render-side amdgpu events

STATUS: Completed 2026-05-19.

Add amdgpu-specific provider only after generic wait intervals work.

Goal:

```text
Was the RX 9070 XT render queue late?
```

Record:

```text
amdgpu job queued
amdgpu job started
amdgpu job finished/signaled
```

Keep this separate from display-side waits.

Do **not** yet say this is dGPU→iGPU copy cost. Say:

```text
render GPU queue/fence delay
```

## 2.9 Third implementation: display-side i915 wait events

STATUS: Completed 2026-05-19.

Add i915-specific provider.

Goal:

```text
Did the UHD630 display path wait on an imported/shared buffer fence?
```

Record:

```text
i915 wait start
i915 wait done
duration
process/thread if available
context/seqno if available
```

If the event can be linked to an amdgpu fence, mark:

```text
exporter_driver = "amdgpu"
importer_driver = "i915"
correlation_basis = "context_seqno"
confidence = "high"
```

If not:

```text
correlation_basis = "driver_time_overlap"
confidence = "medium" or "low"
```

## 2.10 Fourth implementation: correlate with KMS pageflip

STATUS: Completed 2026-05-19.

Now combine:

```text
KMS flip delayed
+
i915 fence wait near same timestamp
+
frame outlier near same timestamp
```

Report candidate:

```text
cross_gpu_display_wait_candidate
```

Do **not** report:

```text
copy latency = exact X ms
```

Report:

```text
i915 display-side fence waits explain up to X ms near frame outliers
```

That wording is honest.

## 2.11 Add report summary

STATUS: Completed 2026-05-19.

In `report/analysis.rs`, add:

```rust
build_drm_fence_timing_summary(
    &artifacts.drm_fence_events,
    &artifacts.kms_flip_events,
    &artifacts.frame_events,
    &clusters,
)
```

Fields:

```rust
pub struct DrmFenceTimingSummary {
    pub event_count: usize,
    pub wait_interval_count: usize,

    pub median_wait_ms: Option<f64>,
    pub p95_wait_ms: Option<f64>,
    pub p99_wait_ms: Option<f64>,
    pub max_wait_ms: Option<f64>,

    pub render_gpu_wait_count: usize,
    pub display_gpu_wait_count: usize,
    pub cross_gpu_candidate_count: usize,

    pub waits_near_frame_outliers: usize,
    pub waits_near_kms_delays: usize,

    pub top_waits: Vec<DrmFenceWaitSummary>,
    pub notes: Vec<String>,
    pub confidence: String,
}
```

## 2.12 Data-quality rules

STATUS: Completed 2026-05-19.

Downgrade quality when:

```text
tracepoints missing
only signal-only events exist
no stable fence key exists
ring buffer drops occurred
driver mapping unknown
both render and display cards not identified
too many events were truncated
```

Never treat missing fence events as proof that there was no GPU wait.

---

# 3. Implement Wayland presentation timing

Goal:

```text
Measure commit-to-present timing, discarded frames, output identity, and zero-copy/direct-scanout hints where available.
```

Wayland’s `presentation-time` protocol is the correct concept here: feedback is associated with a `wl_surface.commit`, `presented` reports final presentation timing, and flags can indicate things like zero-copy presentation. ([wayland.app][3])

Important limitation:

```text
stutter cannot observe arbitrary Wayland clients unless the client, Gamescope, compositor, or a wrapper cooperates.
```

So implement this in three levels.

---

## 3A. Level 1: external log ingestion

This is the safest first version.

## 3A.1 Define log format

STATUS: Completed 2026-05-19.

Create docs:

```text
docs/WAYLAND_PRESENTATION_LOG.md
```

Use NDJSON or CSV. Prefer NDJSON.

Example line:

```json
{
  "commit_ns": 123456789000,
  "presented_ns": 123456797400,
  "output_name": "DP-1",
  "refresh_ns": 6944444,
  "sequence": 99182,
  "zero_copy": true,
  "discarded": false,
  "source": "gamescope"
}
```

## 3A.2 Add parser module

STATUS: Completed 2026-05-19.

Create:

```text
stutter/src/wayland_presentation.rs
```

Add:

```rust
pub struct WaylandPresentationLogReader { ... }

impl WaylandPresentationLogReader {
    pub fn open(path: &Path) -> anyhow::Result<Self>;
    pub fn read_new_events(&mut self) -> anyhow::Result<Vec<WaylandPresentationEventRecord>>;
}
```

Mirror the existing MangoHud log-tail pattern.

## 3A.3 Add monitor tick

STATUS: Completed 2026-05-19.

In `MonitorSession`, add a tick context similar to `FrameTickContext`:

```rust
struct WaylandPresentationTickContext {
    event: recorder::WaylandPresentationEventRecord,
}
```

Add runtime state:

```rust
wayland_presentation_reader: Option<WaylandPresentationLogReader>
```

On each telemetry tick:

```text
read new log events
convert timestamps to elapsed_ms
dispatch MonitorEvent::WaylandPresentationEvent
```

## 3A.4 Report summary

STATUS: Completed 2026-05-19.

Add:

```rust
build_wayland_presentation_summary(&artifacts.wayland_presentation_events)
```

Fields:

```rust
event_count
presented_count
discarded_count
zero_copy_count
zero_copy_ratio
median_commit_to_present_ms
p95_commit_to_present_ms
p99_commit_to_present_ms
max_commit_to_present_ms
outputs_seen
notes
```

---

## 3B. Level 2: self-test Wayland client

This measures compositor/output behavior, not the actual game.

## 3B.1 Add a new subcommand

STATUS: Completed 2026-05-19.

Add:

```bash
stutter wayland-probe \
  --duration 30 \
  --output DP-1 \
  --fullscreen \
  --out-dir ./wayland-probe-run
```

This should run without root.

## 3B.2 Use `wayland-client` and `presentation-time`

STATUS: Completed 2026-05-19.

Add optional dependency:

```toml
wayland-client = ...
wayland-protocols = ...
```

Gate behind a Cargo feature if you want to avoid pulling Wayland dependencies into all builds:

```toml
features = ["wayland-probe"]
```

## 3B.3 Create a surface and request feedback

STATUS: Completed 2026-05-19.

Loop:

```text
draw frame
wl_surface.commit
request presentation feedback
wait for presented/discarded
record commit_ns/presented_ns
```

Write:

```text
wayland_presentation_events.json
```

This gives a clean compositor/output baseline.

## 3B.4 Add warning in report

STATUS: Completed 2026-05-19.

If source is `self_test`, report:

```text
This measures stutter's own test surface, not the game surface.
```

---

## 3C. Level 3: Gamescope/compositor cooperation

This is the useful gaming version.

## 3C.1 Prefer Gamescope stats over LD_PRELOAD

STATUS: Completed 2026-05-19.

Avoid LD_PRELOAD first. Proton, Steam Runtime, anti-cheat, and Vulkan layers make this fragile.

Better path:

```text
Gamescope emits presentation events
stutter ingests them with --wayland-presentation-log
```

Ideal Gamescope data:

```text
game frame received
game frame submitted to Gamescope
Gamescope commit
presentation feedback
direct scanout / composited status
KMS flip done if DRM backend
VRR status
```

## 3C.2 Add source classification

STATUS: Completed 2026-05-19.

In `WaylandPresentationEventRecord`:

```rust
source: "gamescope"
surface_role: "game"
```

Then report separately:

```text
game surface presentation
gamescope output presentation
self-test surface presentation
```

## 3C.3 Correlate with KMS and frame events

STATUS: Completed 2026-05-19.

Derived report:

```text
MangoHud frame outlier at 83.4s
Wayland commit-to-present: 11.8 ms
KMS flip duration: 1.2 ms
scheduler delay: low
candidate: compositor/presentation queue delay
```

---

# 4. Implement “display cable is on UHD630” cost

This should **not** be a live probe.

It should be a comparison between two controlled runs.

Goal:

```text
Estimate the cost of using UHD630 as the display/scanout GPU instead of the RX 9070 XT.
```

## 4.1 Add display-path metadata

STATUS: Completed 2026-05-19.

In `MonitorConfig`, add optional label:

```rust
pub display_path_label: Option<String>,
```

CLI:

```bash
--display-path-label dgpu-display
--display-path-label uhd630-display
```

Also add optional metadata:

```bash
--display-render-gpu amdgpu
--display-scanout-gpu i915
--display-connector DP-1
```

This metadata should go into `metadata.json` / `session.json`.

Example:

```json
"display_path": {
  "label": "uhd630-display",
  "render_gpu": "amdgpu",
  "scanout_gpu": "i915",
  "connector": "DP-1"
}
```

## 4.2 Add a comparison command

STATUS: Completed 2026-05-19.

Add one of these:

```bash
stutter compare display-path \
  --baseline runs/dgpu-display \
  --test runs/uhd630-display
```

or extend existing report diff machinery:

```bash
stutter report --compare-display-path \
  --baseline runs/dgpu-display \
  --test runs/uhd630-display
```

Prefer a dedicated command if the current report diff code is already crowded.

## 4.3 Require two runs

STATUS: Completed 2026-05-19.

The command should refuse to produce a high-confidence estimate from one run.

Inputs:

```text
baseline run: display cable on 9070 XT
test run:     display cable on UHD630
```

Optional labels:

```text
baseline label: dgpu-display
test label: uhd630-display
```

## 4.4 Validate comparability

STATUS: Completed 2026-05-19.

Before calculating cost, check:

```text
same scenario name, if available
same duration within tolerance
same MangoHud/frame log availability
same resolution, if metadata exists
same refresh/FPS cap, if metadata exists
same game process class
same Gamescope mode, if metadata exists
same probe availability
```

Output:

```text
comparison_quality: high | medium | low
comparison_warnings: [...]
```

Examples:

```text
medium: baseline has KMS timing, test lacks KMS timing
low: different durations and no frame logs
low: baseline capped at 141 FPS, test uncapped
```

## 4.5 Compute basic FPS and frame-pacing deltas

STATUS: Completed 2026-05-19.

Use existing frame events first.

The artifact docs say `frame_pacing` already includes frame count, median, p95, p99, max, outlier count, compositor cluster count, and game cluster count. 

Calculate:

```text
baseline_fps = baseline.frame_count / baseline.duration_s
test_fps     = test.frame_count / test.duration_s

fps_delta = test_fps - baseline_fps
fps_delta_percent = (test_fps - baseline_fps) / baseline_fps * 100
```

Calculate frametime deltas:

```text
median_delta_ms = test.median_ms - baseline.median_ms
p95_delta_ms    = test.p95_ms - baseline.p95_ms
p99_delta_ms    = test.p99_ms - baseline.p99_ms
max_delta_ms    = test.max_ms - baseline.max_ms
```

## 4.6 Add KMS timing delta

STATUS: Completed 2026-05-19.

If both runs have `kms_flip_events.json`:

```text
kms_median_delta_ms
kms_p95_delta_ms
kms_p99_delta_ms
kms_long_flip_delta_count
```

Interpretation:

```text
If KMS p99 worsens mainly in UHD630 run, display/scanout path is a candidate.
```

## 4.7 Add fence timing delta

STATUS: Completed 2026-05-19.

If both runs have `drm_fence_events.json`:

```text
display_side_fence_wait_p99_delta_ms
render_side_fence_wait_p99_delta_ms
cross_gpu_candidate_count_delta
```

Interpretation:

```text
If display-side i915 fence waits increase in UHD630 run, cross-GPU synchronization is a candidate.
```

## 4.8 Add Wayland presentation delta

STATUS: Completed 2026-05-19.

If both runs have `wayland_presentation_events.json`:

```text
commit_to_present_p99_delta_ms
discarded_frame_delta
zero_copy_ratio_delta
```

Interpretation:

```text
If commit-to-present worsens and zero_copy ratio falls, compositor/presentation path is a candidate.
```

## 4.9 Add CPU scheduler control comparison

STATUS: Completed 2026-05-19.

Use existing scheduler data:

```text
game_cluster_count_delta
compositor_cluster_count_delta
scheduler_p99_delta
runtime wait delta
```

Interpretation:

```text
If frame pacing worsens but scheduler latency does not, suspect GPU/display/presentation path.
If scheduler latency worsens too, do not blame UHD630 display path alone.
```

This matters because the project’s core current measurement is scheduler runnable latency, not display latency. The README defines the main measurement as `sched_wakeup timestamp -> sched_switch timestamp`. 

## 4.10 Output final estimate

STATUS: Completed 2026-05-19.

Example JSON:

```json
{
  "display_path_cost": {
    "baseline_label": "dgpu-display",
    "test_label": "uhd630-display",
    "comparison_quality": "medium",
    "avg_fps_delta_percent": -4.8,
    "median_frame_delta_ms": 0.3,
    "p95_frame_delta_ms": 1.1,
    "p99_frame_delta_ms": 2.7,
    "kms_p99_delta_ms": 0.9,
    "drm_fence_p99_delta_ms": 1.6,
    "wayland_present_p99_delta_ms": 2.1,
    "likely_causes": [
      "cross_gpu_fence_wait_candidate",
      "wayland_presentation_queue_candidate"
    ],
    "notes": [
      "This is an A/B estimate, not direct photon latency.",
      "Input-to-photon latency requires external measurement hardware."
    ]
  }
}
```

Human report:

```text
Estimated UHD630 display-path cost:
  FPS:             -4.8%
  median frame:    +0.3 ms
  p95 frame:       +1.1 ms
  p99 frame:       +2.7 ms
  KMS p99:         +0.9 ms
  fence p99:       +1.6 ms
  Wayland p99:     +2.1 ms

Likely cause:
  cross-GPU fence/display wait + presentation queue delay

Confidence:
  medium
```

## 4.11 Use cautious wording

STATUS: Completed 2026-05-19.

Never output:

```text
The UHD630 cable added exactly 4.2 ms of latency.
```

Output:

```text
The UHD630-display run showed +4.2 ms p99 frame/presentation cost versus the dGPU-display baseline.
```

That distinction matters.

---
