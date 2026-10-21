# `stutter` engineering plan: correctness → validation → calibration → release

The project should now enter a **consolidation and proof phase**. The audit found that the architecture is already broad and in many places quite mature, but the central measurement/decision loop needs to become more trustworthy before any more major tuning providers or subsystems are added. Pasted text

The plan below addresses all of the issues identified in the audit:

- incorrect/ambiguous autotune objective semantics;
- `GameRunnableLatency` not directly measuring runnable latency;
- compile activity being treated as compile throughput;
- missing objective-level regression contracts;
- documentation/code drift;
- lack of repeatable real-system acceptance testing;
- insufficient real crash/recovery validation;
- incomplete kernel compatibility evidence;
- diagnosis thresholds and score weights lacking empirical calibration;
- too much synthetic evidence relative to real captures;
- unclear feature-maturity boundaries;
- lack of an executable GitHub issue backlog;
- no proper first software-release boundary yet.

The intended order is important:

> **Objective correctness → live end-to-end validation → recovery validation → empirical calibration → supported product boundary → first release.** Pasted text

---

# Phase 0 — Freeze scope and turn the audit into the active backlog

## Goal

Prevent new feature work from expanding the validation burden while the core semantics are corrected.

The audit explicitly recommends postponing more power knobs, VM tuning, IRQ mutation, scheduler work, remote-control expansion, another UI, more telemetry integrations and large architecture refactors. Pasted text

### Step 0.1 — Declare a temporary feature freeze

Add a short section to `docs/ROADMAP.md` or a new development-status section:

```text
Current engineering phase: Core Validation

Until the Core Validation milestone is complete:

Allowed:
- correctness fixes
- objective semantics
- measurement infrastructure
- test/acceptance infrastructure
- recovery/safety fixes
- validation corpus work
- documentation synchronization
- release engineering required by the above

Deferred:
- new mutation providers
- new system-wide tunables
- high-risk autotune
- scheduler replacement/integration
- major remote-control features
- new UI surfaces
- unrelated crate splitting/refactoring
```

This is not a permanent product restriction. It is a development discipline.

### Step 0.2 — Create a GitHub milestone

Suggested milestone:

```text
Core Validation
```

The milestone ends when:

- `GameRunnableLatency` has a real primary signal;
- compile throughput can no longer produce misleading autonomous results;
- all objectives have behavioral contracts;
- live observation acceptance passes;
- live low-risk mutation/rollback acceptance passes;
- crash recovery passes;
- initial real-world calibration corpus exists.

The audit found that the roadmap is large while the issue tracker does not currently represent the actual engineering queue. Pasted text

### Step 0.3 — Create the initial issue set

I would create these immediately:

```text
P0: Define direct GameRunnableLatency objective signal

P0: Prevent compile activity from being treated as compile throughput

P0: Add behavioral contract tests for every autotune objective

P0: Introduce machine-readable objective descriptors

P0: Make AUTOTUNE_OBJECTIVES.md validated/generated from implementation

P1: Add live observation acceptance harness

P1: Add live reversible CPU-affinity acceptance harness

P1: Add crash/startup recovery acceptance matrix

P1: Establish real-kernel compatibility corpus

P2: Establish diagnosis calibration corpus

P2: Calibrate scheduler/IRQ/I/O/GPU diagnosis thresholds

P2: Evaluate and calibrate StutterScore weighting

P2: Track validation evidence provenance and real-evidence coverage

P3: Define supported/experimental/internal capability tiers

P3: Prepare first versioned Stutter release
```

Use labels such as:

```text
priority:P0
priority:P1
correctness
autotune
measurement
safety
validation
kernel
diagnosis
release
```

---

# Phase 1 — Lock the current bugs into regression tests

Do this **before fixing the implementation**.

The point is to ensure you can prove both:

1. why the old behavior was wrong;
2. that it can never silently return.

Use a branch such as:

```text
fix/autotune-objective-semantics
```

---

## Step 1.1 — Add a failing test for `GameRunnableLatency`

The current problem is that `GameRunnableLatency` effectively returns the generic experiment result unless a guardrail rejects it. Pasted text

Construct two windows where:

```text
baseline runnable latency = clearly better
candidate runnable latency = clearly worse

but

generic stutter score = candidate appears better
```

Expected new behavior:

```text
GameRunnableLatency => Regressed
```

Expected current behavior:

```text
GameRunnableLatency => Improved
```

or whatever generic result exposes the mismatch.

This becomes the regression test proving why the change is necessary.

---

## Step 1.2 — Add the inverse runnable-latency test

Create:

```text
baseline:
    runnable p99 = 4.0 ms

candidate:
    runnable p99 = 2.0 ms

generic score:
    approximately unchanged
```

Expected:

```text
Improved
```

This proves that the objective can detect improvement even when the generic score is inconclusive.

---

## Step 1.3 — Add guardrail tests

You need separate tests proving:

```text
runnable latency improves
BUT frame pacing materially regresses
=> Regressed / rejected
```

and:

```text
runnable latency improves
BUT thermal state becomes degraded
=> rejected
```

and similarly for CPU/GPU power-limit guardrails.

This encodes the architectural principle from the audit:

