# Evidence-Based Linux Game-Performance Tuning with `stutter`

## Abstract

Linux game performance tuning is often driven by anecdote: players change CPU
affinity, scheduler settings, compositor options, Wine/Proton flags, or driver
settings and judge the result from short play sessions or average FPS. This
project explores a different approach: an evidence-based Linux game-performance
tuning tool that collects scheduler, frame-timing, process-tree, GPU, and data
quality evidence before making tuning recommendations. The prototype, `stutter`,
uses scheduler-aware eBPF profiling, structured artifacts, advisor-generated
hypotheses, reversible tuning profiles, and repeated A/B validation to decide
whether a proposed change is supported.

The main evaluation is a real Kingdom Come: Deliverance 1 workload under
GE-Proton10-34, Gamescope, and Sway/Wayland. `stutter` captured valid baseline
runs, generated a plausible CPU-affinity hypothesis, tested it against an
online-baseline profile, and declined to recommend the tuned profile when the
A/B evidence did not support it. The tuned profile was not validated:
`baseline-online` had a lower primary diagnostic score in all five paired
iterations. This negative tuning result is the central positive result of the
project: the tool completed the evidence loop and avoided a false-positive
tuning recommendation.

## 1. Introduction

Linux game performance problems are not only average-FPS problems. A game can
report a reasonable mean frame rate while still feeling uneven because of frame
pacing spikes, p99 frametime regressions, scheduler delay, presentation stalls,
or noisy background work. These effects are especially visible in complex
Proton/Wine workloads, where a single game session may involve the game process,
Wine helper processes, Gamescope, Steam runtime tasks, driver work, and display
presentation paths.

Manual tuning is common in this environment. Players try CPU-affinity masks,
custom schedulers, launch options, compositor changes, memory allocators, and GPU
driver flags. Some changes may help under some workloads, but short subjective
tests are weak evidence. A tuning workflow that accepts every plausible story is
dangerous because noisy workloads can make unsupported changes look convincing.

The goal of this project is to build and evaluate `stutter`, a Linux
game-performance profiling and tuning prototype that supports evidence-based
tuning rather than magic tweaks. The tool collects runtime evidence, produces
scoped hypotheses, applies only guarded and reversible experiments, and reports
when evidence is not strong enough to recommend a change.

The research question is:

> Can a Linux game-performance tool collect enough evidence from a real Proton
> workload to generate, test, and validate or reject a tuning hypothesis?

The main answer from the evaluation is yes, with an important qualification. In
the KCD1 case study, the specific CPU-affinity hypothesis was not validated and
should not be recommended on current evidence. That is still a successful
workflow result because the project is designed to avoid unsupported
recommendations.

## 2. Background

### 2.1 Runnable Latency

`stutter` measures scheduler runnable latency:

```text
sched_wakeup timestamp -> sched_switch timestamp = runnable latency
```

This asks whether a task was ready to run but waited before receiving CPU time.
For games, this is useful because frame pacing can be affected by short periods
where important game, render, audio, compositor, or runtime tasks are runnable
but delayed.

Runnable latency is not the only possible source of stutter. GPU saturation,
shader compilation, display presentation, I/O, compositor behavior, and game
engine work can all matter. The point of measuring runnable latency is not to
claim every frame issue is a scheduler issue. It is to add scheduler-visible
evidence to a broader frame and process analysis.

### 2.2 Proton, Wine, and Process Trees

Windows games running through Proton are not simple single-process Linux
applications. Wine, Proton, Steam runtime components, helper processes, DXVK
threads, audio threads, streaming threads, and compositor processes can all be
part of the active workload. A tuning tool therefore needs process-tree and task
classification rather than only one PID.

This complexity is a motivation for `stutter`'s explicit targeting model.
Commands can target a live process tree, record per-task evidence, and classify
tasks by role. For the KCD1 case study, explicit `--tree-pid` targeting was used
for the KCD1/Gamescope process tree.

### 2.3 Gamescope, MangoHud, and Frame Timing

