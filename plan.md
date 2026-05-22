Below is a full methodical implementation plan for the six changes, ordered so each step leaves the codebase compiling and easier to review.

# Implementation progress

- [x] 0. Preparation steps completed on branch `fix-autotune-comparison-and-architecture-cleanups`; baseline `cargo fmt --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings` passed. Surface snapshots were saved under `.llm-runs/plan-implementation/`.
- [x] 1. Normalize experiment score comparison completed: `WindowScore` now exposes normalized rates, experiment comparison uses score-per-sample and over-5ms-per-1k-samples for decisions, raw totals remain diagnostic, docs were updated, and `cargo test -p stutter` plus `cargo clippy -p stutter --all-targets -- -D warnings` passed.
- [x] 2. Make objective comparison semantics explicit and differentiated completed: objective decisions now use explicit primary metrics for I/O, IRQ, and thermal recovery, foreground/frame objectives use direct signals with normalized-score fallback, docs and architecture coverage were added, and `cargo test -p stutter` plus `cargo clippy -p stutter --all-targets -- -D warnings` passed.
- [x] 3. Add async/concurrency model documentation completed: `docs/CONCURRENCY.md` now records runtime, store, channel, lock, blocking, and shutdown rules; daemon/agent ownership boundaries are locally documented; an architecture test enforces the doc; and architecture, daemon, agent, and clippy checks passed.
- [x] 4. Remove the `autotune::candidate` migration facade completed: internal imports now target `autotune::planning::*`, the public `api::autotune::candidate` facade re-exports from real owner modules, the internal shim was deleted, an architecture guard prevents resurrection, and check/test/clippy passed.
- [x] 5. Section and document the eBPF program completed: `stutter-ebpf/src/main.rs` now has a file map, logical sections, and tracepoint comments without behavior changes; architecture docs note the single-translation-unit constraint. `cargo test --workspace` and workspace clippy passed; direct `cargo check -p stutter-ebpf` remains blocked by the existing no-std/BPF target toolchain setup in this environment.
- [ ] 6. Decompose `actions/cgroup.rs`
- [ ] Final integration pass

# Overall patch order

Do the work in this order:

1. **Normalize experiment score comparison**
2. **Make objective comparison semantics explicit and more differentiated**
3. **Add async/concurrency model documentation**
4. **Remove the `autotune::candidate` migration facade**
5. **Section and document the eBPF program**
6. **Decompose `actions/cgroup.rs`**

The first two should happen before the rest because they affect tuning correctness. The remaining four are maintainability and architecture cleanup.

---

# 0. Preparation steps

1. Create a working branch:

```bash
git switch -c fix-autotune-comparison-and-architecture-cleanups
```

2. Run the current baseline checks and save the output:

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

3. Use `rg` to capture current dependency/import surfaces:

```bash
rg "autotune::candidate|crate::autotune::candidate|super::candidate|candidate::" stutter/src
rg "compare_experiment|compare_for_objective|score.total|over_5ms" stutter/src/autotune stutter/src/scorer.rs
rg "DaemonStateStore|tokio::sync|tokio::spawn|mpsc|oneshot|Mutex|RwLock" stutter/src/daemon stutter/src/agent stutter/src/session
```

4. Do not mix formatting-only churn into logic patches. For each phase, run:

```bash
cargo fmt
cargo test -p stutter
```

5. After each major phase, run the full workspace checks again.

---

# 1. Normalize experiment score comparison

## Goal

Stop comparing raw accumulated `score.total` values when baseline and candidate windows have different `scored_samples`, `interval_count`, or durations.

The decision logic should compare **rates**, while still retaining raw totals for diagnostics.

## 1.1 Add failing comparison tests first

Edit:

```text
stutter/src/autotune/comparison.rs
```

Add tests proving the bug exists.

1. Add a helper that allows custom sample counts:

```rust
fn window_with_samples(
    total: u64,
    over_5ms: u64,
    frame_p99_ms: f64,
    scored_samples: u64,
    interval_count: usize,
) -> WindowScore
```

2. Add a test where baseline and candidate have the same score rate but candidate has 3× samples:

```text
baseline: total=1000, scored_samples=100
candidate: total=3000, scored_samples=300
```

Expected result after the fix:

```text
Inconclusive, not Regressed
```

3. Add a test where candidate has lower raw total but worse normalized rate:

```text
baseline: total=1000, scored_samples=1000 => 1.0/sample
candidate: total=900, scored_samples=100 => 9.0/sample
```