> the primary metric determines whether the objective improved; guardrails determine whether the improvement caused unacceptable damage elsewhere. Pasted text

---

# Phase 2 — Implement a real runnable-latency objective

This is the highest-value technical fix in the project.

---

## Step 2.1 — Define the exact semantics first

Before writing code, define what `GameRunnableLatency` means.

Something like:

> `GameRunnableLatency` measures runnable-to-running scheduler delay experienced by tasks classified as latency-sensitive members of the currently targeted game/process tree during the experiment window.

Decide explicitly which task classes count.

For example:

```text
Game
GameRenderThread
GameWorkerThread
GameHelper
WineServer
GameScope
```

Potentially some should contribute to guardrails but not the primary metric.

Do not leave this implicit.

---

## Step 2.2 — Define the primary statistic

Do **not** average interval p99 values.

For example:

```text
interval A p99 = 1 ms from 10,000 samples
interval B p99 = 20 ms from 4 samples

average interval p99 = 10.5 ms
```

That does not represent the actual distribution.

Instead aggregate the underlying latency data across the entire measurement window.

Preferred hierarchy:

### Best option

Merge latency histograms across the window.

Then derive:

```text
p50
p95
p99
p99.9 if sample volume permits
max
>1 ms rate
>2 ms rate
>5 ms rate
```

### Acceptable alternative

Maintain a window-local streaming latency histogram specifically for objective evaluation.

The important part is that percentile calculations operate over the **combined sample population**.

---

# Phase 3 — Extend `ObjectiveSignals`

The audit recommends first-class runnable-latency fields rather than routing everything through the generic score. Pasted text

Add something like:

```rust
pub struct ObjectiveSignals {
    // existing...

    pub runnable_latency_p95_ns: Option<u64>,
    pub runnable_latency_p99_ns: Option<u64>,
    pub runnable_latency_max_ns: Option<u64>,

    pub runnable_over_1ms: Option<u64>,
    pub runnable_over_2ms: Option<u64>,
    pub runnable_over_5ms: Option<u64>,

    pub runnable_samples: Option<u64>,

    pub runnable_latency_source: Option<String>,
}
```

Add to the quality snapshot:

```rust
pub runnable_latency: ObjectiveSignalQuality,
```

---

## Step 3.1 — Prefer rates to raw counts

Raw:

```text
baseline: 10 over-5ms events
candidate: 11 over-5ms events
```

means very little if:

```text
baseline samples: 1,000
candidate samples: 100,000
```

Therefore derive:

```text
over_5ms_rate =
    over_5ms / runnable_samples
```

Do the same for other thresholds.

---

## Step 3.2 — Keep both percentile and tail-event information

A good scheduler-latency objective probably should not depend on only one number.

For example:

Primary:

```text
p99 runnable latency
```

Secondary:

```text
>5ms event rate
```

Supporting diagnostics:

```text
p95
max
>1ms
>2ms
sample count
```

That avoids a situation where a tiny p99 improvement hides a large increase in catastrophic stalls.

---

# Phase 4 — Define the `GameRunnableLatency` comparator

Do not just write another ad-hoc `if`.

Create a clearly documented comparator.

A reasonable first policy:

```text
1. Validate data quality.

2. Require enough runnable-latency samples.

3. Reject if required runnable signal is unavailable.

4. Check global safety guardrails.

5. Compare p99 runnable latency.

6. Check >5ms rate as a tail guardrail.

7. If primary change is too small for a meaningful verdict:
      Inconclusive.

8. If p99 improves sufficiently and tail rate does not regress:
      Improved.

9. If p99 regresses materially OR severe-tail rate regresses:
      Regressed.

10. Otherwise:
      Inconclusive.
```

Do not define an aggressive improvement threshold yet. Start conservative and calibrate it later.

---

## Step 4.1 — Add sample sufficiency

A p99 based on 40 latency samples is not the same thing as a p99 from 100,000 samples.

Add a policy such as:

```text
minimum_runnable_samples
```

and return:

```text
Inconclusive {
    reason: "insufficient runnable-latency samples"
}
```

rather than pretending the result is meaningful.

---

## Step 4.2 — Preserve fallback behavior only where explicitly intended

If runnable latency is missing, I would **not** let `GameRunnableLatency` quietly become a generic stutter-score objective.

Prefer:

```text
Inconclusive:
"direct runnable-latency signal unavailable"
```

If you retain a fallback at all, make it explicit in the result metadata:

```text
primary_metric_used = generic_score_proxy
confidence = derived/approximate
```

But for an autonomous apply decision, I would require the direct signal.

---

# Phase 5 — Fix compile-throughput semantics immediately

The compile issue is potentially more dangerous because the current metric can reward longer activity. Pasted text

Do this in two stages.

---

# Stage A — Safety correction now

Do not wait until the ideal compile benchmark system exists.

## Step 5.1 — Rename the existing signal conceptually

Change:

```text
compile_progress_intervals
compile_progress_samples
```

toward:

```text
compile_activity_intervals
compile_activity_samples
```

and:

```text
compile_progress_source
```

toward:

```text
compile_activity_source
```

This communicates what the signal actually measures.

---

## Step 5.2 — Change its quality classification