Gamescope and MangoHud are common parts of technical Linux gaming setups.
MangoHud can provide frame-timing logs, while Gamescope creates a more explicit
presentation/runtime environment. `stutter` ingests MangoHud frame data so that
scheduler evidence can be interpreted alongside frame pacing rather than in
isolation.

Average FPS is insufficient for this project because the relevant problem is
often tail behavior: p95 or p99 frametime, long frame outliers, and repeated
small stalls that affect perceived smoothness. The KCD1 baseline measurements
show this clearly: the route produced usable but noisy runs, with median
frametime varying meaningfully between repeated measurements of the same route.

### 2.4 Why A/B Testing Matters

Real games are noisy workloads. Open-world traversal, asset streaming,
background runtime behavior, shader state, and OS scheduling can vary between
runs. A single tuned run cannot prove a tuning change helped. `stutter` treats
recommendations as experiments: a diagnosis produces a structured hypothesis,
and repeated comparable baseline/tuned measurements decide whether that
hypothesis is supported.

The trusted loop is:

```text
diagnosis candidate -> structured fix hypothesis -> validation experiment -> A/B evidence -> fix verdict
```

This is the academic center of the project. The tool is valuable not because it
always finds a positive tweak, but because it can refuse weak tuning stories.

## 3. Requirements and Design Goals

### 3.1 Functional Requirements

The project needs to support the following functional behavior:

- Record scheduler, frame, process-tree, GPU, and quality evidence from a target
  game or process tree.
- Correlate scheduler-visible delay with frame timing and workload identity.
- Classify relevant tasks and processes, including game, Wine, runtime,
  compositor, and helper roles.
- Generate scoped tuning hypotheses from recorded evidence.
- Represent tuning changes as explicit profiles or fix plans.
- Apply reversible low/medium-risk actions only through policy-controlled paths.
- Compare baseline and tuned runs using repeated A/B measurements.
- Report uncertainty, data quality, verdicts, and reasons instead of only a
  single performance number.
- Explain profile matching before or after an experiment so a user can audit
  which rules matched which tasks.

### 3.2 Non-Functional Requirements

The non-functional requirements are equally important:

- Safety first: observation and planning must not mutate the machine.
- Explicit targeting: the user should know which process tree or workload is
  being measured.
- Reversibility: system-changing actions should have rollback state before
  they are applied.
- No false confidence: underpowered or noisy evidence must not be presented as
  a validated fix.
- Reproducible artifacts: runs, profiles, summaries, and recommendations should
  be inspectable after the experiment.
- Human auditability: recommendations should expose the evidence, the profile,
  the verdict, and the uncertainty.

These requirements make a negative KCD1 tuning result look like success rather
than failure. A tool designed to avoid unsupported recommendations should be
judged partly by whether it can say "not validated" when a plausible tuning
idea does not survive measurement.

## 4. System Architecture

`stutter` can be described as an evidence pipeline:

```text
record -> analyze/report -> advisor -> profile/fix plan -> tune/recommend -> explain
```

The pipeline separates observation, diagnosis, planning, action execution, and
validation.

### 4.1 Observation and Recording

The recording path collects live evidence from a target workload. Scheduler
events provide runnable-latency samples. Process-tree snapshots identify which
tasks belong to the game and related runtime. Optional collectors provide frame
timing, GPU samples, CPU frequency, foreground-window context, and runtime
slice information.

Observation is deliberately non-mutating. It reads live state and emits
artifacts, but it does not decide that a tuning action is safe.

### 4.2 Analysis and Reporting

Analysis turns raw artifacts into summaries: latency thresholds, frame-pacing
tails, task attribution, quality warnings, and diagnostic scores. The report
path produces human-readable and machine-readable output so that later
recommendation steps can be audited.

The diagnostic score used in the KCD1 evaluation is an internal comparison
score, not FPS. Lower is better. It combines scheduler-latency threshold counts
for relevant task classes with frame-time tail counts. Its purpose is to compare
candidate profiles under the same workload, route, and measurement settings.

### 4.3 Advisor and Fix Plans

