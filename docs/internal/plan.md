My current target read is:

| Front                                  | Current | Target after this plan |
| -------------------------------------- | ------: | ---------------------: |
| Data-backed fix proposal / tuning loop |    ~77% |             **86–90%** |
| Real-world validation                  |    ~66% |             **82–86%** |
| Safety / operational readiness         |    ~79% |             **85–89%** |

The core idea is to move from:

```text
diagnosis candidate → advisor suggestion → user manually experiments
```

to:

```text
diagnosis candidate → structured fix hypothesis → generated validation experiment → A/B evidence → validated / rejected / inconclusive fix verdict
```

That is the missing jump.

---

# 1. Data-backed fix proposal / tuning loop

## Current code reality

The current advisor model is intentionally simple:

```rust
pub struct AdvisorRecommendation {
    pub title: String,
    pub rationale: String,
    pub confidence: Confidence,
    pub suggested_commands: Vec<String>,
    pub safety_note: String,
}
```

This lives in:

```text
stutter/src/advisor/models.rs
```

That is useful, but it cannot express a proper data-backed fix proposal. It has no structured intervention, no expected metric movement, no validation recipe, no stop condition, no risk class, and no link to a later A/B result.

The current A/B code is much stronger:

```text
stutter/src/tune/statistics.rs
stutter/src/tune/recommendation.rs
stutter/src/tune/recommendation_html.rs
stutter/src/recommend/builder.rs
stutter/src/recommend/render.rs
```

It already computes:

```text
bootstrap CI
effect size
sample count
noise ratio
underpowered warnings
formal metric comparisons
```

But advisor and tune/recommend are still not tied together as a closed loop.

## Goal

After this work, an advisor output should be able to say:

```text
Hypothesis:
  Game render thread is scheduler-delayed on CPUs 0-3.

Proposed fix:
  Move render/main game threads to isolated CPUs 4-9 using profile X.

Expected metric movement:
  diagnostic_raw_score_total down by >= 5%
  over_5ms samples down by >= 10%
  frame_p99_ms non-regressing

Validation:
  collect 5 baseline runs and 5 test runs under same scenario
  compare with bootstrap CI
  accept only if CI excludes zero on diagnostic_raw_score_total and no safety metric regresses

Verdict after A/B:
  validated / rejected / inconclusive
```

---

## Step 1.1 — Add structured fix hypothesis models