It should not be:

```text
Direct compile throughput
```

because it isn't.

At most it is:

```text
Direct compile activity detection
```

but:

```text
Approximate / Derived
```

for throughput.

---

## Step 5.3 — Prohibit it from producing `Improved`

Until a genuine throughput signal exists:

```text
CompileThroughputWithForegroundProtection
```

must not say:

```text
Improved
```

because there were more build-active intervals.

Instead:

```text
direct throughput unavailable
=> Inconclusive
```

Optionally generic score may be included as supporting evidence:

```text
"interactive latency improved while compile activity was observed,
but direct build-throughput measurement is unavailable."
```

That is honest.

---

## Step 5.4 — Add the critical regression test

Baseline:

```text
compile activity for 20 seconds
```

Candidate:

```text
compile activity for 30 seconds
```

Current proxy might see more active intervals in Candidate.

The test must prove:

```text
candidate MUST NOT be classified as improved throughput
```

This test should remain forever.

---

# Stage B — Implement real compile throughput

The simplest useful implementation is not compiler-plugin integration.

Use **fixed-work completion time**.

---

## Step 5.5 — Introduce workload progress metadata

Create a generic mechanism rather than hardcoding Cargo.

Conceptually:

```rust
WorkloadProgress {
    completed_units: u64,
    total_units: Option<u64>,
    elapsed: Duration,
    source: ProgressSource,
}
```

Possible sources later:

```text
ScenarioCompletion
ExternalMarker
Cargo
Ninja
CMake
CustomCommand
```

But the first implementation can be much simpler.

---

## Step 5.6 — Add benchmark markers

Let a scenario or acceptance harness signal:

```text
WORKLOAD_START
WORKLOAD_COMPLETE
```

For fixed work:

```text
throughput comparison = total completion time
```

Lower completion time is better.

You can later support actual progress rates.

---

## Step 5.7 — Define compile throughput as fixed-work elapsed time first

This is far easier to reason about:

```text
baseline build duration = 31.4 sec
candidate build duration = 28.0 sec

improvement =
(31.4 - 28.0) / 31.4
```

The workload must be identical.

Examples:

```text
same source tree
same target
same clean/incremental state
same job count
same cache state policy
same benchmark command
```

---

## Step 5.8 — Protect foreground responsiveness separately

The objective name is:

```text
CompileThroughputWithForegroundProtection
```

so implement exactly that.

Primary metric:

```text
fixed-work compile completion time
```

Guardrails:

```text
foreground runnable latency
frame pacing, if relevant
thermal state
power-limit state
memory pressure
```

Then the semantics are clean:

```text
Compile finished faster
AND interactive latency did not materially degrade
=> Improved
```

---

# Phase 6 — Establish formal behavioral contracts for every objective

The audit recommends objective regression/contract testing for every objective. Pasted text

Build a reusable test matrix.

For every objective test:

```text
clear improvement
clear regression
no meaningful change
missing primary signal
insufficient data
low-quality data
target disappears
thermal regression
CPU power regression
GPU power regression
generic-score conflict
```

Where applicable.

---

## Proposed matrix

### `StutterScore`

Test:

```text
score/sample decreases
score/sample increases
zero samples
large sample imbalance
```

### `GameFramePacing`

Test:

```text
p99 improves
p99 regresses
frames missing
foreground regresses while p99 improves
```

### `GameRunnableLatency`

Test:

```text
runnable p99 improves
runnable tail rate worsens
frame pacing worsens
signal missing
insufficient sample count
```

### `DesktopInteractivity`

Test:

```text
foreground latency improvement/regression
```

### `BrowserInteractivity`

Same, but with browser-target semantics.

### `CompileThroughputWithForegroundProtection`

Test:

```text
faster fixed work
slower fixed work
equal completion time
foreground regression
throughput signal missing
compile activity alone
```

### `IoLatency`

Test direct count and worst latency.

### `IrqOverlapReduction`

Same pattern.

### `ThermalRecovery`

Test:

```text
degraded → healthy
healthy → degraded
throttle reduction
throttle increase
```

---

# Phase 7 — Introduce a machine-readable objective registry

The audit identified documentation drift: code and `AUTOTUNE_OBJECTIVES.md` disagree about compile progress. Pasted text

The solution is not more documentation.

It is one authoritative metadata source.

---

## Step 7.1 — Create `ObjectiveDescriptor`

Something like:

```rust
pub struct ObjectiveDescriptor {
    pub kind: ObjectiveKind,
    pub primary_metric: &'static str,
    pub primary_signal_quality: ObjectiveSignalQuality,
    pub required_signals: &'static [&'static str],
    pub fallback_policy: ObjectiveFallbackPolicy,
    pub guardrails: &'static [&'static str],
    pub autonomous_apply_allowed: bool,
    pub maturity: ObjectiveMaturity,
}
```

Possible maturity:

```rust
enum ObjectiveMaturity {
    Experimental,
    Validating,
    Supported,
}
```

---

## Step 7.2 — Make the registry exhaustive

Implement:

```rust
ObjectiveKind::descriptor()
```

with an exhaustive `match`.

Adding a new `ObjectiveKind` should fail compilation until its contract is defined.

---