The advisor turns evidence into a scoped tuning hypothesis. A hypothesis is not
proof and is not permission to change the system. It records a candidate action,
why the action might help, what evidence motivated it, and what validation must
show before the action can be considered supported.

This design keeps the project from becoming an automatic tweak generator. The
advisor can propose an experiment, but the validation pipeline must still
decide whether the proposal is recommended.

### 4.4 Tuning and Recommendation

The tuning flow compares profiles across repeated runs. A typical A/B profile
set includes a `baseline-online` profile that leaves relevant tasks on the
online CPU mask and one or more tuned profiles. Repeated measurements are then
ranked by diagnostic and workload-specific metrics.

The recommendation step can return outcomes such as validated, underpowered,
inconclusive, invalid experiment, or needs retest. These outcomes matter because
they prevent the report from turning weak evidence into advice.

### 4.5 Safety and Rollback

The safety model separates observation, recommendation, and system change.
State-changing actions are represented as `TuningAction` values and checked
through action descriptors, policy, preflight checks, and rollback behavior.
The daemon architecture describes an always-on observer, planner, and guarded
action runner. Providers suggest candidates; they do not mutate the system
directly.

Important safety rules include:

- Observe-only behavior is the default.
- Apply mode must be explicitly enabled.
- Unsupported action families are denied by policy.
- Autonomous apply requires rollback.
- Data-quality failure blocks action.
- Failed verification or worse/inconclusive evidence triggers rollback unless a
  policy explicitly says otherwise.

This safety model is part of the FYP contribution because it treats tuning as a
controlled experiment rather than as a permanent machine policy.

### 4.6 Profile Explainability

Profile explainability makes a tuning hypothesis auditable. `profile-plan` and
`apply-profile --dry-run --explain` show which rules match which tasks, what
classes and `comm` values are involved, what CPU masks would be applied, and how
many pending changes would occur.

This feature matters in the KCD1 case study because the profile used a broad
`match_comm = ["Main"]` rule. The explainability artifact showed that important
KCD1 worker threads were matched through `process_comm = "Main"`, which made
the negative A/B result more meaningful: the profile did not merely fail because
it missed the relevant game threads.

## 5. Implementation

### 5.1 Rust Workspace Layout

The project is implemented as a Rust workspace. The workspace members are:

- `stutter`: the main CLI and application logic.
- `stutter-common`: shared common structures.
- `stutter-config`: configuration model and resolution logic.
- `stutter-core`: core typed primitives.
- `stutter-ebpf`: eBPF-side code and build integration.
- `stutter-report`: report model, loading, analysis, and rendering.
- `xtask`: repository maintenance and validation commands.

This split lets the project keep artifact models, configuration, eBPF code,
report rendering, and validation tooling in separate ownership areas.

### 5.2 eBPF Probe Handling

The scheduler profiler is built around eBPF tracepoints for wakeup and context
switch timing. The measured quantity is runnable latency: the time between a
task becoming runnable and the task actually being switched onto a CPU.

Because eBPF tracing usually requires privileges, live recording is separated
from offline commands. Reporting, recommendation, advisor, and audit commands
can operate on files without loading probes.

### 5.3 Artifact Format

`stutter` records structured artifacts such as JSON and NDJSON files for session
metadata, tree events, frame correlation, GPU samples, CPU frequency samples,
runtime slices, foreground/focus events, spike events, and interval summaries.
This is essential for the FYP because the evaluation can be inspected after the
live game has stopped.

The case study keeps these artifacts under `reports/kcd1-case-study/`, with
separate areas for setup notes, baseline runs, advisor output, profile plans,
tune output, result reports, drop-counter investigation, and exploratory
personal-stack measurements.

### 5.4 Scoring and Comparison

The diagnostic score used in the KCD1 case study combines scheduler and frame
signals. In the frame-aware path, scheduler thresholds are weighted by counts
over 1ms, 2ms, and 5ms, while frame thresholds are weighted by counts over
roughly 16ms, 33ms, and 50ms. The score is useful only when comparing runs
under the same workload and measurement settings.