Expected result:

```text
Regressed
```

4. Add a test where candidate has better normalized rate despite higher raw total:

```text
baseline: total=1000, scored_samples=100 => 10.0/sample
candidate: total=1500, scored_samples=300 => 5.0/sample
```

Expected result:

```text
Improved
```

5. Add an equal-sample test proving old behavior is preserved when sample counts match.

## 1.2 Add normalized metric helpers to `WindowScore`

Edit:

```text
stutter/src/autotune/experiment.rs
```

Add methods:

```rust
impl WindowScore {
    pub fn score_per_sample(&self) -> Option<f64> { ... }
    pub fn over_1ms_per_sample(&self) -> Option<f64> { ... }
    pub fn over_2ms_per_sample(&self) -> Option<f64> { ... }
    pub fn over_5ms_per_sample(&self) -> Option<f64> { ... }
    pub fn duration_seconds(&self) -> Option<f64> { ... }
    pub fn score_per_second(&self) -> Option<f64> { ... }
}
```

Implementation rules:

1. Return `None` when denominator is zero.
2. Use `scored_samples` for latency score normalization.
3. Use `duration_unix_nanos()` for time normalization.
4. Keep `score_total()` unchanged for raw reporting compatibility.
5. Add unit tests in `experiment.rs` for zero denominator, normal denominator, and duration calculation.

## 1.3 Add comparison value helpers

Edit:

```text
stutter/src/autotune/comparison.rs
```

Add internal helpers:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
struct ComparableScore {
    score_per_sample: f64,
    over_5ms_per_sample: f64,
    raw_total: u64,
    raw_over_5ms: u64,
}
```

Add constructor:

```rust
fn comparable_score(label: &str, score: &WindowScore) -> Result<ComparableScore, String>
```

Rules:

1. Use `WindowScore::score_per_sample()`.
2. Use `WindowScore::over_5ms_per_sample()`.
3. Return `Invalid` if either value is missing or non-finite.
4. Keep raw fields only for messages and debug context.

## 1.4 Convert percent helpers to operate on `f64`

Currently:

```rust
fn improvement_percent(baseline_total: u64, candidate_total: u64) -> f64
pub(crate) fn regression_percent(baseline_total: u64, candidate_total: u64) -> f64
```

Change to rate-based helpers:

```rust
fn improvement_percent_f64(baseline: f64, candidate: f64) -> f64
pub(crate) fn regression_percent_f64(baseline: f64, candidate: f64) -> f64
```

Then either:

1. Replace callers directly, or
2. Keep the old `u64` helpers only for tests/reporting and route decision code through the new `f64` helpers.

Best option: decision code should only call the `f64` helpers.

## 1.5 Update `compare_scores_with_config`

In:

```text
stutter/src/autotune/comparison.rs
```

Change the core logic from:

```rust
regression_percent(input.baseline.score.total, input.candidate.score.total)
```

to:

```rust
let baseline = comparable_score("baseline", input.baseline)?;
let candidate = comparable_score("candidate", input.candidate)?;
let regression_percent =
    regression_percent_f64(baseline.score_per_sample, candidate.score_per_sample);