## Step 7.3 — Use it at runtime

Use the same descriptor for:

```text
CLI explanation
API output
TUI
report metadata
autotune decision explanation
```

Then the system can tell the user:

```text
Objective: game_runnable_latency
Primary metric: runnable p99
Signal quality: direct
Fallback: none
Autonomous apply: allowed
Maturity: validating
```

---

## Step 7.4 — Validate documentation from it

Add:

```text
cargo run -p xtask -- objective-docs
```

Two possible modes:

```text
--write
--check
```

Generate the table in:

```text
docs/AUTOTUNE_OBJECTIVES.md
```

CI executes:

```text
cargo run -p xtask -- objective-docs --check
```

If implementation changes but documentation wasn't regenerated:

```text
CI fails
```

That permanently solves this class of drift.

---

# Phase 8 — Finish P0 with a dedicated objective CI gate

Add a CI job or explicit step:

```text
cargo test -p stutter autotune::objective
```

plus:

```text
cargo run -p xtask -- objective-docs --check
```

The important point is visibility.

Objective semantics should be a clearly identifiable CI gate, not merely one tiny part of a 2,000-test suite.

---

# P0 completion gate

Do not start new tunables until all of these are true:

- direct runnable-latency objective exists;
- histogram/window aggregation is correct;
- compile activity cannot masquerade as throughput;
- real compile-throughput semantics exist or the objective remains safely inconclusive;
- every objective has contract tests;
- objective registry is authoritative;
- objective documentation cannot drift;
- current full CI is green.

At this point the **decision model** becomes substantially more trustworthy.

---

# Phase 9 — Build the canonical live acceptance framework

The audit identified this as the next major maturity boundary. Offline and simulated CI are already strong; real privileged system testing remains primarily manual. Pasted text

Create one official entry point:

```text
cargo run -p xtask -- live-acceptance
```

or:

```text
scripts/acceptance/live-core.sh
```

I slightly prefer `xtask`, with shell wrappers only for privilege transitions.

---

# Phase 10 — Split live acceptance into two independent suites

Never make basic profiler validation depend on mutation support.

## Suite A

```text
live-observation
```

## Suite B

```text
live-mutation
```

This allows a system to support:

```text
recording/diagnosis
```

while still failing or disabling:

```text
autotune mutation
```

---

# Phase 11 — Implement live observation acceptance

The audit defines the desired end-to-end sequence. Pasted text

Run:

### 11.1 Preflight

Validate:

```text
Linux
tracefs
required tracepoints
BPF capability
kernel layouts
output directory writable
required userspace dependencies
```

### 11.2 Spawn a deterministic test workload

Do not depend on a real game.

Create something that generates:

```text
known runnable scheduler activity
known CPU contention
optional controlled sleeps
```

A tiny helper binary is ideal.

### 11.3 Start recording

Verify:

```text
eBPF loads
required probes attach
target discovered
target tree remains tracked
```

### 11.4 Generate controlled contention

For example:

```text
target wakes periodically
background CPU workers contend
```

### 11.5 Stop recording

Verify:

```text
clean probe detach
no fatal drops
```

### 11.6 Validate artifacts

Check:

```text
schema version
run metadata
interval records
target identity
drop counters
objective signals
diagnosis
```

### 11.7 Run report pipeline

Generate:

```text
JSON report
HTML report if applicable
advisor output
```

### 11.8 Validate expected evidence

Do not require exact timing.

Use ranges/invariants:

```text
samples > 0
runnable p99 > 0
known target present
expected class present
no malformed artifacts
```

---

# Phase 12 — Build a live low-risk mutation acceptance test

CPU affinity is the ideal first real mutation provider because it is:

- reversible;
- observable;
- easy to verify;
- relatively safe against a disposable process;
- already central to the project's historical workflow.

---

## Step 12.1 — Spawn a disposable process tree

Example structure:

```text
acceptance-parent
 ├── worker-A
 ├── worker-B
 └── worker-C
```

Record:

```text
PID
start time
original affinity
process identity
```

---

## Step 12.2 — Run dry-run

Expected:

```text
correct process count
correct intended masks
no state modification
audit dry-run event written
```

---

## Step 12.3 — Apply a reversible affinity change

Use CPUs that are known online from runtime topology.

Never hardcode masks.

---

## Step 12.4 — Verify

Read actual affinity back from the kernel.

Verify:

```text
expected mask == actual mask
```

---

## Step 12.5 — Validate journal

Before accepting success:

```text
rollback token exists
original state exists
affected identity matches
```

---

## Step 12.6 — Roll back

Restore original affinity.

---

## Step 12.7 — Verify restoration

Do not treat `rollback()` returning `Ok` as sufficient.

Read the kernel state again.

Require:

```text
restored == original
```

---

## Step 12.8 — Validate audit trail

Require events for:

```text
preflight
dry-run
apply
verify
rollback
final result
```

The audit found that the action architecture is already good; the need is to prove it on a real machine rather than redesign it. Pasted text

---

# Phase 13 — Build a crash/recovery acceptance matrix

This is one of the most important safety milestones.

The existing architecture already models startup recovery and rollback journals. Now deliberately break it.