For auto-tune controller decisions, the architecture also records normalized
score rates and objective-specific signals so that unequal windows and workload
objectives can be handled more carefully. The important design principle is
that no single global score should be treated as a universal performance unit.

### 5.5 CPU-Affinity Profile Model

The KCD1 evaluation used a TOML profile model. Profiles contain ordered rules,
and rules are first-match-wins. A rule can match by class or by `comm`. In the
current behavior, `match_comm` checks both a task's own `comm` and its process
`process_comm`. This can be powerful but also needs auditability because a broad
process-level match can capture many worker threads.

### 5.6 Safety and Rollback Implementation

System-changing actions are guarded by policy and action descriptors. A fix
plan records risk, rollback requirements, effect scope, privilege needs,
persistence flags, and whether the default policy allows the proposed
experiment. Apply paths are expected to support preflight, dry-run, apply,
verify, rollback, and audit.

The key implementation choice is that recommendations do not automatically
become permanent changes. The tool can suggest experiments, apply guarded
actions where policy allows, and restore saved state.

### 5.7 Testing Strategy

Testing is part of the project evidence. The repository includes unit tests,
integration tests, architecture tests, report golden tests, validation-corpus
fixtures, and `xtask` checks. The validation corpus includes real and synthetic
fixtures covering vendors, compositors, scenarios, known false positives, known
false negatives, quality levels, and display-path cases.

This test strategy supports the FYP argument that the project is more than a
single case study. KCD1 is the main real evaluation, but the repository also
contains broader regression and fixture coverage for the tool's artifact and
analysis behavior.

## 6. Validation Methodology

### 6.1 Repeated Measurement

The validation methodology treats tuning as an experiment. A candidate profile
must be tested against comparable baseline behavior under the same route,
duration, and workload labels. Repeated runs are needed because game workloads
vary.

In the KCD1 case study, the formal baseline set used five 180-second route
runs, and the A/B tune used five measured iterations for `baseline-online` and
five for the tuned CPU-affinity profile. The tool also reported that some
metrics may need roughly 18-30+ runs per condition to estimate smaller effects
precisely.

### 6.2 Baseline-Online

`baseline-online` is the within-tune control profile. It keeps relevant tasks
on the online CPU mask rather than applying the tuned affinity split. Comparing
the tuned profile against `baseline-online` inside the same tune run helps
separate the tuning hypothesis from unrelated changes in the earlier baseline
archive.

### 6.3 Warmup and Measurement Windows

Game runs often need stabilization time. Warmup windows let the workload settle
before scoring. Measurement windows then provide the evidence used for profile
comparison. In the KCD1 tune run, each epoch used warmup plus measurement; the
case-study report records that each epoch used 90 seconds of warmup followed by
180 seconds of measurement.

### 6.4 Diagnostic Score

`diagnostic_raw_score_total` is not FPS and not a general benchmark score. It is
an internal weighted penalty score for comparable runs under the same settings.
Lower is better. Larger values mean more or worse scheduler/frame-pacing
outliers during the measured window.

This distinction is important for academic reporting. The score supports an
within-experiment comparison, not broad claims across games or machines.

### 6.5 Uncertainty and Verdicts

The recommendation model prevents false confidence. A result can be
underpowered, inconclusive, invalid, or marked as needing retest even when one
profile ranks better in a small sample. This is not hedging; it is the mechanism
that makes the tool evidence-based.

For KCD1, the generated recommendation selected `baseline-online` as the current
best profile and reported `NeedsRetest`. The tuned-profile conclusion in this
report is therefore based on the profile candidate statistics in
`tuning_summary.json`, while the recommendation artifact is interpreted
carefully because some formal comparison fields compare the selected best
profile against itself.

### 6.6 Non-Validation Is Not Proof of Harm

"Not validated" means the evidence did not support recommending the tuned
profile. It is not the same as proving the profile is harmful in every
condition. The KCD1 result says that under the tested route, hardware, Proton
version, and profile, the tuned profile did not earn a recommendation.

This distinction is crucial to the FYP. The point is to demonstrate evidence
discipline, not to force a strong claim from a small experiment.