Status: **Completed 2026-06-02.** Added `stutter/src/advisor/fix_plan.rs`, extended advisor recommendations with `fix_plan`, added report-level `fix_plans`, and bumped advisor reports to schema version 2. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter advisor`.

Add a new file:

```text
stutter/src/advisor/fix_plan.rs
```

Add model types:

```rust
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    actions::SafetyClass,
    diagnosis::{Confidence, StutterCause},
    daemon::policy::{ActionEffectScope, RollbackRequirement},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorFixKind {
    CpuAffinityProfile,
    NicePriority,
    IoPriority,
    UClamp,
    CgroupPlacement,
    IrqAffinityInvestigation,
    GpuPowerInvestigation,
    DisplayPathInvestigation,
    BlockIoInvestigation,
    CollectMoreData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorFixValidationStatus {
    NotRun,
    Validated,
    Rejected,
    Inconclusive,
    Underpowered,
    UnsafeToRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorExpectedMetricMovement {
    pub metric: String,
    pub lower_is_better: bool,
    pub minimum_relative_improvement_percent: Option<f64>,
    pub maximum_allowed_regression_percent: Option<f64>,
    pub required_ci_excludes_zero: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorValidationRecipe {
    pub baseline_runs_required: usize,
    pub test_runs_required: usize,
    pub scenario_name: Option<String>,
    pub baseline_command: String,
    pub experiment_command: String,
    pub compare_command: String,
    pub stop_conditions: Vec<String>,
    pub acceptance_criteria: Vec<AdvisorExpectedMetricMovement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorFixPlan {
    pub id: String,
    pub kind: AdvisorFixKind,
    pub cause: StutterCause,
    pub confidence: Confidence,
    pub rationale: String,
    pub safety_class: SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub rollback: RollbackRequirement,
    pub expected_metric_movement: Vec<AdvisorExpectedMetricMovement>,
    pub validation: AdvisorValidationRecipe,
    pub suggested_commands: Vec<String>,
    pub candidate_plan_path: Option<PathBuf>,
    pub safety_notes: Vec<String>,
}
```

Update:

```text
stutter/src/advisor/mod.rs
```

Add:

```rust
mod fix_plan;
```

Expose internally:

```rust
pub(crate) use fix_plan::*;
```

Then extend `AdvisorRecommendation` in:

```text
stutter/src/advisor/models.rs
```

From:

```rust
pub struct AdvisorRecommendation {
    pub title: String,
    pub rationale: String,
    pub confidence: Confidence,
    pub suggested_commands: Vec<String>,
    pub safety_note: String,
}
```

To:

```rust
pub struct AdvisorRecommendation {
    pub title: String,
    pub rationale: String,
    pub confidence: Confidence,
    pub suggested_commands: Vec<String>,
    pub safety_note: String,

    #[serde(default)]
    pub fix_plan: Option<AdvisorFixPlan>,
}
```

Bump advisor report schema:

```rust
schema_version: 2
```

in `build_advisor_report_from_evidence`.

### Tests

Update existing advisor tests by adding `fix_plan: None` only if constructors need it. Since struct literals are mostly in production code, this should be straightforward.

Add tests:

```text
stutter/src/advisor/tests.rs
```

New tests:

```rust
#[test]
fn scheduler_delay_advisor_includes_cpu_affinity_fix_plan() { ... }

#[test]
fn gpu_candidate_advisor_includes_investigation_plan_not_cpu_fix() { ... }

#[test]
fn irq_candidate_advisor_includes_investigation_plan_with_validation_recipe() { ... }
```

Acceptance:

```text
cargo test -p stutter advisor
```

must pass.

---

## Step 1.2 — Generate fix plans from existing diagnosis causes

Status: **Completed 2026-06-02.** Scheduler diagnoses now emit CPU-affinity profile fix plans; GPU, IRQ, and block-I/O diagnoses emit investigation-only plans that preserve the current safety contract. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter advisor`.

Current advisor decision logic already branches on:

```rust
has_scheduler
has_gpu
has_irq_candidate
has_block_io_candidate
```

in:

```text
stutter/src/advisor/engine.rs
```

Add helper builders in `advisor/fix_plan.rs`:

```rust
pub(crate) fn scheduler_profile_fix_plan(
    run: &Path,
    tree_pid: Option<u32>,
    profiles: Option<&Path>,
    evidence: Option<String>,
) -> AdvisorFixPlan
```

For scheduler causes:

```text
StutterCause::GameThreadSchedulerDelay
StutterCause::CompositorSchedulerDelay
```

the fix plan should be:

```text
kind: CpuAffinityProfile
safety_class: ReversibleLowRisk
effect_scope: LocalProcessTree
rollback: RequiredBeforeApply
baseline_runs_required: 5
test_runs_required: 5
acceptance:
  diagnostic_raw_score_total: >=5% improvement, CI excludes zero
  over_5ms: >=10% improvement, CI excludes zero preferred
  frame_p99_ms: maximum allowed regression 5%
```

The command should be generated from already-known fields:

```rust
let pid_arg = tree_pid
    .map(|pid| pid.to_string())
    .unwrap_or_else(|| "<PID>".to_owned());

let profiles_arg = profiles
    .map(|path| path.display().to_string())
    .unwrap_or_else(|| "<profiles.toml>".to_owned());

format!(
    "stutter tune --tree-pid {pid_arg} --profiles {profiles_arg} --runs 5 --baseline-profile baseline-online"
)
```

For GPU-bound causes, do **not** emit CPU-affinity fix. Emit:

```text
kind: GpuPowerInvestigation or DisplayPathInvestigation
safety_class: ObserveOnly
effect_scope: ObserveOnly
validation:
  collect hwmon + drm fence + display-path comparison
```

For IRQ causes, emit:

```text
kind: IrqAffinityInvestigation
safety_class: ObserveOnly for now
effect_scope: Irq
rollback: Unavailable or BestEffortOnly
```

This matches the current safety stance in `docs/DAEMON_CONTRACT.md`, which says IRQ affinity is forbidden by default and manual investigation only.

For block I/O:

```text
kind: BlockIoInvestigation
safety_class: ObserveOnly
```

### Important policy rule

Do **not** create applyable IRQ/GPU/block I/O plans yet. The current daemon contract forbids these by default:

```text
IRQ affinity
GPU power settings
VM knobs
system-wide mutation
```

So the fix plan should clearly distinguish:

```text
applyable experiment
```

from:

```text
investigation-only hypothesis
```

---

## Step 1.3 — Add fix-plan rendering

Status: **Completed 2026-06-02.** Advisor text output now renders fix kind, cause, safety class, effect scope, rollback, policy allowance, expected metric movement, validation recipe, acceptance criteria, stop conditions, and candidate plan path. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter advisor`.

Update:

```text
stutter/src/advisor/render.rs
```

Currently it prints:

```text
title
rationale
confidence
safety
command
```

Add structured sections:

```text
fix kind
cause
safety class
effect scope
rollback
expected metric movement
validation recipe
acceptance criteria
stop conditions
candidate plan path
```

Example markdown output:

```md
### Fix hypothesis

- kind: cpu_affinity_profile
- cause: GameThreadSchedulerDelay
- safety: ReversibleLowRisk
- rollback: RequiredBeforeApply

Expected metric movement:

| Metric | Target |
|---|---|
| diagnostic_raw_score_total | >= 5% lower, CI excludes zero |
| over_5ms | >= 10% lower |
| frame_p99_ms | no >5% regression |

Validation:

1. Collect 5 baseline runs.
2. Run the suggested tune experiment for 5 runs.
3. Compare with `stutter recommend --baseline ... --tune ... --html ...`.
4. Accept only if the diagnostic CI excludes zero and frame metrics do not regress.
```

Add tests asserting advisor text contains:

```text
Expected metric movement
Validation
diagnostic_raw_score_total
CI excludes zero
```

---

## Step 1.4 — Add machine-readable fix-plan output files

Status: **Completed 2026-06-02 via inline JSON.** `stutter advisor --run <run> --json` includes report-level `fix_plans` and each recommendation’s `fix_plan`; standalone export remains a later convenience, not required for the closed-loop schema. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter advisor`.

Right now `advisor` can output JSON or text. Add optional fix plan files under the run directory or state dir.

Add to `AdvisorReport`:

```rust
#[serde(default)]
pub fix_plans: Vec<AdvisorFixPlan>,
```

Then when building the report, collect every recommendation’s `fix_plan`.

Add CLI option:

```text
stutter advisor --write-fix-plans
```

or keep it automatic in JSON only.

Better design:

```text
stutter advisor --run <run> --json
```

should include fix plans inline.

Later:

```text
stutter advisor export-fix-plan --run <run> --out <path>
```

can write standalone plan files.

Files:

```text
stutter/src/cli/advisor.rs
stutter/src/cli/parse/reports.rs or relevant parse module
stutter/src/advisor/command.rs
```

Use schema:

```text
advisor_fix_plan.schema_version = 1
```

Potential artifact path:

```text
<run-dir>/advisor_fix_plan_<id>.json
```

or:

```text
~/.local/state/stutter/advisor/fix_plans/<hash>.json
```

---

## Step 1.5 — Add `stutter validate-fix` or `stutter recommend --fix-plan`

Status: **Completed 2026-06-02 via `recommend --fix-plan`.** The recommend command now accepts `--fix-plan`, loads either a standalone fix-plan JSON or advisor JSON with inline `fix_plans`, and emits `FixValidationReport` JSON/Markdown/HTML with `validated`, `rejected`, `underpowered`, `inconclusive`, or `invalid_experiment` outcomes. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter recommend`.

The missing closed-loop feature is: consume a fix plan and validate whether the experiment succeeded.

Add a new command:

```text
stutter recommend --fix-plan <advisor_fix_plan.json> --baseline <run> --baseline <run> --tune <tune-dir> --html <out>
```

or a clearer new command:

```text
stutter validate-fix --plan <advisor_fix_plan.json> --baseline <run> --baseline <run> --tune <tune-dir> --html <out>
```

Given existing structure, I’d add it under `recommend` first to avoid command sprawl.

Files:

```text
stutter/src/cli/report.rs
stutter/src/cli/parse/reports.rs
stutter/src/recommend/model.rs
stutter/src/recommend/builder.rs
stutter/src/recommend/render.rs
```

Add model:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixValidationReport {
    pub schema_version: u32,
    pub fix_plan: AdvisorFixPlan,
    pub baseline_tune_recommendation: BaselineTuneRecommendation,
    pub status: AdvisorFixValidationStatus,
    pub passed_criteria: Vec<String>,
    pub failed_criteria: Vec<String>,
    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
}
```

Validation logic:

```rust
pub fn validate_fix_plan_against_recommendation(
    plan: &AdvisorFixPlan,
    rec: &BaselineTuneRecommendation,
) -> FixValidationReport
```

Rules:

```text
Validated:
  all required metrics exist
  required metrics have enough_samples = true
  required CI excludes zero when required
  improvement_delta is positive for lower-is-better metrics
  relative improvement meets minimum
  no guardrail metric exceeds max regression

Underpowered:
  any required metric has enough_samples=false
  any required CI missing
  low sample warnings exist

Rejected:
  primary required metric regresses
  required CI excludes zero in wrong direction
  guardrail regression exceeds allowed amount

Inconclusive:
  CI crosses zero but enough samples exist
```

This will move “fix proposal / tuning loop” above 80 because the tool now knows what success means.

---

## Step 1.6 — Add power / sample-size recommendations

Status: **Completed 2026-06-02.** `FormalMetricComparison` now carries `PowerEstimate`, underpowered warnings include estimated runs per side when calculable, and Markdown/HTML A/B rendering shows sample-size guidance. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter power_estimate`.

Current code says:

```text
low sample count: baseline_runs=2 tuned_runs=3 recommended_each=5
```

That is helpful but generic.

Add to:

```text
stutter/src/tune/statistics.rs
```

New model:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerEstimate {
    pub target_relative_improvement_percent: f64,
    pub estimated_runs_per_side: Option<usize>,
    pub reason: String,
}
```

Extend `FormalMetricComparison`:

```rust
#[serde(default)]
pub power_estimate: Option<PowerEstimate>,
```

Add conservative function:

```rust
pub fn estimate_runs_per_side_for_effect(
    baseline_values: &[f64],
    tuned_values: &[f64],
    target_relative_improvement_percent: f64,
) -> PowerEstimate
```

Keep it simple and honest. You do not need a full formal power package. A useful first implementation:

```text
pooled stddev
baseline median
target absolute delta
standardized effect = delta / pooled_stddev
estimated n per side using normal approximation
clamp to [3, 30]
```

Formula approximation:

```rust
// two-sided alpha 0.05, power 0.80
// n per group ≈ 2 * ((z_alpha + z_beta) / effect)^2
// z_alpha=1.96, z_beta=0.84
let n = 2.0 * ((1.96 + 0.84) / effect).powi(2);
```

Warnings:

```text
"estimated 8 runs per side needed to detect 10% improvement at current noise"
```

Render in:

```text
stutter/src/tune/uncertainty_html.rs
stutter/src/tune/recommendation.rs
stutter/src/recommend/render.rs
```

Add tests:

```rust
#[test]
fn power_estimate_recommends_more_runs_for_noisy_small_effect() { ... }

#[test]
fn power_estimate_is_unavailable_for_zero_variance_or_missing_samples() { ... }
```

This directly fixes the current “underpowered but not actionable” gap.

---

## Step 1.7 — Unify ranking confidence with formal A/B where possible

Status: **Completed 2026-06-02.** `stutter tune` recommendations now downgrade high/medium rank-confidence winners to `NeedsRetest` when the formal diagnostic-score comparison is missing, underpowered, insignificant, or not positive. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter formal_diagnostic_score_blocks_recommended_verdict_when_underpowered` and `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter medium_confidence_produces_recommended_verdict`.

Current ranking path:

```text
stutter/src/tune/ranking.rs
```

uses:

```text
median rank
IQR gate
0.5σ normalized effect-size gate
```

Formal A/B path:

```text
stutter/src/tune/statistics.rs
```

uses:

```text
bootstrap CI
effect size
noise ratio
underpowered warnings
```

These should not fight.

Add a new type:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormalRankingDecision {
    pub best_profile: String,
    pub compared_against: String,
    pub metric: String,
    pub verdict: TuneRecommendationVerdict,
    pub reason: String,
    pub formal_metric: FormalMetricComparison,
}
```

Add function:

```rust
pub fn formal_ranking_decision_between_profiles(
    best: &[TuneCandidateSummary],
    second: &[TuneCandidateSummary],
) -> FormalRankingDecision
```

Then use it in:

```text
stutter/src/tune/recommendation.rs
```

At minimum, add a warning if:

```text
ranking says High/Medium
but formal diagnostic_raw_score_total says underpowered or CI crosses zero
```

This is partly already happening in `recommend`, but not fully unified inside `tune` ranking.

Acceptance:

```text
A tune result cannot be "Recommended" if its formal diagnostic score metric is underpowered or CI crosses zero.
```

The `recommend` path already blocks recommendation on formal score:

```rust
formal_score_blocks_recommendation
```

Mirror that discipline in `stutter tune` recommendation.

---

## Step 1.8 — Add direct “validated fix” HTML section

Status: **Completed 2026-06-02.** `recommend --fix-plan --html` renders a dedicated `Fix validation` HTML section with hypothesis, fix kind, expected movement, actual movement, pass/fail per metric, and final status. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter recommend`.

Enhance:

```text
stutter/src/recommend/render.rs
stutter/src/tune/uncertainty_html.rs
```

Add section:

```html
<section id="fix-validation">
  <h2>Fix validation</h2>
  <p>Status: Validated / Rejected / Underpowered / Inconclusive</p>
  ...
</section>
```

For `recommend --fix-plan`, show:

```text
hypothesis
fix kind
expected movement
actual movement
CI
pass/fail per metric
final status
```

Tests:

```text
stutter/src/recommend/tests.rs
```

Assert HTML contains:

```text
Fix validation
Validated
diagnostic_raw_score_total
expected
actual
```

---

## Step 1.9 — Add docs for the closed loop

Status: **Completed 2026-06-02.** Added `docs/TUNING_WORKFLOW.md` and linked the closed advisor/tune/recommend proof loop from `docs/SAFETY.md`, `docs/ROADMAP.md`, `docs/ARTIFACT_SCHEMA.md`, and `docs/CONFIGURATION.md`. The docs explicitly state that underpowered or incomparable results do not count as validated fixes.

Update:

```text
docs/SAFETY.md
docs/ROADMAP.md
docs/ARTIFACT_SCHEMA.md
docs/CONFIGURATION.md
docs/TUNING.md or create docs/TUNING_WORKFLOW.md
```

Add canonical flow:

```bash
stutter record --tree-pid <PID> --duration 180 --run-name baseline-a
stutter advisor --run baseline-a --json > advisor.json
stutter tune --tree-pid <PID> --profiles profiles.toml --runs 5 --baseline-profile baseline-online
stutter recommend --fix-plan advisor_fix_plan.json \
  --baseline baseline-a \
  --baseline baseline-b \
  --baseline baseline-c \
  --tune tune-dir \
  --html fix-validation.html
```

Docs must repeat:

```text
A recommendation is validated only if the acceptance criteria pass.
Underpowered means do not apply as proof.
```

---

# 2. Real-world validation

## Current code reality

The validation corpus currently contains 34 run fixtures:

```text
stutter/tests/fixtures/runs/
```

The real fixtures include:

```text
real_amd_gamescope_gpu_bound
real_amd_hyprland_clean
real_block_io_overlap
real_clean_baseline
real_community_rules_classification
real_compositor_scheduler_delay
real_foreground_window
real_game_thread_scheduler_delay
real_gpu_bound_looking
real_intel_kwin_cpu_bound
real_intel_sway_compositor_delay
real_irq_overlap
real_nvidia_gnome_false_positive
real_nvidia_kwin_irq_overlap
real_truncated_low_quality
```

The matrix test exists in:

```text
stutter/src/validation_corpus_tests/real_captures.rs
```

It validates coverage across:

```text
AMD / NVIDIA / Intel
Sway / Hyprland / Gamescope / KWin / GNOME
clean / false-positive / cpu-bound / gpu-bound / irq / compositor
```

The gap is that fixtures are still small, sanitized, partly synthetic-shaped, and there is no false-negative catalogue.

## Goal

Move validation from:

```text
a useful fixture set
```

to:

```text
a maintained real-world validation program
```

with:

```text
coverage matrix
metadata completeness
false positives
false negatives
multi-machine captures
known expected misses
regression gates
validation dashboard
```

---

## Step 2.1 — Add richer fixture metadata schema

Status: **Completed 2026-06-02.** `FixturePlatform` and parsed expectations now support kernel, CPU, topology, display-refresh, and capture-feature buckets; the six real-matrix fixtures require and carry these fields. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter validation_corpus`.

Current `fixture.toml` has:

```toml
[platform]
gpu_vendor = "AMD"
gpu_driver = "amdgpu"
compositor = "Hyprland"
session_type = "wayland"
scenario = "clean"
sanitized_capture_id = "..."
```

Extend this in:

```text
stutter/src/test_fixture_builder/metadata.rs
stutter/src/validation_corpus_tests/expectation.rs
```

Add:

```rust
#[derive(Clone, serde::Serialize)]
pub(super) struct FixturePlatform {
    gpu_vendor: String,
    gpu_driver: String,
    compositor: String,
    session_type: String,
    scenario: String,
    sanitized_capture_id: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    kernel_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kernel_version_bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cpu_vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cpu_topology_bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_refresh_bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    capture_features: Vec<String>,
}
```

Do **not** store exact private machine details. Use buckets:

```text
kernel_version_bucket = "6.8-6.12"
cpu_topology_bucket = "6c12t"
display_refresh_bucket = "120-165hz"
```

Update tests to require these fields only for new v2 real fixtures at first.

---

## Step 2.2 — Add a validation coverage report

Status: **Completed 2026-06-02.** Added a validation coverage report in tests plus `cargo run -p xtask -- fixture-coverage`; `fixture-check` now prints coverage first and fails when required vendor/compositor/scenario cells disappear. Verified with `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-coverage` and `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter validation_corpus`.

Create:

```text
stutter/src/validation_corpus_tests/coverage.rs
```

Or production-side helper:

```text
stutter/src/validation_corpus/coverage.rs
```

Model:

```rust
#[derive(Debug, Serialize)]
pub struct ValidationCoverageReport {
    pub real_fixture_count: usize,
    pub synthetic_fixture_count: usize,
    pub vendors: BTreeMap<String, usize>,
    pub compositors: BTreeMap<String, usize>,
    pub scenarios: BTreeMap<String, usize>,
    pub kernels: BTreeMap<String, usize>,
    pub known_false_positive_count: usize,
    pub known_false_negative_count: usize,
    pub missing_cells: Vec<String>,
}
```

Add xtask command:

```text
cargo run -p xtask -- fixture-coverage
```

or extend:

```text
cargo run -p xtask -- fixture-check
```

Current `xtask fixture-check` only runs:

```text
cargo test -p stutter validation_corpus
```

Add coverage output:

```text
vendors: AMD=...
compositors: ...
missing: AMD+GNOME+false-negative
```

Files:

```text
xtask/src/main.rs
stutter/src/validation_corpus_tests/
```

Acceptance:

```text
fixture-check prints coverage matrix and fails if required cells disappear.
```

---

## Step 2.3 — Add false-negative catalogue support

Status: **Completed 2026-06-02.** Fixture metadata now supports `expected_behavior = "known_miss"` and known misses fail if the expected diagnosis starts appearing, with an instruction to reclassify the fixture. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter validation_corpus`.

Current search for false-negative terms returns nothing. Add explicit support.

Define new metadata:

```toml
[expected.known_limitation]
kind = "false_negative"
reason = "The current model does not yet distinguish X from Y"
must_not_regress = true
```

Or cleaner:

```toml
[expected.diagnosis]
expected_behavior = "must_diagnose" | "must_not_diagnose" | "known_miss" | "informational"
```

Add enum:

```rust
#[derive(Debug, Deserialize)]
pub enum ExpectedDiagnosisBehavior {
    MustDiagnose,
    MustNotDiagnose,
    KnownMiss,
    Informational,
}
```

Files:

```text
stutter/src/validation_corpus_tests/expectation.rs
stutter/src/test_fixture_builder/metadata.rs
stutter/src/validation_corpus_tests/assertions.rs
```

Add tests:

```rust
#[test]
fn validation_corpus_known_false_negative_is_tracked_not_silently_passed() { ... }
```

Policy:

```text
KnownMiss fixtures pass only if they remain listed as known misses.
If a known miss starts diagnosing correctly, test should fail with instruction to reclassify it.
```

This sounds backwards, but it prevents silent drift. The test message should say:

```text
known miss is now diagnosed; update metadata from known_miss to must_diagnose
```

This gives you a real false-negative catalogue.

---

## Step 2.4 — Add minimum real fixture gates

Status: **Completed 2026-06-03.** Release readiness now has `real_validation_matrix`, `false_negative_catalogue`, and `multi_machine_validation` inputs, with low-risk stable requiring the real matrix and false-negative catalogue evidence. The committed validation corpus now satisfies the final maturity targets directly: 20 real fixtures, 3 false-positive fixtures, 3 known false-negative fixtures, 20 distinct sanitized capture IDs, and no fixture-coverage maturity/privacy warnings. Verified with `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-coverage --html target/fixture-coverage.html` and `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-check`.

Current matrix requires scenarios and compositors for the six new fixtures.

Add stronger gates:

```rust
const MIN_REAL_FIXTURES: usize = 20;
const MIN_FALSE_POSITIVE_FIXTURES: usize = 3;
const MIN_KNOWN_FALSE_NEGATIVE_FIXTURES: usize = 3;
const MIN_DISTINCT_CAPTURE_IDS: usize = 20;
```

But introduce gradually:

```text
Phase A: warning-only coverage report
Phase B: required in experimental validation gate
Phase C: required in low-risk-stable release gate
```

Do not break current development immediately.

Add to release readiness:

```text
stutter/src/release.rs
```

New inputs:

```rust
pub real_validation_matrix: bool,
pub false_negative_catalogue: bool,
pub multi_machine_validation: bool,
```

For `low-risk-stable`, require:

```rust
real_validation_matrix
false_negative_catalogue
```

Maybe keep `multi_machine_validation` required only for production packaging / medium-risk.

---

## Step 2.5 — Add fixture capture manifest

Status: **Completed 2026-06-02.** Added validation-corpus contributor guidance to `docs/VALIDATION_CORPUS.md`, a template at `docs/examples/validation_fixture_manifest.toml`, and a contributor note in `CONTRIBUTING.md`. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter validation_corpus`.

Create:

```text
docs/VALIDATION_CORPUS.md
```

Describe how to add a real fixture safely:

```text
1. Record run.
2. Strip usernames, hostnames, full paths, window titles.
3. Preserve buckets: GPU vendor, driver, compositor, kernel bucket, CPU topology bucket.
4. Assign sanitized_capture_id.
5. Mark expected behavior.
6. Run fixture regeneration.
7. Run fixture-check and fixture-coverage.
```

Add a template:

```text
docs/examples/validation_fixture_manifest.toml
```

Example:

```toml
sanitized_capture_id = "external-amd-hyprland-scheduler-v1"
operator_id = "external-001"
hardware_bucket = "amd-gpu-8c16t"
kernel_version_bucket = "6.10-6.12"
game_workload_bucket = "dxvk-ue4-open-world"
privacy_reviewed = true
```

Do not store exact game title unless allowed. Bucket is enough for validation diversity.

---

## Step 2.6 — Add validation corpus dashboard artifact

Status: **Completed 2026-06-02.** `cargo run -p xtask -- fixture-coverage --html target/fixture-coverage.html` now writes a self-contained validation corpus dashboard with coverage counts, fixture counts, false positives, known misses, data-quality distribution, platform distributions, privacy warnings, and warning-only maturity targets. Verified with `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-coverage --html target/fixture-coverage.html`.

Add command:

```text
stutter validate-corpus --html validation_corpus_report.html
```

or xtask:

```text
cargo run -p xtask -- fixture-coverage --html target/fixture-coverage.html
```

Render:

```text
coverage matrix
fixture counts
false-positive fixtures
known misses
data quality distribution
platform distribution
privacy checks
```

This is especially useful for project maturity.

Files:

```text
xtask/src/main.rs
stutter/src/validation_corpus_tests/coverage.rs
```

Keep it self-contained HTML like existing recommendation reports.

---

## Step 2.7 — Add real-world A/B validation fixtures

Status: **Completed 2026-06-02.** Added committed `stutter/tests/fixtures/tune_ab/` fixture manifests for validated scheduler affinity, underpowered scheduler affinity, and GPU-bound CPU-affinity rejection cases. Added recommend tests that expand those fixtures into repeated baseline/tune artifacts and validate the full `recommend --fix-plan` status path. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter real_ab_fixture`.

The current real corpus tests diagnosis. It does not yet validate the full tune/recommend loop against real repeated runs.

Add a small fixture set:

```text
stutter/tests/fixtures/tune_ab/
  scheduler_affinity_validated/
    baseline-1/
    baseline-2/
    baseline-3/
    tune/
      tuning_summary.json
      tuning_recommendation.json
  scheduler_affinity_underpowered/
  gpu_bound_cpu_affinity_rejected/
```

Tests:

```text
stutter/src/recommend/tests.rs
```

Add:

```rust
#[test]
fn real_ab_fixture_validates_scheduler_fix_when_ci_excludes_zero() { ... }

#[test]
fn real_ab_fixture_rejects_cpu_affinity_for_gpu_bound_case() { ... }

#[test]
fn real_ab_fixture_marks_underpowered_when_baselines_are_missing() { ... }
```

This ties real-world validation to the exact goal: diagnosis plus A/B proof.

---

## Step 2.8 — Add external contribution checks

Status: **Completed 2026-06-02.** Extended sanitized-real privacy scanning for Linux/macOS/Windows home paths, Steam library paths, hostnames, email-like strings, and public-looking IPv4 addresses; contribution docs now call out fixture metadata and privacy requirements. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter validation_corpus`.

Because the biggest validation gap is breadth, add a contribution path.

Docs:

```text
docs/VALIDATION_CORPUS.md
CONTRIBUTING.md
```

Add checklist:

```text
- fixture.toml complete
- privacy tokens absent
- capture_id unique
- expected behavior set
- not all metadata null
- fixture-check passes
```

Add automated privacy scan extension in:

```text
stutter/src/validation_corpus_tests/assertions.rs
```

Currently sanitized-real scans for:

```text
/home/
users/
hostname
steamapps/common
```

Extend forbidden tokens:

```text
C:\Users\
/Users/
.local/share/Steam/steamapps
actual hostname markers from session metadata
email-like regex
IP addresses except local/private buckets maybe
```

Keep it careful to avoid false positives.

---

# 3. Safety / operational readiness

## Current code reality

The safety architecture is strong:

```text
DaemonPolicy
ActionDescriptor
ActionEffectScope
RollbackRequirement
ActionRunner
rollback tokens
emergency restore
service doctor
release gates
docs/DAEMON_CONTRACT.md
```

But packaging is explicitly not production-ready:

```text
docs/PACKAGING.md
docs/INSTALL.md
packaging/gentoo/stutter-9999.ebuild
```

`release.rs` has packaging gates, but they are advisory:

```rust
gate("production_distro_packaging", false, ...)
gate("reproducible_packaged_ebpf_object", false, ...)
gate("packaging_install_tests", false, ...)
gate("packaging_service_smoke_tests", false, ...)
gate("versioned_release_tarball", false, ...)
```

To move safety/ops above 80, you do not need perfect production distro packaging, but you do need stronger local-install and service readiness evidence.

---

## Step 3.1 — Add operational readiness channels

Status: **Completed 2026-06-02.** Release readiness now tracks operational inputs for local install, service doctor/start-stop smoke, emergency restore smoke, unprivileged report smoke, package layout, packaging service smoke, packaged eBPF artifacts, install tests, and versioned release tarballs separately from the safety release channel. Observe/low-risk gates require the relevant smoke evidence, while production distro packaging remains an explicit advisory gate until all packaging evidence exists. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter release_readiness`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter low_risk_stable_requires_real_validation_matrix_and_false_negative_catalogue`, `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- service-smoke`, `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- package-layout-check`, and `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- local-install-smoke`.

Current channels:

```rust
Experimental
ObserveStable
LowRiskStable
MediumRisk
```

Add a separate packaging readiness concept:

```rust
pub enum OperationalReadinessLevel {
    SourceOnly,
    LocalInstallSupported,
    ServiceUnitsSupported,
    DistroPackagingPreview,
    ProductionDistroPackage,
}
```

Or add release inputs:

```rust
pub local_install_smoke_tests: bool,
pub service_doctor_smoke_tests: bool,
pub emergency_restore_smoke_tests: bool,
pub unprivileged_report_smoke_tests: bool,
pub packaged_artifact_layout_tests: bool,
```

For `observe-stable`, require:

```text
local_install_smoke_tests
service_doctor_smoke_tests
unprivileged_report_smoke_tests
```

For `low-risk-stable`, require:

```text
emergency_restore_smoke_tests
service_start_stop_smoke_tests
real_machine_validation
soak_tests
```

Files:

```text
stutter/src/release.rs
stutter/src/commands/release.rs
stutter/src/cli/report.rs
stutter/src/cli/parse/reports.rs
stutter/src/cli/tests/report/args.rs
docs/RELEASE_CHECKLIST.md
docs/PACKAGING.md
```

Tests:

```text
stutter/src/release.rs
stutter/src/commands/release.rs
```

---

## Step 3.2 — Turn service smoke tests into an xtask gate

Status: **Completed 2026-06-02.** Added `cargo run -p xtask -- service-smoke` to validate systemd/OpenRC service command shape without enabling services. The gate checks low-risk/observe mode pins, emergency-restore `ExecStop` paths, Unix-socket agent defaults, and service HOME/state expectations. Verified with `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- service-smoke`.

Add:

```bash
cargo run -p xtask -- service-smoke
```

This should not actually enable system services. It should test:

```text
systemd unit syntax via systemd-analyze verify when available
OpenRC scripts parse enough to expose required fields
service doctor dry-run for systemd-system/user/openrc
emergency restore dry-run command path exists
unit files reference valid stutter subcommands
environment variables match docs
```

Files:

```text
xtask/src/main.rs
stutter/src/service/tests.rs
```

Current service tests already inspect docs and templates. Extend them from static string checks to command-shape verification.

Add checks:

```text
stutter-autotune-low-risk.service has ExecStop emergency-restore
observe service cannot set apply mode
low-risk service refuses non-low-risk mode
agent service uses Unix socket by default
all services pin HOME to /var/lib/stutter or user equivalent
```

---

## Step 3.3 — Add package layout tests

Status: **Completed 2026-06-02.** Added `cargo run -p xtask -- package-layout-check` to validate tarball manifest paths, systemd/OpenRC unit references, docs referenced by packaging metadata, PKGBUILD documentation installs, and Gentoo ebuild production-readiness wording. Verified with `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- package-layout-check`.

Create:

```text
xtask/src/package_layout.rs
```

or inside current xtask.

Command:

```bash
cargo run -p xtask -- package-layout-check
```

Validate:

```text
packaging/tarball/MANIFEST.txt entries exist in source tree or are generated artifacts
systemd and openrc units referenced by tarball manifest exist
docs referenced by PKGBUILD/ebuild exist
PKGBUILD installs docs/PACKAGING.md and docs/INSTALL.md
Gentoo ebuild does not claim production-ready
```

Add release gate input:

```rust
packaged_artifact_layout_tests
```

---

## Step 3.4 — Make prebuilt eBPF artifact flow testable

Status: **Completed 2026-06-02.** Added `stutter/src/ebpf/artifact_manifest.rs` with `EbpfArtifactManifest`, canonical ABI/map/program metadata, and loader debug metadata. Added `cargo run -p xtask -- ebpf-manifest --object <path> --out <manifest.json>` to hash non-empty prebuilt objects and write a reproducible manifest. Existing loader tests already reject missing/empty runtime overrides without fallback. Verified with a fake object manifest run, `RUSTUP_TOOLCHAIN=nightly cargo test -p xtask ebpf_manifest`, `RUSTUP_TOOLCHAIN=nightly cargo test -p xtask expected_subcommands_are_registered`, and `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter ebpf_artifact_manifest_records_abi_maps_and_programs`.

Docs already support:

```text
STUTTER_USE_PREBUILT_BPF=1
STUTTER_BPF_OBJECT=/usr/lib/stutter/stutter.bpf.o
```

Need stronger test and release artifact plan.

Add tests around build script if not already present:

```text
stutter/build.rs
stutter/src/ebpf/loader.rs
```

Search and add tests for:

```text
missing STUTTER_BPF_OBJECT fails
empty object fails
runtime override does not silently fallback
embedded object metadata is recorded in report
```

Add an artifact manifest:

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct EbpfArtifactManifest {
    pub schema_version: u32,
    pub stutter_version: String,
    pub ebpf_object_sha256: String,
    pub event_abi_version: u32,
    pub map_names: Vec<String>,
    pub program_names: Vec<String>,
}
```

Create:

```text
stutter/src/ebpf/artifact_manifest.rs
```

Add command:

```text
stutter release ebpf-manifest --object <path> --out <manifest.json>
```

or xtask:

```text
cargo run -p xtask -- ebpf-manifest
```

This directly addresses:

```text
reproducible packaged eBPF object
```

without pretending package manager integration is solved.

---

## Step 3.5 — Strengthen rollback drills

Status: **Completed 2026-06-02.** Added `stutter daemon rollback-drill --dry-run [--json]` plus a structured rollback-drill report in `daemon doctor`. The drill checks daemon state readability, controller journal readability, affinity/profile restore files, emergency-restore command path, pending rollback restorable/empty status, and privileged-worker socket status for medium-risk mode. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter rollback_drill` and `RUSTUP_TOOLCHAIN=nightly cargo run -p stutter -- daemon rollback-drill --dry-run --json`.

Current rollback is tested widely, but operational readiness needs a user-facing drill.

Add command:

```text
stutter daemon rollback-drill --dry-run
```

or:

```text
stutter service doctor --rollback-drill
```

It should verify:

```text
restore files readable
controller journal readable
emergency restore command path works
pending rollback state is either empty or restorable
privileged worker socket configured if medium-risk service enabled
```

Files:

```text
stutter/src/commands/daemon/
stutter/src/daemon/doctor.rs
stutter/src/service/
```

Tests:

```text
stutter/src/daemon/tests/
stutter/src/service/tests.rs
```

Acceptance:

```text
service doctor reports rollback-drill status
release low-risk-stable requires rollback drill evidence
```

---

## Step 3.6 — Make safety risk explicit in recommendations and fix plans

Status: **Completed 2026-06-02.** `AdvisorRecommendation` and `AdvisorFixPlan` now carry `AdvisorSafetyRisk`, including safety class, effect scope, rollback, privilege/system-wide/persistence flags, required policy mode, and a default-policy check computed with `DaemonPolicy::check_action`. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter advisor`.

Current `AdvisorRecommendation` has only:

```rust
safety_note: String
```

After Step 1, `AdvisorFixPlan` has:

```rust
safety_class
effect_scope
rollback
```

Add:

```rust
pub safety_risk: AdvisorSafetyRisk,
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorSafetyRisk {
    pub safety_class: SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub rollback_requirement: RollbackRequirement,
    pub requires_privilege: bool,
    pub system_wide: bool,
    pub persistent: bool,
    pub allowed_by_default_policy: bool,
    pub required_policy_mode: String,
}
```

Use `DaemonPolicy::check_action` to compute `allowed_by_default_policy`.

This bridges advisor to actual policy rather than prose.

---

## Step 3.7 — Add “policy proof” to candidate plans

Status: **Completed 2026-06-02.** Candidate plan files are now schema v2 and include `policy_intent`, serialized `policy_explanation`, `dry_run_command`, `apply_command`, and `rollback_command`. Runtime dry-run plan output passes the actual daemon policy so suggest-mode plans do not advertise apply, while high-risk/system-adjacent plans never serialize a direct apply command. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter candidate_plan_file`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter generic_candidate_suggestion_writes_plan_file_and_uses_apply_candidate_command`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter high_risk_system_candidate_suggestion_is_dry_run_only`, and `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter dry_run_all_safe_writes_plan_files_without_starting_experiment`.

There are already generic candidate plan concepts in daemon docs. Ensure every candidate plan includes:

```rust
pub policy_explanation: PolicyExplanation,
```

So a user can see:

```text
why this is allowed
why this is blocked
which mode is required
which rollback path exists
```

Files likely involved:

```text
stutter/src/autotune/planner/
stutter/src/autotune/commands/
stutter/src/daemon/privilege/
stutter/src/actions/runner/
```

Search current candidate plan model and extend it. If there is no central serialized plan type, create one:

```text
stutter/src/autotune/candidate_plan.rs
```

Model:

```rust
pub struct SerializedCandidatePlan {
    pub schema_version: u32,
    pub descriptor: ActionDescriptor,
    pub policy_intent: PolicyIntent,
    pub policy_explanation: PolicyExplanation,
    pub dry_run_command: String,
    pub apply_command: Option<String>,
    pub rollback_command: String,
}
```

Acceptance:

```text
suggest mode output cannot show apply command unless policy permits it
high-risk never shows direct apply command
```

This is already a documented contract in `docs/DAEMON_CONTRACT.md`; make the artifact enforce it.

---

## Step 3.8 — Add local install smoke script

Status: **Completed 2026-06-02.** Added `scripts/smoke-local-install.sh` and the `cargo run -p xtask -- local-install-smoke` wrapper. The smoke installs into a temporary prefix, runs the installed `stutter --version`, dry-runs `service doctor`, and dry-runs `daemon emergency-restore`. Verified with `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- local-install-smoke`.

Create:

```text
scripts/smoke-local-install.sh
```

It should run in a temp prefix:

```bash
PREFIX="$(mktemp -d)"
scripts/install-local.sh --prefix "$PREFIX"
"$PREFIX/bin/stutter" --version
"$PREFIX/bin/stutter" service doctor --dry-run ...
```

Add xtask wrapper:

```text
cargo run -p xtask -- local-install-smoke
```

Release gate:

```rust
local_install_smoke_tests
```

This gets operational readiness above 80 without claiming distro packages are done.

---

## Step 3.9 — Tighten remote agent operational safety

Status: **Completed 2026-06-02.** Agent capabilities and daemon status now expose `remote_transport`, `remote_auth_configured`, and `remote_apply_enabled` through a shared remote-access status. Remote apply is reported enabled only for local transports with apply-capable auth and low-risk limits. Foreground title capture is rejected on unsafe TCP even with a valid token, and default remote limits explicitly reject medium/high-risk apply modes. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter capabilities_report`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter daemon_status_response`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter remote_policy_limits`, and `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter foreground_title_capture`.

Inspect:

```text
stutter/src/remote/
stutter/src/agent/
stutter/src/daemon/policy/remote.rs
```

Add explicit tests:

```text
remote cannot request medium/high risk by default
remote cannot enable foreground title capture in unsafe mode
remote limits cap max duration, targets, safety class
remote apply requires auth + loopback or Unix socket
```

Some of these likely already exist. Add missing ones under:

```text
stutter/src/agent/tests/
stutter/src/remote/tests.rs
stutter/src/daemon/policy_tests/
```

Add report field in `agent status` or `daemon doctor`:

```text
remote_transport = unix_socket / loopback_tcp / unsafe_tcp
remote_auth_configured = true/false
remote_apply_enabled = true/false
```

---

# 4. Cross-front integration: the “proof workflow”

This is the piece that makes all three fronts reinforce each other.

## Step 4.1 — Add scenario identity to run artifacts

Status: **Completed 2026-06-03.** Recording metadata now carries `scenario_name`, `scenario_hash`, `workload_label`, and `route_label`; `record`, `monitor`, `bench`, `scenario run`, and `tune` propagate normalized scenario identity; `recommend` warns/blocks fix validation on baseline/tune scenario mismatches unless `--allow-scenario-mismatch` is set. Bumped session artifacts to schema 23, regenerated validation fixtures, added `docs/examples/artifacts/v23`, and fixed `xtask fixture-update` to run the current ignored generators. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter record_accepts_scenario_identity_flags`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter recommend_fix_plan_blocks_on_scenario_mismatch_unless_allowed`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter validation_corpus`, `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter artifact_contract_tests`, and `RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-update`.

The tune/recommend path needs to know whether baseline/test are comparable. Comparability already checks coverage/sample/frame overlap, but user intent is not encoded enough.

Extend session metadata:

```text
stutter/src/recorder/SessionFile or session metadata model
```

Add:

```rust
pub scenario_name: Option<String>,
pub scenario_hash: Option<String>,
pub workload_label: Option<String>,
pub route_label: Option<String>,
```

The CLI already has scenario code under:

```text
stutter/src/scenario/
```

Use it.

Add CLI flags to `record`, `monitor`, `tune` if missing:

```text
--scenario <name>
--workload-label <label>
```

Then `recommend` should warn or reject validation if:

```text
baseline scenario != tune scenario
```

unless forced.

---

## Step 4.2 — Make comparability a hard validation gate for fix validation

Status: **Completed 2026-06-02.** Fix validation now maps major comparability/drop-counter warnings into `FixValidationBlocker` values and returns `invalid_experiment` instead of treating incomparable A/B evidence as proof. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter recommend`.

Current tune comparability warnings are real:

```text
stutter/src/tune/comparability/
```

But fix validation should treat major comparability failure as blocking.

Add:

```rust
pub enum FixValidationBlocker {
    ScenarioMismatch,
    MajorThreadTopologyShift,
    FrameCountMismatch,
    CoverageTooLow,
    DropCountersNonzero,
}
```

In `validate_fix_plan_against_recommendation`:

```text
if major comparability warning exists:
  status = Inconclusive or RejectedForInvalidExperiment
```

Maybe add:

```rust
AdvisorFixValidationStatus::InvalidExperiment
```

This is important: a fix should not be “validated” if the workload changed.

---

## Step 4.3 — Add one-command guided workflow

Status: **Completed 2026-06-03.** Added top-level `stutter prove-fix --plan <plan> --profiles <profiles.toml> --tree-pid <PID>` workflow rendering. The command accepts direct advisor fix-plan JSON or full advisor report JSON, inherits plan run counts/scenario where available, supports explicit scenario/workload/route labels, and prints baseline recording, tuning, and validation commands that preserve scenario identity across the experiment. Verified with `RUSTUP_TOOLCHAIN=nightly cargo test -p stutter prove_fix`.

Add command:

```text
stutter prove-fix
```

or keep under advisor:

```text
stutter advisor prove --plan <plan> --profiles <profiles.toml> --tree-pid <PID>
```

Given scope, a simple version can just print commands.

Example output:

```text
1. Record baseline:
   stutter record --tree-pid 123 --scenario city-run --duration 180 --run-name city-baseline-1

2. Repeat until 5 baselines exist.

3. Run tuning:
   stutter tune --tree-pid 123 --profiles profiles.toml --runs 5 --baseline-profile baseline-online

4. Validate:
   stutter recommend --fix-plan plan.json --baseline ... --tune ... --html fix-validation.html
```

This alone improves user-facing readiness significantly, because the tool stops leaving the experiment design implicit.

---

# 5. Concrete milestone sequence

## Milestone A — Structured fix plans

Expected score movement:

```text
Data-backed fix proposal / tuning loop: 77 → 81
```

Changes:

```text
stutter/src/advisor/fix_plan.rs
stutter/src/advisor/models.rs
stutter/src/advisor/engine.rs
stutter/src/advisor/render.rs
stutter/src/advisor/tests.rs
docs/ARTIFACT_SCHEMA.md
```

Acceptance:

```bash
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter advisor
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
```

---

## Milestone B — Fix validation report

Expected score movement:

```text
Data-backed fix proposal / tuning loop: 81 → 86
Statistical A/B: small boost
```

Changes:

```text
stutter/src/recommend/model.rs
stutter/src/recommend/builder.rs
stutter/src/recommend/render.rs
stutter/src/recommend/tests.rs
stutter/src/cli/report.rs
stutter/src/cli/parse/reports.rs
```

Acceptance:

```text
recommend --fix-plan produces validated/rejected/underpowered/inconclusive status
HTML includes Fix validation section
tests fail if acceptance criteria are not enforced
```

---

## Milestone C — Power estimates

Expected score movement:

```text
Data-backed fix proposal / tuning loop: 86 → 88
Statistical rigor: 81 → 85
```

Changes:

```text
stutter/src/tune/statistics.rs
stutter/src/tune/uncertainty_html.rs
stutter/src/tune/recommendation.rs
stutter/src/recommend/render.rs
```

Acceptance:

```text
underpowered warning includes estimated runs per side when possible
HTML shows sample-size guidance
```

---

## Milestone D — Validation metadata + false-negative catalogue

Expected score movement:

```text
Real-world validation: 66 → 75
```

Changes:

```text
stutter/src/test_fixture_builder/metadata.rs
stutter/src/validation_corpus_tests/expectation.rs
stutter/src/validation_corpus_tests/assertions.rs
stutter/src/validation_corpus_tests/real_captures.rs
docs/VALIDATION_CORPUS.md
```

Acceptance:

```text
fixture metadata tracks platform buckets
known false-negative fixtures are supported
fixture-check reports known misses
```

---

## Milestone E — Real A/B validation fixtures

Expected score movement:

```text
Real-world validation: 75 → 82
Data-backed tuning loop: +1 or +2
```

Changes:

```text
stutter/tests/fixtures/tune_ab/
stutter/src/recommend/tests.rs
stutter/src/test_fixture_builder/
docs/VALIDATION_CORPUS.md
```

Acceptance:

```text
tests prove validated / rejected / underpowered A/B outcomes
fixtures cover at least one scheduler fix, one GPU non-fix, one underpowered case
```

---

## Milestone F — Operational smoke gates

Expected score movement:

```text
Safety / operational readiness: 79 → 84
```

Changes:

```text
xtask/src/main.rs
xtask/src/package_layout.rs
scripts/smoke-local-install.sh
stutter/src/release.rs
stutter/src/service/tests.rs
docs/RELEASE_CHECKLIST.md
docs/PACKAGING.md
```

Acceptance:

```bash
cargo run -p xtask -- local-install-smoke
cargo run -p xtask -- package-layout-check
cargo run -p xtask -- service-smoke
```

---

## Milestone G — eBPF artifact manifest and release readiness

Expected score movement:

```text
Safety / operational readiness: 84 → 87
```

Changes:

```text
stutter/src/ebpf/artifact_manifest.rs
stutter/src/commands/release.rs
xtask/src/main.rs
docs/PACKAGING.md
docs/RELEASE_CHECKLIST.md
packaging/tarball/
```

Acceptance:

```text
prebuilt eBPF object has manifest
tarball manifest references eBPF manifest
release check can mark reproducible_packaged_ebpf_object with evidence
```

---

## Milestone H — Rollback drill and service doctor integration

Expected score movement:

```text
Safety / operational readiness: 87 → 89
```

Changes:

```text
stutter/src/daemon/doctor.rs
stutter/src/commands/daemon/
stutter/src/service/
stutter/src/service/tests.rs
docs/SAFETY.md
docs/DAEMON_CONTRACT.md
```

Acceptance:

```text
service doctor reports rollback drill status
low-risk-stable release gate can require rollback drill evidence
```

---

# 6. Final acceptance matrix

To say all three weak fronts are above 80%, I would require these concrete checks.

## Data-backed fix proposal / tuning loop >80%

Required:

```text
AdvisorRecommendation has structured fix_plan.
Fix plans include expected_metric_movement, validation recipe, stop conditions, safety risk.
recommend --fix-plan validates or rejects the fix using formal A/B metrics.
HTML report shows fix validation status.
Underpowered result is not allowed to count as validated.
Comparability failures block validation.
```

Test names to add:

```rust
scheduler_delay_advisor_includes_cpu_affinity_fix_plan
gpu_candidate_advisor_includes_investigation_plan_not_cpu_fix
recommend_fix_plan_validates_when_required_ci_excludes_zero
recommend_fix_plan_rejects_when_metric_regresses
recommend_fix_plan_marks_underpowered_when_ci_missing
recommend_fix_plan_blocks_on_comparability_failure
```

---

## Real-world validation >80%

Required:

```text
real fixture count >= 20
false-positive fixtures >= 3
known false-negative fixtures >= 3
metadata buckets exist for new real captures
fixture coverage report exists
real A/B validation fixtures exist
privacy scan covers paths, hostnames, usernames, email-like strings
```

Test names to add:

```rust
validation_corpus_tracks_known_false_negative_cases
validation_corpus_real_metadata_has_platform_buckets
validation_corpus_coverage_reports_required_matrix
real_ab_fixture_validates_scheduler_fix
real_ab_fixture_rejects_cpu_affinity_for_gpu_bound_case
```

---

## Safety / operational readiness >80%

Required:

```text
local install smoke gate
service smoke gate
package layout gate
rollback drill
eBPF artifact manifest
release readiness tracks these as non-advisory for low-risk-stable
remote safety tests cover risky request paths
```

Test / command names:

```text
cargo run -p xtask -- local-install-smoke
cargo run -p xtask -- service-smoke
cargo run -p xtask -- package-layout-check
stutter daemon rollback-drill --dry-run
stutter release check --channel low-risk-stable --soak-tests --real-machine-validation --real-validation-matrix --false-negative-catalogue --local-install-smoke-tests --service-doctor-smoke-tests --emergency-restore-smoke-tests --unprivileged-report-smoke-tests --packaged-artifact-layout-tests --service-start-stop-smoke-tests --rollback-drill --enforce
```

---

# 7. Recommended order

Do it in this order:

1. **Structured advisor fix plans**
2. **Fix validation report**
3. **Power/sample-size estimates**
4. **False-negative catalogue**
5. **Real A/B validation fixtures**
6. **Fixture coverage dashboard**
7. **Operational smoke gates**
8. **eBPF artifact manifest**
9. **Rollback drill**
10. **Release gate tightening**

That order is deliberate. The first three improve the core product promise. The next three make the validation claim credible. The final four make the tool safer to ship and operate.

After these are in place, I’d score the three weak fronts roughly like this:

| Front                                  | After plan |
| -------------------------------------- | ---------: |
| Data-backed fix proposal / tuning loop |    **88%** |
| Real-world validation                  |    **83%** |
| Safety / operational readiness         |    **87%** |

And the overall “diagnose, prove, propose, and validate a fix” goal would move from **~84–85%** to about **~89–91%**.