The audit calls out these required failure classes: apply failure, partial apply, verification failure, target exit, daemon death after apply, restart with an outstanding journal, rollback failure, suspend/resume and PID identity issues. Pasted text

---

## Test 13.1 — Process dies before apply

Expected:

```text
apply refused
nothing mutated
journal clean
```

---

## Test 13.2 — Process dies during apply

Expected:

```text
partial mutation detected if applicable
remaining live tasks rolled back
dead tasks reported as skipped
journal reaches safe terminal state
```

---

## Test 13.3 — Force verification failure

Apply succeeds.

Then deliberately make verification fail.

Expected:

```text
automatic rollback
restored state verified
audit records verify failure + rollback
```

---

## Test 13.4 — Kill controller immediately after apply

This is critical.

Sequence:

```text
apply
journal persists rollback state
SIGKILL controller
restart
startup recovery detects outstanding mutation
rollback
verify original state
journal clean
```

If this fails, do not graduate mutation support.

---

## Test 13.5 — Kill controller during verification

Same expectation:

```text
persistent rollback information must survive
```

---

## Test 13.6 — Stale PID reuse

Create rollback state.

Terminate the original process.

Cause or simulate the PID being reused.

Startup recovery must not alter the unrelated replacement process.

Task identity should include enough information beyond integer PID.

---

## Test 13.7 — Suspend/resume

During an experiment:

```text
suspend
resume
```

Expected:

```text
measurement invalidated
new mutation temporarily blocked
stabilization period observed
active experiment rolled back or reset according to policy
```

---

## Test 13.8 — Simulated rollback failure

Use controlled fault injection.

Expected:

```text
system enters explicit fault/emergency state
autotuning stops
no further mutations allowed
manual restoration guidance emitted
audit trail preserved
```

Never silently continue.

---

# Phase 14 — Make live acceptance produce a portable evidence bundle

Each acceptance run should save:

```text
kernel release
/proc/version
distribution info
CPU model/topology
tracepoint preflight result
BPF loader result
Stutter commit
Rust version
Aya revision
acceptance test results
drop counters
artifact schema versions
```

For privacy reasons, sanitize:

```text
usernames
home paths
hostnames
process command lines where unnecessary
tokens
```

This bundle becomes the basis for kernel compatibility evidence.

---

# Phase 15 — Build an explicit kernel compatibility matrix

Runtime kernel validation is one of the project's strong design choices. The tool refuses incompatible tracepoint layouts instead of assuming build-host compatibility. Pasted text

Now collect actual evidence across kernels.

---

## Step 15.1 — Define statuses

Do not just say "supported."

Use:

```text
Observed compatible
Observed degraded
Observed incompatible
Untested
```

---

## Step 15.2 — Record kernel families

Eventually target examples such as:

```text
mainline
NixOS distro kernel
Arch
Fedora
Ubuntu
CachyOS
Zen
Gentoo/custom
```

You do not need them all immediately.

Start with machines you actually have access to.

---

## Step 15.3 — Preserve tracepoint fixtures

For each new kernel:

```text
stutter doctor tracepoints --dump --json
```

then save compatible formats as test fixtures where appropriate.

---

## Step 15.4 — Separate required and optional probe compatibility

Matrix example:

| Kernel | sched core | sched_stat_wait | IRQ | block | KMS/DRM | Overall |
|---|---|---|---|---|---|---|
| A | direct | direct | direct | direct | degraded | compatible |
| B | direct | unavailable | direct | direct | unavailable | compatible/degraded |

This matches Stutter's existing degraded-evidence philosophy much better than a binary yes/no.

---

# Phase 16 — Start empirical diagnosis calibration

Only after objective correctness and live safety validation are solid.

The audit specifically recommends calibration rather than adding more diagnosis categories. Pasted text

The current values such as:

```text
scheduler delay       2 ms
IRQ significance      250 µs
block I/O              1 ms
GPU bounded            95%
frame spike           16.6 ms
low IPC               0.75
cache MPKI             30
```

should be treated as hypotheses to validate, not sacred constants. Pasted text

---

# Phase 17 — Create a validation scenario taxonomy

For each cause, create:

```text
positive scenario
negative control
mixed scenario
```

Example:

## Scheduler-delay candidate

Positive:

```text
latency-sensitive target
+
controlled CPU contention
```

Negative:

```text
same target
+
no contention
```

Mixed:

```text
CPU contention + I/O activity
```

---

## IRQ-delay candidate

Positive:

```text
controlled heavy interrupt source
```

Negative:

```text
similar CPU usage without IRQ load
```

---

## Block-I/O candidate

Positive:

```text
controlled blocking storage activity
```

Negative:

```text
CPU workload with no significant I/O
```

---

## GPU-bound candidate

Positive:

```text
GPU saturation
CPU relatively unconstrained
```

Negative:

```text
CPU-bound workload at moderate GPU load
```

---

# Phase 18 — Label evidence provenance

Every validation fixture should contain metadata:

```text
evidence_kind:
    synthetic
    fixture_real
    real_workload

expected_primary_cause:
    game_thread_scheduler_delay

expected_confidence:
    medium

environment:
    ...

notes:
    ...
```