## 7. Main Evaluation: KCD1 Case Study

### 7.1 Setup

The main evaluation used Kingdom Come: Deliverance 1 under Steam with
GE-Proton10-34, Sway/Wayland, Gamescope, and MangoHud frame logging. The route
was a repeatable 180-second Rattay route from the same save. The system used an
Intel i5-10600K with 6 cores and 12 threads and an AMD Radeon RX 9070 XT.

The experiment intentionally excluded the author's larger personal optimized
launch configuration. The measurement setup retained Gamescope, MangoHud, and
the archived KCD1 config, but did not include RADV experimental flags, FSR/FSR4,
gamemode, mimalloc, or forced Wine CPU topology. This kept the main variable to
the CPU-affinity profile.

### 7.2 Baseline Findings

Five formal baseline runs passed the basic validity checks. Each ran for about
180 seconds, stopped because the maximum duration was reached, ingested MangoHud
frame data, used monotonic frame timestamp alignment, and reported `Medium`
data quality.

| Run | Frames | Median frametime | P99 | Max | Frame-pacing outliers |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline-01 | 8,833 | 17.725ms | 51.008ms | 562.266ms | 1,347 |
| baseline-02 | 7,744 | 21.276ms | 49.712ms | 272.166ms | 1,354 |
| baseline-03 | 9,261 | 16.309ms | 49.085ms | 563.179ms | 1,303 |
| baseline-04 | 7,421 | 22.811ms | 46.778ms | 84.384ms | 1,229 |
| baseline-05 | 7,607 | 22.884ms | 44.788ms | 87.387ms | 1,098 |

The baseline route was valid but noisy. Median frametime ranged from about
16.3ms to 22.9ms, while p99 frametime remained around the mid-40s to low-50s
milliseconds. This justified repeated A/B measurement rather than a one-run
tuning claim.

### 7.3 Advisor Hypothesis

The advisor generated a plausible CPU-placement hypothesis: reserve the core-0
SMT pair for Gamescope/runtime work and place KCD1/Wine/game threads on the
remaining CPUs. This hypothesis was plausible because the baseline evidence
showed scheduler-visible tail latency and frame-pacing outliers, but it was not
treated as validated by diagnosis alone.

### 7.4 CPU-Affinity Profile

The tested profile used two conditions:

- `baseline-online`: leave relevant classes on the online CPU set.
- `kcd1-game-on-1-5-7-11-gamescope-on-0-6`: place game/Wine classes on
  `1-5,7-11` and Gamescope/runtime classes on `0,6`.

The intended effect was to isolate presentation/runtime work from the main game
work. On this CPU, that also reduced the game side from 12 logical CPUs to 10
logical CPUs, which later became important for interpretation.

### 7.5 Profile Explainability

The profile-plan follow-up showed that the tuned profile would match 114 of 181
snapshot tasks and had 114 pending affinity changes. Rule counts were 88, 25,
and 1 for the three profile rules.

The key interpretive result is that important KCD/DXVK/Wine worker threads were
matched through the broad `process_comm = "Main"` behavior, including render,
streaming, DXVK, audio, shader, physics, and job-system threads. The tuned
profile did not fail simply because it missed the relevant tasks.

### 7.6 A/B Result

The proper A/B tune run, `kcd1-affinity-02`, tested both profiles with five
valid measured iterations each. Lower diagnostic score is better.

| Profile | Valid runs | Median diagnostic score | Mean diagnostic score | Median frame P99 | Mean over-5ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| `baseline-online` | 5 | 21,533 | 23,431.4 | 38.337ms | 0.2 |
| `kcd1-game-on-1-5-7-11-gamescope-on-0-6` | 5 | 38,806 | 49,746.0 | 37.798ms | 0.6 |

The tuned profile had a worse primary diagnostic score in every paired
iteration:

| Iteration | Baseline-online score | Tuned profile score | Delta |
| ---: | ---: | ---: | ---: |
| 1 | 19,579 | 32,052 | +63.7% |
| 2 | 21,533 | 34,566 | +60.5% |
| 3 | 16,643 | 44,845 | +169.5% |
| 4 | 26,408 | 38,806 | +46.9% |
| 5 | 32,994 | 98,461 | +198.4% |