```

Do the same for improvement.

## 1.6 Fix the `over_5ms` regression guard

Current logic compares raw counts:

```rust
candidate.over_5ms > baseline.over_5ms + max_over_5ms_regression
```

That still has the same duration bug.

Change the guard to compare normalized rates.

Small-step approach:

1. Add a new config field:

```rust
pub max_over_5ms_regression_per_1k_samples: f64
```

2. Keep the old field temporarily:

```rust
pub max_over_5ms_regression: u64
```

3. Mark the old field as raw-count compatibility only in a comment.
4. Set default normalized slack to `0.0` if you want strict no-regression behavior.
5. Compare like this conceptually:

```text
candidate_over_5ms_per_1k_samples >
baseline_over_5ms_per_1k_samples + max_over_5ms_regression_per_1k_samples
```

6. Update tests to prove a longer candidate window with the same `over_5ms` rate is not rejected.

## 1.7 Improve inconclusive/debug messages

Update the `Inconclusive` message to include both raw and normalized values:

```text
baseline_score_per_sample=...
candidate_score_per_sample=...
baseline_raw_total=...
candidate_raw_total=...
baseline_scored_samples=...
candidate_scored_samples=...
```

This matters because raw totals are still useful for debugging.

## 1.8 Add documentation

Update:

```text
docs/AUTOTUNE_ARCHITECTURE.md
docs/AUTOTUNE_CONFIG.md
```

Add a short section:

```text
Experiment comparison uses normalized score rates for keep/revert decisions.
Raw score totals are retained for diagnostics only.
```

Mention:

```text
score_per_sample = score.total / scored_samples
```

## 1.9 Final verification for change 1

Run:

```bash
cargo test -p stutter autotune::comparison
cargo test -p stutter autotune::experiment
cargo test -p stutter
cargo clippy -p stutter --all-targets -- -D warnings
```

Commit:

```bash
git add stutter/src/autotune/comparison.rs stutter/src/autotune/experiment.rs docs/AUTOTUNE_ARCHITECTURE.md docs/AUTOTUNE_CONFIG.md
git commit -m "Normalize autotune experiment score comparison"
```

---

# 2. Make objective comparison semantics explicit and differentiated

## Goal

Make `ObjectiveKind` behavior honest and clear.

Right now, objective comparison mostly says:

```text
generic stutter-score improvement first,
then objective-specific guardrails
```

The end state should be:

```text
each objective has an explicit primary metric and explicit safety guardrails
```

## 2.1 Document the current intended semantics before changing code

Create:

```text
docs/AUTOTUNE_OBJECTIVES.md
```

Add a table:

| Objective                                   | Primary comparison                                                     | Required signals                      | Guardrails                         |
| ------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------- | ---------------------------------- |
| `StutterScore`                              | normalized score per sample                                            | scored samples                        | frame p99, over-5ms rate           |
| `GameFramePacing`                           | frame pacing + normalized score                                        | frame data if available               | foreground latency, thermal/power  |
| `GameRunnableLatency`                       | normalized runnable latency score                                      | scored task latency                   | frame p99, thermal/power           |
| `DesktopInteractivity`                      | foreground over-5ms rate                                               | foreground latency                    | thermal/power                      |
| `BrowserInteractivity`                      | foreground/browser latency                                             | foreground latency                    | thermal/power                      |
| `CompileThroughputWithForegroundProtection` | compile progress/score when available; foreground protection otherwise | foreground latency                    | frame/foreground regression        |
| `IoLatency`                                 | block I/O overlap reduction                                            | block I/O overlap count/worst latency | generic score cannot regress badly |
| `IrqOverlapReduction`                       | IRQ overlap reduction                                                  | IRQ count/worst overlap               | generic score cannot regress badly |
| `ThermalRecovery`                           | degraded/throttle reduction                                            | thermal signals                       | stutter score cannot regress badly |

Be honest where current signals are approximate.

## 2.2 Add an architecture test that docs exist

Edit or add an architecture test under:

```text
stutter/src/architecture_tests/
```

Add a simple test that checks:

```rust
include_str!("../../../docs/AUTOTUNE_OBJECTIVES.md")
```

contains all `ObjectiveKind` names.

This prevents adding a new objective without documenting it.

## 2.3 Refactor `compare_for_objective` into small stages

Edit:

```text
stutter/src/autotune/objective.rs
```

Replace the current shape:

```rust
let base = compare_experiment(...);
if !Improved { return base; }
...
match objective { ... }
```

with staged helpers:

```rust
pub fn compare_for_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult {
    if let Some(result) = reject_invalid_or_low_quality(input.clone()) { return result; }
    if let Some(result) = missing_objective_signals(input.clone()) { return result; }
    if let Some(result) = reject_if_power_or_thermal_regressed(input.clone()) { return result; }

    match input.objective {
        ObjectiveKind::StutterScore => compare_stutter_score_objective(input),
        ObjectiveKind::GameFramePacing => compare_game_frame_pacing_objective(input),
        ...
    }
}
```

Do this mechanically first, without changing semantics.

Run tests.

## 2.4 Rename generic comparison result inside objective code

Where the code still uses generic comparison, name it clearly:

```rust
let generic_score_result = compare_experiment(...);
```

Do not call it `base`. That hides the behavior.

## 2.5 Add objective-specific comparison helpers one by one

Add one helper per objective:

```rust
fn compare_stutter_score_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_game_frame_pacing_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_game_runnable_latency_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_desktop_interactivity_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_browser_interactivity_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_compile_throughput_with_foreground_protection_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_io_latency_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_irq_overlap_reduction_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
fn compare_thermal_recovery_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult
```

Initially, each helper can preserve current behavior.

Then improve them incrementally.

## 2.6 Make I/O objective primary on I/O signals

For `IoLatency`:

1. Require block I/O signals.
2. Treat reduced overlap count as improvement.
3. Treat equal count but reduced worst latency as improvement.
4. Treat worse count or worse worst latency as regression.
5. If I/O improves but normalized stutter score badly regresses, return `Regressed`.
6. If I/O improves and generic score is neutral/slightly worse within allowed guardrails, return `Improved`.

This fixes the current problem where generic stutter-score improvement must happen first.

Add tests:

```text
io_objective_improves_when_io_overlap_drops_even_if_generic_score_is_flat
io_objective_rejects_when_io_overlap_worsens_even_if_generic_score_improves
io_objective_rejects_when_required_signals_missing
```

## 2.7 Make IRQ objective primary on IRQ signals

For `IrqOverlapReduction`:

1. Require IRQ overlap signals.
2. Candidate improves if overlap count drops.
3. Candidate improves if count is equal but worst overlap drops.
4. Candidate regresses if count rises or worst overlap rises.
5. Generic normalized score becomes a guardrail, not the primary metric.

Add tests equivalent to I/O tests.

## 2.8 Make thermal objective primary on thermal signals

For `ThermalRecovery`:

1. Candidate improves if baseline is degraded and candidate is not.
2. Candidate improves if throttle count drops.
3. Candidate regresses if candidate becomes degraded and baseline was not.
4. Candidate regresses if throttle count rises.
5. Generic normalized score remains a guardrail.

Add tests:

```text
thermal_objective_improves_when_candidate_clears_degraded_state
thermal_objective_improves_when_throttle_count_drops
thermal_objective_rejects_new_thermal_degradation
```

## 2.9 Make frame/interactivity objectives use clearer primary metrics

For `GameFramePacing`:

1. Use `frame_p99_ms` as primary when frame signal quality is not missing.
2. Use normalized stutter score as fallback.
3. Reject if foreground over-5ms rate regresses.
4. Reject if thermal/power guardrails regress.

For `DesktopInteractivity` and `BrowserInteractivity`:

1. Use `foreground_over_5ms` rate when present.
2. Fall back to normalized score when foreground signal is missing.
3. Return `Inconclusive` if the objective claims direct foreground comparison but both foreground and score are unusable.

For `CompileThroughputWithForegroundProtection`:

1. Keep foreground protection as strict guardrail.
2. Do not claim compile throughput is directly measured unless there is an actual compile throughput signal.
3. If no throughput signal exists, document and return generic normalized score comparison guarded by foreground latency.
4. Add a TODO with a concrete future signal name, not vague wording.

## 2.10 Update `objective_regression_percent`

Currently it uses raw total through `regression_percent`.

Change it to the same normalized comparison metric used by `comparison.rs`.

Either:

1. Reuse a public/internal helper from `comparison.rs`, or
2. Add a small local helper that calls `WindowScore::score_per_sample()`.

Do not leave objective regression percentages raw-total based.

## 2.11 Update docs and roadmap

Update:

```text
docs/AUTOTUNE_OBJECTIVES.md
docs/AUTOTUNE_ARCHITECTURE.md
docs/ROADMAP.md
```

Add a roadmap section:

```text
Objective comparison maturity
```

State which objectives are:

```text
primary-metric implemented
guardrail-only
fallback-based
missing direct signal
```

## 2.12 Final verification for change 2

Run:

```bash
cargo test -p stutter autotune::objective
cargo test -p stutter autotune::comparison
cargo test -p stutter architecture_tests
cargo test -p stutter
cargo clippy -p stutter --all-targets -- -D warnings
```

Commit:

```bash
git add stutter/src/autotune/objective.rs stutter/src/architecture_tests docs/AUTOTUNE_OBJECTIVES.md docs/AUTOTUNE_ARCHITECTURE.md docs/ROADMAP.md
git commit -m "Make autotune objective comparison semantics explicit"
```

---

# 3. Add async/concurrency model documentation

## Goal

Make the daemon concurrency model visible and enforceable enough for future refactors.

## 3.1 Create `docs/CONCURRENCY.md`

Add sections:

```text
# Concurrency model