This implements the audit's recommendation to distinguish synthetic, fixture-real and actual real captured workloads. Pasted text

---

# Phase 19 — Introduce validation coverage reporting

Create something like:

```text
cargo run -p xtask -- validation-coverage
```

Output:

```text
Diagnosis: GameThreadSchedulerDelay
  synthetic:    yes
  fixture-real: yes
  real:         yes

Diagnosis: IrqDelayCandidate
  synthetic:    yes
  fixture-real: no
  real:         no

Objective: GameRunnableLatency
  synthetic:    yes
  controlled-real: yes

Provider: CpuAffinity
  fault-injection: yes
  live rollback:   yes
```

This makes validation debt visible.

---

## Add the "real evidence percentage"

For example:

```text
critical behavior coverage:
  synthetic only:       31%
  fixture-real covered: 44%
  real live covered:    25%
```

Do not turn this into a meaningless vanity metric.

Use it to identify subsystems that are supported primarily by synthetic assumptions.

---

# Phase 20 — Calibrate each diagnosis independently

Do not tune every threshold simultaneously.

For each diagnosis:

1. collect positive cases;
2. collect negative controls;
3. run existing threshold;
4. measure false positives;
5. measure false negatives;
6. sweep nearby threshold values;
7. compare;
8. choose conservative operating point;
9. document evidence.

Example scheduler sweep:

```text
0.5 ms
1.0 ms
1.5 ms
2.0 ms
2.5 ms
3.0 ms
4.0 ms
```

Measure:

```text
true positive rate
false positive rate
confidence classification
primary-cause selection stability
```

---

# Phase 21 — Calibrate confidence, not only thresholds

The more important output may be confidence.

For example:

```text
scheduler delay exists
```

can be highly certain while:

```text
scheduler delay caused this visible frame spike
```

is much less certain.

Calibrate:

```text
Low
Medium
High
```

against evidence quality and number of independent supporting signals.

---

# Phase 22 — Evaluate `StutterScore` scientifically

The audit points out that the score uses hand-selected weighting such as:

```text
>5 ms × 100
>2 ms × 20
>1 ms × 1
```

rather than empirically calibrated weights. Pasted text

Do not immediately replace it with something "more mathematical."

First determine what role the score should play.

---

## Step 22.1 — Define its intended meaning

Possible answer:

> A scalar ranking proxy for scheduler/frame-pacing degradation inside comparable experiments.

Not:

> An absolute user-perceived stutter percentage.

This distinction is important.

---

## Step 22.2 — Test monotonicity

Controlled degradation should generally produce worse scores.

Examples:

```text
0 ms induced delay
1 ms
2 ms
4 ms
8 ms
16 ms
```

Score should behave sensibly.

---

## Step 22.3 — Compare against external metrics

Correlate with:

```text
frame p99
frame p99.9
>16.6ms frames
>33.3ms frames
runnable p99
runnable >5ms rate
```

If possible later:

```text
subjective visible-stutter annotations
```

---

## Step 22.4 — Decide whether the weights actually need change

If the existing score ranks experimental severity reliably, keep it.

Do not optimize coefficients merely because they look arbitrary.

Change only when evidence demonstrates a better model.

---

# Phase 23 — Prevent score calibration from changing objective semantics

Even after calibration:

```text
GameRunnableLatency
```

must still use runnable latency as its primary metric.

Likewise:

```text
CompileThroughput
```

must still use compile throughput.

The generic score remains:

```text
fallback
guardrail
supporting metric
```

not the universal decision oracle.

This distinction was one of the central findings of the audit. Pasted text

---

# Phase 24 — Create explicit feature maturity levels

Once the previous work exists, define product boundaries.

Suggested levels:

```text
Supported
Experimental
Internal
Disabled
```

---

## `Supported`

Requires:

```text
behavioral contract
synthetic regression tests
real acceptance evidence
documented failure behavior
stable artifact contract
diagnostics
```

---

## `Experimental`

May be useful but lacks complete validation.

Must say so clearly.

---

## `Internal`

Architecture/prototype code that users should not rely upon.

---

## `Disabled`

Known unsafe, semantically incomplete or intentionally unavailable.

---

# Phase 25 — Apply maturity levels to current capabilities

The audit suggests a narrow initial supported surface. Pasted text

Likely first supported group:

```text
doctor
record
bench
report
diagnose
advisor
recommend
manual tune evaluation
CPU-affinity dry-run/apply/restore
audit
restore
```

Experimental:

```text
always-on autotune
medium-risk mutation
remote mutation
GPU power mutation
IRQ affinity
VM tuning
system-wide/scheduler changes
```

These can graduate individually later.

---

# Phase 26 — Add a formal provider graduation checklist

Every mutable provider should meet the same standard.

Create something like:

```text
docs/PROVIDER_GRADUATION.md
```

or encode it into architecture tests.

A provider cannot become supported until it has:

```text
[ ] explicit descriptor
[ ] explicit safety class
[ ] explicit effect scope
[ ] preflight
[ ] dry-run
[ ] apply
[ ] verify
[ ] rollback
[ ] durable rollback token if necessary
[ ] audit coverage
[ ] policy denial tests
[ ] partial-apply fault injection
[ ] verification-failure rollback test
[ ] live real-system apply test
[ ] live real-system rollback test
[ ] recovery-after-crash test if persistent state is possible
[ ] documentation generated/validated
```