The correct conclusion is that the tuned profile was not validated and should
not be recommended on current evidence. This does not claim the profile is
harmful in all conditions. It says that the tested route and system did not
support recommending this CPU-affinity split.

### 7.7 Why the Profile Likely Failed

The most plausible interpretation is that the profile reduced useful CPU
capacity. KCD1/DXVK/Wine had many active worker, render, streaming, physics,
audio, and helper threads. Moving the game workload from all 12 logical CPUs to
`1-5,7-11` gave it only 10 logical CPUs, while reserving `0,6` for Gamescope.
On this 6-core/12-thread CPU, that trade-off appears to have increased
scheduling pressure more than it helped presentation isolation.

This is an interpretation, not a proven causal mechanism. The evidence supports
a careful claim: the profile caught relevant tasks, but repeated A/B measurement
did not support recommending it.

### 7.8 Drop-Counter Investigation

A separate measurement-quality pilot investigated recurring non-zero wakeup
replacement counters. The important finding was that the issue was not
ring-buffer reserve failure. Baselines showed about 1436-1557 wakeup
replacements per second with zero ring-buffer reserve failures, and the
mapfactor-4 pilot still showed about 1568 wakeup replacements per second with
zero reserve failures.

The likely interpretation is wakeup timestamp churn from rapid repeated wakeups
for the same target tasks. This is a measurement-quality nuance, not evidence
that frame data was dropped.

### 7.9 Personal-Stack Add-On

An exploratory add-on compared the stripped-down measurement stack against the
author's normal gaming configuration bundle, including `scx_lavd` and many
launch flags. This changed many variables at once, so it is not a causal test of
one flag or scheduler.

In three runs per condition, the personal stack did not show a clear
frame-pacing advantage. The value of the add-on is realism: `stutter` can
capture and compare a complex player-used configuration bundle without
overclaiming causality.

### 7.10 Evaluation Summary

The KCD1 case study validates the workflow rather than the tuning tweak.
`stutter` captured a complex Proton workload, generated a plausible hypothesis,
tested it, explained why the result mattered, and declined to recommend the
hypothesis when the evidence did not support it.

## 8. Results and Discussion

The main result is methodological. `stutter` successfully completed the
evidence loop on a real Proton/Wine/Gamescope workload:

- It recorded scheduler and frame evidence from a live game route.
- It produced a scoped CPU-affinity hypothesis.
- It represented that hypothesis as a reversible profile.
- It tested the profile through repeated A/B measurement.
- It selected `baseline-online` as the current best profile.
- It did not recommend the tuned profile when the evidence was weak or negative.
- It later made the profile behavior auditable through rule-level
  explainability.

This is a useful result precisely because it is not a positive tuning story. An
evidence-based advisor should not be rewarded only when it finds improvements.
It should also be rewarded when it refuses unsupported advice.

The project therefore demonstrates a methodology:

```text
observe -> hypothesize -> validate -> recommend only if supported
```

The case study also shows why explainability matters. Before the profile-plan
artifact, a reader might suspect that the CPU-affinity profile failed because it
did not match important KCD1 worker threads. The follow-up artifact showed that
the profile did match those threads. That shifts the interpretation from "bad
matching" to a more interesting tuning result: a plausible and relevant profile
still did not perform better under measurement.

The expected result at the start of a tuning experiment is often "maybe this
will help." The actual result was "this should not be recommended on current
evidence." That is a stronger academic contribution because it shows restraint.

## 9. Limitations

The project has several important limitations:

- The main case study used one machine.
- It used one game, one route, and one Proton version.
- The formal KCD1 artifacts did not include IRQ/KMS/DRM correlation.
- The workload variance was high.
- Five runs per profile were enough for a case-study demonstration, but not
  enough for precise small-effect estimates.
- The tool estimated that some metrics may need roughly 18-30+ runs per
  condition for a 10% effect at the observed noise level.