## Runtime model
## Daemon state ownership
## DaemonStateStore mutation rules
## Agent/server task model
## Channel boundaries
## Locking rules
## Blocking filesystem and kernel-state operations
## Shutdown/cancellation model
## Testing expectations
```

## 3.2 Document `DaemonStateStore`

Edit:

```text
stutter/src/daemon/store.rs
```

Add module-level docs explaining:

1. `DaemonStateStore` is a single-owner mutable state store.
2. It is not meant to be shared behind arbitrary `Arc<Mutex<_>>`.
3. Mutations should flow through daemon runtime/policy paths.
4. Persistence writes happen through `replace`.
5. Future locking changes must update `docs/CONCURRENCY.md`.

## 3.3 Document daemon facade concurrency expectations

Edit:

```text
stutter/src/daemon/mod.rs
```

Add a short paragraph to the existing module docs:

```text
Concurrency model:
- daemon state is mutated by owned runtime/store paths;
- async tasks communicate through explicit channels;
- privileged/kernel mutations remain serialized by policy/runtime boundaries.
```

## 3.4 Document agent/server task behavior

Edit relevant files:

```text
stutter/src/agent.rs
stutter/src/agent/server.rs
stutter/src/daemon/monitor.rs
```

Add comments near `tokio::spawn`, `mpsc`, `oneshot`, and `Mutex` usage explaining what is shared and what is not.

Do not over-comment every line. Put the comment at ownership boundaries.

## 3.5 Add architecture test for concurrency docs

Add a test under:

```text
stutter/src/architecture_tests/
```

Test:

1. `docs/CONCURRENCY.md` exists.
2. It mentions `DaemonStateStore`.
3. It mentions `tokio::spawn`.
4. It mentions `mpsc`.
5. It mentions `Mutex`.
6. It mentions kernel/host mutation serialization.

This is a cheap guardrail.

## 3.6 Search for undocumented shared state

Run:

```bash
rg "Arc<Mutex|Mutex<|RwLock<|tokio::spawn|spawn_blocking|mpsc::channel|oneshot::channel" stutter/src
```

For each result:

1. Decide whether it needs a local comment.
2. Add comments only where the ownership model is non-obvious.
3. Avoid comment spam in tests.

## 3.7 Final verification for change 3

Run:

```bash
cargo test -p stutter architecture_tests
cargo test -p stutter daemon
cargo test -p stutter agent
cargo clippy -p stutter --all-targets -- -D warnings
```

Commit:

```bash
git add docs/CONCURRENCY.md stutter/src/daemon stutter/src/agent.rs stutter/src/architecture_tests
git commit -m "Document daemon concurrency model"
```

---

# 4. Remove the `autotune::candidate` migration facade

## Goal

Remove:

```text
stutter/src/autotune/candidate/mod.rs
```

as an internal migration shim, and route imports to the real owner:

```text
stutter/src/autotune/planning/*
```

Important nuance: `api::autotune::candidate` can stay as a **public API facade** if external callers depend on it. The cleanup target is the internal `crate::autotune::candidate` compatibility module.

## 4.1 Inventory all internal imports

Run:

```bash
rg "autotune::candidate|crate::autotune::candidate|super::candidate|candidate::" stutter/src -g '*.rs'
```

Classify results into:

1. Internal implementation imports.
2. Tests.
3. Public API facade exports in `stutter/src/api.rs`.
4. Architecture dependency allowlists.

## 4.2 Update imports mechanically in small batches

The real modules live under:

```text
stutter/src/autotune/planning/
```

Map old symbols to new modules:

```text
CandidateAction, CandidateFamily, CandidateEvidence, action plans
    -> autotune::planning::candidate

CandidateDryRunRecord, CandidateDryRunner, RealCandidateDryRunner, dry_run_*
    -> autotune::planning::dry_run

CandidateExecutablePlan
    -> autotune::planning::executable_plan

CandidatePlanFile, candidate_plan_path, default_candidate_plan_dir, apply_candidate_plan_file
    -> autotune::planning::plan_io

generate_profile_candidates, GeneratedCpuSetPolicy, etc.
    -> autotune::planning::profile_candidates

CandidateSuggestion, print_candidate_suggestions, suggestions_from_*
    -> autotune::planning::suggestion
```

Do one module group at a time.

Suggested order:

1. `stutter/src/actions/factory.rs`
2. `stutter/src/autotune/experiment.rs`
3. `stutter/src/autotune/workload_policy.rs`
4. `stutter/src/autotune/providers/*`
5. `stutter/src/autotune/apply_low_risk/*`
6. `stutter/src/autotune/live_experiment/*`
7. `stutter/src/autotune/active_config/*`
8. `stutter/src/daemon/privilege/*`
9. tests/support modules
10. `stutter/src/api.rs`

After each group:

```bash
cargo check -p stutter
```

## 4.3 Preserve public API intentionally

Edit:

```text
stutter/src/api.rs
```

Change public re-exports from:

```rust
pub use crate::autotune::candidate::{ ... };
```

to:

```rust
pub use crate::autotune::planning::{
    candidate::{ ... },
    dry_run::{ ... },
    executable_plan::{ ... },
    plan_io::{ ... },
    profile_candidates::{ ... },
    suggestion::{ ... },
};
```

Keep the public module named:

```rust
pub mod candidate
```

That name is okay because it is the external API contract. The internal shim is the problem.

## 4.4 Update architecture dependency matrix

Edit:

```text
stutter/src/architecture_tests/dependencies.rs
```

Replace references to:

```text
autotune::candidate
```

with more precise dependencies:

```text
autotune::planning::candidate
autotune::planning::dry_run
autotune::planning::plan_io
autotune::planning::profile_candidates
autotune::planning::suggestion
```

Or, if the matrix is intentionally coarse:

```text
autotune::planning
```

Prefer the precise version if the test already supports it.

## 4.5 Remove the internal module

Edit:

```text
stutter/src/autotune/mod.rs
```

Remove:

```rust
pub(crate) mod candidate;
```

Delete:

```text
stutter/src/autotune/candidate/mod.rs
```

## 4.6 Add a regression test against facade resurrection

Add an architecture test:

```rust
#[test]
fn autotune_candidate_compatibility_facade_is_removed() {
    assert!(!Path::new("stutter/src/autotune/candidate/mod.rs").exists());
}
```

Better: scan `stutter/src/autotune/mod.rs` and assert it does not contain:

```text
mod candidate;
```

Also scan source for:

```text
crate::autotune::candidate
```

and fail if found.

## 4.7 Final verification for change 4

Run:

```bash
cargo check -p stutter
cargo test -p stutter architecture_tests
cargo test -p stutter
cargo clippy -p stutter --all-targets -- -D warnings
```

Commit:

```bash
git add stutter/src
git commit -m "Remove internal autotune candidate compatibility facade"
```

---

# 5. Section and document the eBPF program

## Goal

Keep the eBPF program in one file if linker constraints make splitting risky, but make the file readable as a set of clear logical sections.

Target file:

```text
stutter-ebpf/src/main.rs
```

## 5.1 Add a top-level file map

At the top of `main.rs`, after crate attributes/imports, add a comment block:

```rust
// Layout:
// 1. Shared constants and map sizing
// 2. Event/map definitions
// 3. Scheduler tracepoints
// 4. Runnable-depth accounting
// 5. Process lifecycle tracepoints
// 6. CPU frequency tracepoints
// 7. Fault counters
// 8. IRQ overlap tracing
// 9. Block I/O tracing
// 10. KMS/flip tracing
// 11. DRM fence tracing
// 12. Tracepoint field readers
// 13. Drop accounting and panic handler
```

## 5.2 Add section dividers

Add visual dividers before each logical group:

```rust
// -----------------------------------------------------------------------------
// Scheduler tracepoints
// -----------------------------------------------------------------------------
```

Suggested sections based on current functions:

1. `Constants and map sizing`
2. `BPF maps`
3. `Shared event structs`
4. `Scheduler entrypoints`
5. `Runnable task accounting`
6. `Target filtering`
7. `Drop accounting`
8. `Process lifecycle`
9. `CPU frequency`
10. `Fault counters`
11. `IRQ overlap`
12. `Block I/O`
13. `KMS flip events`
14. `DRM fence waits/signals`
15. `Tracepoint field readers`
16. `Panic handler`

## 5.3 Add short comments to each public tracepoint

For each `pub fn` tracepoint handler, add a one- or two-line comment:

```rust
/// Tracepoint entry for sched_switch.
/// Keeps runnable-depth state and emits latency data for target tasks.
```

Do this for:

```text
sched_wakeup
sched_wakeup_new
sched_switch
sched_migrate_task
sched_process_exec
cpu_frequency
sched_stat_wait
sched_process_exit
major_fault
minor_fault
irq_handler_entry
irq_handler_exit
block_rq_issue
block_rq_complete
i915_flip_request/done
drm_flip_request/done
drm_vblank_event
amdgpu_flip_request/done
amdgpu_vblank_event
drm_fence_wait_start/done
drm_fence_signal
```

## 5.4 Avoid behavior changes

In this phase:

1. Do not rename maps.
2. Do not move functions across files.
3. Do not alter tracepoint offsets.
4. Do not alter map capacities.
5. Do not alter event structs.
6. Do not change verifier-sensitive code.

This should be documentation/organization only.

## 5.5 Optional low-risk helper reordering

Only reorder private helper functions if it reduces jumping. Keep public tracepoint names stable.

Suggested ordering:

```text
public entrypoint
try_* implementation
private helper used only by that implementation
```

But do this only if tests/builds remain clean.

## 5.6 Add documentation note

Update:

```text
docs/ARCHITECTURE_BOUNDARIES.md
```

Add:

```text
The eBPF crate keeps tracepoint entrypoints in one translation unit for verifier/linking stability. Organization is maintained through internal sections rather than Rust module splitting.
```

## 5.7 Final verification for change 5

Run:

```bash
cargo check -p stutter-ebpf
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

If `stutter-ebpf` has special no-std/clippy constraints, use the existing project command that currently validates it.

Commit:

```bash
git add stutter-ebpf/src/main.rs docs/ARCHITECTURE_BOUNDARIES.md
git commit -m "Document eBPF tracepoint program layout"
```

---

# 6. Decompose `actions/cgroup.rs`

## Goal

Reduce `stutter/src/actions/cgroup.rs` from one 1740-line file into smaller owner modules without changing behavior.

Current internal seams are already visible:

```text
policy/action types
rollback handler
snapshot/procfs reading
preflight validation
path/cpuset validation
file writing
restore/rollback
tests
```

## 6.1 Create the new module directory

Create:

```text
stutter/src/actions/cgroup/
```

Move the old file to:

```text
stutter/src/actions/cgroup/mod.rs
```

Initially make no logic changes.

Update:

```text
stutter/src/actions/mod.rs
```

The existing:

```rust
pub(crate) mod cgroup;
```

should continue to work with `cgroup/mod.rs`.

Run:

```bash
cargo check -p stutter
```

Commit this pure move separately if possible.

## 6.2 Split model/policy types first

Create:

```text
stutter/src/actions/cgroup/model.rs
```

Move:

```rust
CgroupPlacementPolicy
CgroupPlacementTarget
CgroupPlacementAction
CgroupTargetSnapshot
```

Keep visibility minimal:

```rust
pub struct CgroupPlacementPolicy
pub struct CgroupPlacementTarget
pub struct CgroupPlacementAction
pub(super) struct CgroupTargetSnapshot
```

In `mod.rs`:

```rust
mod model;

pub use model::{
    CgroupPlacementAction,
    CgroupPlacementPolicy,
    CgroupPlacementTarget,
};
use model::CgroupTargetSnapshot;
```

Run:

```bash
cargo check -p stutter
cargo test -p stutter actions::cgroup
```

## 6.3 Split file I/O helpers

Create:

```text
stutter/src/actions/cgroup/fs_io.rs
```

Move:

```rust
trait CgroupFileWriter
struct FsCgroupFileWriter
read_trimmed
read_optional_trimmed
write_trimmed
ensure_writable_file
ensure_path_under_root
normalize_cgroup_path
strip_cgroup_leading_slash
```

Visibility:

```rust
pub(super) trait CgroupFileWriter
pub(super) struct FsCgroupFileWriter
pub(super) fn ...
```

Keep test-only fake writer in tests for now, unless it needs access to the trait.

Run checks.

## 6.4 Split validation/preflight

Create:

```text
stutter/src/actions/cgroup/validation.rs
```

Move:

```rust
validate_action_request
validate_target_class
preflight_cgroup_files
ensure_cpuset_available
validate_cpuset_value
```

This module should depend on:

```text
model
fs_io
TaskClass
Path
```

Run checks.

## 6.5 Split procfs/snapshot logic

Create:

```text
stutter/src/actions/cgroup/procfs.rs
```

Move:

```rust
read_target_snapshot_at
identity_warnings
parse_stat_starttime
read_proc_cgroup_path_at
task_exists
```

This module should own process identity reading.

Run checks.

## 6.6 Split rollback/restore logic

Create:

```text
stutter/src/actions/cgroup/rollback.rs
```

Move:

```rust
CgroupRollbackHandler
restore_cpuset_record
cgroup_partial_token
is_dead_task_io_error
```

Also move rollback-specific helper logic from `impl TuningAction` only if it is clearly independent.

Keep `impl RollbackHandler for CgroupRollbackHandler` in this module.

In `mod.rs`, re-export only if needed by the registry:

```rust
pub(crate) use rollback::CgroupRollbackHandler;
```

Run checks.

## 6.7 Keep action implementation in `mod.rs`

Leave in `mod.rs`:

```rust
impl CgroupPlacementAction
impl TuningAction for CgroupPlacementAction
```

This keeps the behavioral core easy to find while helpers are split out.

After the helper split, `mod.rs` should be much smaller and mostly read like:

```text
imports
submodules
public re-exports
action constructors/test hooks
TuningAction impl
```

## 6.8 Split tests last

Create:

```text
stutter/src/actions/cgroup/tests.rs
```

Move the entire `#[cfg(test)] mod tests` block into that file.

In `mod.rs`:

```rust
#[cfg(test)]
mod tests;
```

Update imports inside `tests.rs`:

```rust
use super::*;
use super::fs_io::CgroupFileWriter;
```

If tests need private helpers from submodules, either:

1. Keep helpers `pub(super)`, or
2. Expose test-only wrappers with `#[cfg(test)]`.

Prefer `pub(super)` for helpers that are genuinely cgroup-internal.

Run:

```bash
cargo test -p stutter actions::cgroup
```

## 6.9 Update oversized allowlist

Edit:

```text
stutter/src/architecture_tests/allowlists.rs
```

Remove or shrink the old entry:

```text
src/actions/cgroup.rs
```

Replace with entries only if needed:

```text
src/actions/cgroup/mod.rs
src/actions/cgroup/tests.rs
```

Set strict caps close to actual line counts, for example actual + 25 lines.

Important: do **not** allowlist every new file casually. The goal is to make the oversized exception mostly disappear.

## 6.10 Add module-level docs

Add to:

```text
stutter/src/actions/cgroup/mod.rs
```

A short owner-boundary comment:

```rust
//! Cgroup placement action.
//!
//! Owns audited task migration into configured cgroups and cpuset restoration.
//! Helper modules separate validation, procfs identity checks, filesystem I/O,
//! and rollback handling so mutation sequencing remains reviewable.
```

Add small module docs to:

```text
model.rs
fs_io.rs
validation.rs
procfs.rs
rollback.rs
```

## 6.11 Verify no behavior changed

Run targeted tests:

```bash
cargo test -p stutter actions::cgroup
cargo test -p stutter actions
cargo test -p stutter daemon::privilege
cargo test -p stutter autotune::apply_low_risk
```

Then full checks:

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Commit:

```bash
git add stutter/src/actions/cgroup stutter/src/actions/mod.rs stutter/src/architecture_tests/allowlists.rs
git commit -m "Decompose cgroup action module"
```

---

# Final integration pass

After all six commits exist, do one final review pass.

## A. Search for old raw-score decision mistakes

Run:

```bash
rg "score\\.total|score_total\\(|over_5ms" stutter/src/autotune stutter/src/scorer.rs
```

For each result, classify it:

```text
decision logic: must use normalized values
diagnostic/reporting: raw total is okay
test fixture: okay if intentional
```

Add comments where raw totals are intentionally diagnostic.

## B. Search for removed facade references

Run:

```bash
rg "crate::autotune::candidate|autotune::candidate|super::candidate" stutter/src
```

Expected:

```text
only public api::autotune::candidate naming, if preserved
no internal crate::autotune::candidate imports
```

## C. Search for concurrency-sensitive new code

Run:

```bash
rg "Mutex|RwLock|tokio::spawn|mpsc|oneshot|spawn_blocking|Arc<" stutter/src
```

Expected:

```text
new or existing shared-state boundaries are explained in docs or local comments
```

## D. Re-run architecture tests

```bash
cargo test -p stutter architecture_tests
```

Expected:

```text
file-size allowlists updated
candidate facade removal enforced
objective docs enforced
concurrency docs enforced
```

## E. Re-run full validation

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## F. Final manual review checklist

Before merging, verify:

1. Unequal-length A/B windows compare by normalized rates.
2. Raw totals still appear in diagnostics where useful.
3. `ObjectiveKind` docs match actual code behavior.
4. I/O, IRQ, and thermal objectives can improve based on their own primary signals.
5. `DaemonStateStore` concurrency assumptions are documented.
6. Internal `autotune::candidate` shim is gone.
7. Public API compatibility is intentionally preserved or intentionally broken with release notes.
8. `stutter-ebpf/src/main.rs` is easier to navigate without behavior changes.
9. `actions/cgroup` is split into focused modules.
10. Oversized allowlist no longer hides `cgroup` as one giant file.

That gives you six clean implementation slices, each reviewable on its own, with the correctness-sensitive scoring/objective work handled before the architecture cleanup.