That stops feature expansion from recreating today's validation gap.

---

# Phase 27 — Prepare the first actual software release

Do this **after P0/P1**, not before.

The audit explicitly recommends waiting until the core contract is trustworthy before treating packaging/release polish as the priority. Pasted text

A sensible first release could be:

```text
v0.1.0
```

provided the supported surface is narrow and explicit.

---

## Step 27.1 — Define release contract

Publish:

```text
Supported
Experimental
Unsupported
Known limitations
Supported Linux architectures
Kernel preflight behavior
Privilege model
Artifact stability promise
```

---

## Step 27.2 — Reproducible build

Make the official release pipeline create:

```text
binary
checksums
source archive
SBOM if practical
release notes
```

Do not need every distro package on day one.

---

## Step 27.3 — Provide one canonical installation path

For example:

```text
Nix
+
source build
```

rather than trying to support six packaging ecosystems immediately.

---

## Step 27.4 — Create one minimal successful tutorial

Not a giant guide.

Something like:

```text
1. install
2. stutter doctor
3. record a workload
4. generate report
5. inspect diagnosis
6. optionally evaluate CPU-affinity experiment
7. restore
```

If a technically competent Linux user cannot complete this reliably, the project is not ready for wider packaging work.

---

# Phase 28 — Perform the first release acceptance run

Before creating the tag:

```text
fresh checkout
fresh development/build environment
build
unit CI
fixture CI
live observation acceptance
live CPU-affinity acceptance
crash recovery acceptance
documentation validation
artifact schema validation
release build
checksum verification
install built release
repeat basic doctor/record/report flow
```

Then tag:

```text
v0.1.0
```

The tag should identify an actually validated source state.

---

# Phase 29 — Only then resume breadth expansion

At this point new providers can be added again.

But they should be selected by value and evidence availability.

I would probably use this approximate order:

```text
1. existing low-risk process-local providers
2. medium-risk reversible providers
3. GPU/power changes with strong hardware-specific verification
4. IRQ affinity
5. VM/system-wide knobs
6. truly high-risk system changes
```

Each must go through the provider-graduation framework.

---

# Recommended PR sequence

I would avoid putting this into one enormous branch.

A much healthier sequence would be:

### PR 1 — Objective regression tests

No semantic fixes yet.

Capture current failures.

---

### PR 2 — Direct runnable-latency window signal

Add aggregation and objective signal representation.

Do not yet alter every UI/report.

---

### PR 3 — `GameRunnableLatency` semantics

Switch comparator to the direct signal.

Add quality/sufficiency handling.

---

### PR 4 — Compile objective safety correction

Rename/demote activity proxy.

Prevent autonomous improvement claims.

---

### PR 5 — Real fixed-work compile throughput

Add scenario completion/progress protocol.

Make compile objective genuinely throughput-based.

---

### PR 6 — Objective descriptor registry

Centralize contracts.

---

### PR 7 — Objective documentation validation

`xtask objective-docs --check`.

Add CI gate.

This closes **P0**.

---

### PR 8 — Live observation acceptance

Real kernel/eBPF/artifact/diagnosis flow.

---

### PR 9 — Live CPU-affinity apply/rollback acceptance

Add disposable-process test.

---

### PR 10 — Crash/recovery acceptance

SIGKILL, stale PID, verification failure, etc.

---

### PR 11 — Kernel compatibility evidence tooling

Standard evidence bundles and compatibility table.

This closes most **P1**.

---

### PR 12 — Validation-corpus metadata

Synthetic/fixture-real/real provenance.

---

### PR 13 — Validation coverage report

Track evidence coverage.

---

### PR 14+ — Diagnosis calibration

One diagnosis family per PR where possible.

---

### PR 15+ — Score calibration

Only after diagnosis corpus is useful.

---

### Final pre-release PR — Capability maturity and release boundary

Supported/experimental classification.

Then prepare `v0.1.0`.

---

# Required CI shape when this work is finished

I would eventually want CI conceptually divided into:

```text
Static / build
    fmt
    clippy
    build
    dependency hygiene
    architecture rules

Objective correctness
    objective contract matrix
    objective docs validation

Offline functional
    recommendation
    advisor
    reports
    artifacts
    validation corpus

Safety
    policy
    action runner fault injection
    journal recovery simulation
    watchdog

Schema
    artifact contracts
    snapshots
    migrations

Live observation
    self-hosted/manual target

Live mutation
    isolated self-hosted/manual target

Release
    packaging
    reproducibility
    install smoke
```

The live jobs do not necessarily need to block every GitHub PR immediately.

But they should become required evidence for:

```text
release candidates
loader changes
mutation changes
kernel-facing changes
```

---

# Cross-cutting requirement: preserve artifact compatibility

Adding objective fields will affect artifacts.

Do not casually mutate serialized structures.

For every schema change:

1. add fields as optional/defaulted where possible;
2. old reports must still deserialize;
3. reports generated from old runs must still render;
4. migration/version behavior must be tested;
5. new fields must clearly expose signal quality.