- The personal-stack comparison changed many variables at once and is therefore
  exploratory and non-causal.
- The current tool still expects technical users who can identify process trees,
  interpret artifacts, and manage privileged tracing.

These limitations do not undermine the result. They define its scope. The
project does not claim a general KCD1 optimization. It claims that the tool can
collect evidence, form a tuning hypothesis, test it, and avoid a false-positive
recommendation under a real workload.

## 10. Future Work

Future work should extend both the tool and the evaluation:

- Add IRQ/KMS/DRM capture to future case studies so scheduler, interrupt, and
  display presentation evidence can be interpreted together.
- Evaluate more games, engines, hardware, GPUs, compositors, and Proton
  versions.
- Improve automated sample-size guidance so users know when a comparison is
  likely underpowered before spending time on more runs.
- Improve report generation so the FYP-style narrative can be produced more
  directly from artifacts.
- Expand profile explainability so every tuning hypothesis is auditable before
  A/B data collection.
- Explore more granular profile hypotheses that do not compress broad game
  worker sets onto fewer CPUs without stronger evidence.
- Improve foreground and target selection for Gamescope and nested runtime
  process trees.
- Consider a UI or dashboard after the evidence, safety, and validation model
  is stable.

## 11. Conclusion

This project demonstrates that Linux game tuning can be made more
evidence-based by collecting scheduler and frame evidence, generating scoped
hypotheses, validating them with repeated measurement, and refusing unsupported
recommendations.

The KCD1 case study is the central evidence. `stutter` captured a real
Proton/Wine/Gamescope workload, produced a plausible CPU-affinity profile,
tested it against an online baseline, and did not recommend it when the A/B data
failed to support it. The experiment validates the workflow rather than the
specific CPU-affinity tweak.

That result is valuable because reliable tuning tools must be able to say no. A
tool that turns every plausible hypothesis into advice is not evidence-based.
`stutter` shows the more useful behavior: collect evidence, test the claim, show
uncertainty, and decline unsupported tuning advice.

## Appendix A: Command Shapes

The KCD1 archive records command shapes rather than every exact shell
invocation. Live PIDs were re-detected before recording.

Baseline record shape:

```bash
stutter record \
  --tree-pid <KCD1_OR_GAMESCOPE_TREE_PID> \
  --duration 180 \
  --run-name kcd1-rattay-baseline-XX \
  --scenario kcd1-rattay-route-1 \
  --workload-label kcd1-proton-ge-10-34 \
  --route-label rattay-fixed-route-1 \
  --out-dir reports/kcd1-case-study/runs/baseline-XX \
  --mangohud-log <KingdomCome_MANGOHUD_CSV> \
  --hwmon \
  --cpu-freq \
  --runtime-slices \
  --foreground-window \
  --foreground-source sway
```

Tune shape:

```bash
stutter tune \
  --tree-pid <KCD1_OR_GAMESCOPE_TREE_PID> \
  --profiles reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --runs 5 \
  --baseline-profile baseline-online \
  --warmup-seconds 90 \
  --epoch-seconds 270 \
  --scenario kcd1-rattay-route-1 \
  --workload-label kcd1-proton-ge-10-34 \
  --route-label rattay-fixed-route-1 \
  --out-dir reports/kcd1-case-study/tune/kcd1-affinity-02
```

Profile-plan shape:

```bash
stutter profile-plan \
  --tree-pid <KCD1_OR_GAMESCOPE_TREE_PID> \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6
```

Recommend shape:

```bash
stutter recommend \
  --fix-plan reports/kcd1-case-study/fix-plan-cpu-affinity-profile.json \
  --baseline reports/kcd1-case-study/runs/baseline-01 \
  --baseline reports/kcd1-case-study/runs/baseline-02 \
  --baseline reports/kcd1-case-study/runs/baseline-03 \
  --baseline reports/kcd1-case-study/runs/baseline-04 \
  --baseline reports/kcd1-case-study/runs/baseline-05 \
  --tune reports/kcd1-case-study/tune/kcd1-affinity-02 \
  --markdown reports/kcd1-case-study/results/kcd1-fix-validation.md \
  --html reports/kcd1-case-study/results/kcd1-fix-validation.html
```

Validation flow:

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt --all -- --check
RUSTUP_TOOLCHAIN=nightly cargo test --all
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-check
```

## Appendix B: KCD1 Profile TOML

```toml
[[profile]]
name = "baseline-online"

[[profile.rules]]
affinity = "online"
match_class = ["Game", "GameHelper", "WineServer", "GameScope", "Compositor", "Launcher", "SteamRuntime", "Helper", "Unknown"]

[[profile]]
name = "kcd1-game-on-1-5-7-11-gamescope-on-0-6"

[[profile.rules]]
affinity = "1-5,7-11"
match_comm = ["Main"]

[[profile.rules]]
affinity = "1-5,7-11"
match_class = ["Game", "GameHelper", "WineServer"]

[[profile.rules]]
affinity = "0,6"
match_class = ["GameScope", "Compositor", "Launcher", "SteamRuntime"]
```

## Appendix C: Artifact Map

| Artifact | Role |
| --- | --- |
| `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md` | Detailed KCD1 case-study report |
| `reports/kcd1-case-study/CASE_STUDY_SUMMARY.md` | KCD1 archive summary |
| `reports/kcd1-case-study/ARTIFACT_INDEX.md` | Archive map |
| `reports/kcd1-case-study/setup/system-info.txt` | Machine and session context |
| `reports/kcd1-case-study/runs/baseline-*` | Formal baseline run artifacts |
| `reports/kcd1-case-study/tune/kcd1-affinity-02/tuning_summary.json` | Primary A/B profile comparison |
| `reports/kcd1-case-study/tune/kcd1-affinity-02/tuning_recommendation.json` | Recommendation and uncertainty data |
| `reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan-summary.json` | Profile explainability summary |
| `reports/kcd1-case-study/drop-counter-pilot/mapfactor-4-comparison.txt` | Measurement-quality investigation |
| `reports/kcd1-case-study/realworld-stack/realworld-stack-summary.csv` | Exploratory clean vs personal-stack comparison |
| `docs/TUNING_WORKFLOW.md` | Trusted diagnosis-to-validation loop |
| `docs/SAFETY.md` | Safety, rollback, and privilege model |
| `docs/FULL_SYSTEM_WATCHER_ARCHITECTURE.md` | Observer/planner/action-runner architecture |
| `docs/AUTOTUNE_ARCHITECTURE.md` | Controller contract and keep/revert model |

## Appendix D: Reproducibility Checklist

- Use the same Rattay route and save described in the KCD1 method notes.
- Use Steam with GE-Proton10-34.
- Use the stripped-down measurement configuration: Gamescope and MangoHud
  logging, no RADV experimental flags, no FSR/FSR4, no gamemode, no mimalloc,
  and no forced Wine CPU topology.
- Keep `+exec user.cfg` enabled because the archived config is part of the
  workload.
- Use 1920x1080 through Gamescope, 100 Hz output, and a 100 FPS MangoHud cap.
- Use 180 seconds per measured run.
- Re-detect the live Gamescope/KCD process-tree root before each recording.
- Keep background load stable.
- Treat hardware differences as a limitation.

## Appendix E: Glossary

| Term | Meaning |
| --- | --- |
| A/B test | Repeated comparison between baseline and tuned conditions |
| Baseline-online | A tune profile that keeps relevant tasks on the online CPU mask |
| Diagnostic score | Internal weighted penalty score for comparable runs; lower is better |
| eBPF | Linux in-kernel instrumentation mechanism used for live tracing |
| Frame pacing | Consistency of frame delivery over time, especially tail latency |
| Gamescope | Gaming-focused Wayland compositor often used around Proton games |
| MangoHud | Overlay and logging tool used here for frame timing |
| Profile-plan | `stutter` output explaining how profile rules match live tasks |
| Runnable latency | Delay between a task becoming runnable and receiving CPU time |
| Tuning hypothesis | A scoped, testable proposal generated from evidence |