For example old artifacts lacking runnable metrics should render:

```text
Game runnable latency:
Unavailable in artifact schema/source data
```

not:

```text
0 ns
```

Missing evidence must remain distinguishable from a clean measurement.

That matches one of the project's strongest existing principles. Pasted text

---

# Cross-cutting requirement: measure profiler overhead

As the objective layer starts consuming richer scheduler histograms, make sure the profiler does not substantially perturb the workload.

Add controlled tests for:

```text
recording disabled
core scheduler probes
core + IRQ
core + block
core + GPU
full supported probe set
```

Measure:

```text
CPU overhead
memory
event drop rate
ring-buffer pressure
target performance impact
```

This does not need to become a giant optimization project.

It simply provides evidence that the measurement system is not materially creating the problem it measures.

---

# Cross-cutting requirement: report why a decision happened

Every autotune verdict should eventually expose:

```text
Objective
Primary metric
Baseline value
Candidate value
Difference
Signal quality
Sample count
Guardrails checked
Guardrails triggered
Fallback used
Final verdict
Reason
```

Example:

```text
Objective:
  game_runnable_latency

Primary metric:
  runnable_p99_ns

Baseline:
  3.84 ms

Candidate:
  2.91 ms

Change:
  -24.2%

Signal quality:
  direct

Samples:
  baseline 83,241
  candidate 85,903

Guardrails:
  frame p99: unchanged
  >5 ms runnable rate: improved
  thermal: unchanged
  CPU power limit: no regression
  GPU power limit: no regression

Verdict:
  Improved
```

This is especially useful for an experimental project because it makes incorrect assumptions inspectable.

---

# Cross-cutting requirement: keep observation and mutation separable

Do not let the new acceptance architecture turn into:

```text
Stutter only works if autotuning works.
```

The layers should remain:

```text
observe
↓
diagnose
↓
recommend
↓
experiment
↓
apply
↓
verify
↓
rollback
```

A user should be able to stop at any level.

This separation is already one of the better parts of the current project and should be preserved rather than simplified away.

---

# Definition of the `Core Validation` milestone

I would declare the milestone complete only when all of these are true:

## Measurement

- direct runnable-latency aggregation exists;
- objective signal quality is explicit;
- insufficient data becomes inconclusive rather than a false result;
- profiler overhead has at least basic characterization.

## Objective correctness

- `GameRunnableLatency` genuinely evaluates runnable latency;
- compile activity is no longer throughput;
- real compile throughput exists or remains safely unsupported;
- every objective has a behavioral matrix;
- guardrails are separated from primary metrics.

## Documentation

- objective contracts come from one authoritative registry;
- objective documentation is generated/validated;
- no known code/documentation disagreement remains.

## Live system validation

- real eBPF load/attach works through the canonical acceptance harness;
- real scheduler data reaches artifacts;
- diagnosis/report workflow completes;
- artifacts validate.

## Mutation safety

- CPU-affinity live apply works;
- verify works;
- rollback works;
- restored kernel state is independently checked;
- audits are complete.

## Recovery

- controller crash after apply is automatically recovered;
- stale PID identity is safe;
- verification failure rolls back;
- rollback failure creates an explicit fault state;
- suspend/resume behavior is tested.

## Compatibility

- compatibility evidence exists from multiple real kernel environments;
- degraded optional evidence is represented properly.

## Calibration

- initial real/fixture-real diagnosis corpus exists;
- important heuristics have at least basic calibration evidence;
- validation evidence provenance is tracked.

## Product boundary

- supported vs experimental capabilities are explicit;
- unsupported automation cannot accidentally become default behavior.

Once that is achieved, the project has moved from:

> “large, sophisticated experimental prototype”

to:

> **“experimental tool with a genuinely trustworthy supported core.”**

That is the point where expanding the autotuner becomes worthwhile.

---

# What I would start with tomorrow

If you want the shortest actionable queue from this entire plan, I would literally begin in this order:

1. Create the `Core Validation` milestone and P0/P1 issues.
2. Create `fix/autotune-objective-semantics`.
3. Write the failing `GameRunnableLatency` semantic regression test.
4. Write the failing compile-duration counterexample.
5. Design the window-level runnable latency aggregation format.
6. Implement aggregate runnable latency signals.
7. Switch `GameRunnableLatency` to the direct metric.
8. Demote/rename compile activity and disable false throughput improvement verdicts.
9. Implement fixed-work completion time as the first real compile throughput metric.
10. Add objective descriptors and doc-generation checks.
11. Merge the P0 sequence.
12. Build `xtask live-acceptance observation`.
13. Build `xtask live-acceptance affinity`.
14. Add deliberate SIGKILL/startup-recovery acceptance.
15. Start collecting real validation fixtures.
16. Calibrate one diagnosis family at a time.
17. Define supported/experimental capability tiers.
18. Cut the first actual versioned release.

The key is **not allowing step 15 or the release work to distract from steps 3–14**.

The audit's central finding was that `stutter` already has plenty of architecture. What it needs now is proof that its most important measurement, decision and recovery promises are true in practice. Pasted text

Once that is achieved, the existing daemon, autotune, provider and remote architecture becomes an asset instead of additional validation debt.