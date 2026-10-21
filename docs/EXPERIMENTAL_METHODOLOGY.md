# Experimental Methodology

This document defines the default methodology for using `stutter` to evaluate a
performance-tuning hypothesis. It is intended to keep experiments repeatable,
comparable, and conservative enough that noisy workload variation is not mistaken
for an improvement.

The method applies most directly to game and Proton/Wine workloads, but the same
principles are useful for other interactive Linux workloads.

## Core principles

Every controlled tuning experiment should follow these rules:

1. Keep the workload as stable as practical.
2. Change one primary variable at a time.
3. Separate warmup from the measured window.
4. Use repeated measurements rather than a single before/after run.
5. Preserve raw and derived evidence.
6. Control candidate order so time drift is not confounded with the candidate.
7. Reject or downgrade non-comparable runs.
8. Treat missing evidence and uncertainty as results.
9. Prefer reversible changes.
10. Do not make a causal or general performance claim beyond the measured setup.

## Define the experiment before measuring

Before collecting the comparison runs, write down:

- workload and version;
- route, save, scenario, benchmark, or other repeatable action;
- hardware and relevant CPU topology;
- kernel, graphics driver, compositor/session, Proton/Wine, and Gamescope state
  where relevant;
- baseline condition;
- candidate condition;
- primary metric or objective;
- guardrail metrics;
- warmup duration;
- measured duration;
- number of repetitions;
- candidate-order schedule;
- rejection conditions;
- rollback procedure;
- artifact location.

The purpose is to prevent the interpretation rules from changing after the data
is visible.

## Establish the baseline

Collect repeated baseline runs before deciding that a candidate is effective.

A valid baseline run should, where applicable, have:

- the expected stop reason;
- the intended measured duration;
- enough frame samples for frame-aware analysis;
- valid timestamp alignment;
- acceptable data-quality status;
- expected process-tree targeting;
- required artifacts present;
- no known major background workload change.

Do not discard a noisy baseline simply because it makes an expected improvement
harder to demonstrate. High variance is part of the experiment and affects the
amount of evidence required.

## Form a scoped hypothesis

A tuning hypothesis should describe a mechanism and a measurable expectation.

A useful form is:

```text
Because <observed evidence>, changing <specific reversible variable> may improve
<primary metric> without violating <guardrail metrics>.
```

The hypothesis is not yet a recommendation.

For example, a CPU-affinity proposal should identify which task classes or
processes move, which CPU set they move to, why interference may be reduced, and
what capacity or scheduling-flexibility risk the restriction introduces.

## Explain the candidate before applying it

Run the relevant dry-run or profile-plan path before a real tuning experiment.

Confirm:

- which rules match;
- match source (`comm`, process name, class, or other selector);
- first-match-wins behavior;
- affected tasks and threads;
- proposed CPU masks or other target values;
- actions that will not be changed;
- rollback state that will be retained.

If the plan affects tasks that were not intended, fix the profile before
measurement. Do not continue merely because the configuration parses.

## Warmup and measured windows

Warmup should be long enough for the workload to reach the state that the
experiment intends to compare. Shader compilation, asset streaming, cache state,
world traversal, and process startup can otherwise dominate the first part of a
run.

The warmup window must not be silently mixed with the measured window. Store the
durations in the experiment artifacts so later comparisons can confirm that the
same timing rules were used.

## Repeated A/B comparisons

For two candidates, repeated comparisons should use counterbalanced order.

Preferred order:

```text
iteration 1: A -> B
iteration 2: B -> A
iteration 3: A -> B
iteration 4: B -> A
...
```

A fixed order such as `A -> B` for every iteration is paired but not
counterbalanced. It can confound the candidate effect with time-dependent
effects such as temperature, cache state, background activity, route drift, or
progressive workload state.

If an archived run used fixed order, retain the evidence but lower confidence and
mark the result for retest rather than pretending the design was stronger than
it was.

## Comparability checks

Before ranking candidates, confirm that the runs are comparable.

Check at least:

- same workload/scenario identity;
- same intended duration and warmup policy;
- same major launch configuration;
- same target-selection strategy;
- same required probes or a documented degraded-evidence state;
- no incompatible artifact/schema assumptions;
- no obvious background-load or system-state change that invalidates the pair.

The tuning pipeline should surface comparability warnings. A warning that affects
the primary conclusion must be resolved or reflected in the verdict.

## Data-quality checks

Inspect the signals that describe whether the measurement itself is trustworthy:

- stop reason;
- duration;
- frame count;
- timestamp alignment;
- eBPF event-loss or reserve-failure counters;
- map replacement counters;
- missing optional streams;
- process-tree completeness;
- run comparability.

Do not assume that every non-zero counter means the same type of data loss. A
replacement counter, map-capacity event, and ring-buffer reserve failure can
represent different mechanisms and need separate interpretation.

## Metrics and objectives

Choose the primary objective before viewing the result.

Frame-aware experiments may use frame-time distributions and outlier metrics.
Scheduler-focused experiments may use runnable-latency or the project's
diagnostic score. Other objectives can have different primary metrics and
guardrails.

The internal diagnostic score is a weighted evidence score, not FPS. When the
score is used, lower values should be interpreted according to the report's
metric definition rather than described as a direct frame-rate change.

Do not promote a candidate because one secondary metric moved favorably while
the primary objective or an important guardrail regressed.

## Uncertainty and sample size

Repeated runs reduce the risk of treating one noisy sample as a real effect, but
a small number of repetitions can still be underpowered.

Use the available uncertainty outputs, confidence intervals, effect estimates,
noise ratios, and sample-size guidance as conservative decision aids. They do not
convert an uncontrolled experiment into causal proof.

A confidence interval crossing zero, high variance, low sample count, or a
methodological warning can justify `NeedsRetest` even when one candidate ranks
first.

## Verdict rules

Use conservative verdict language.

### Validated

Use only when the experiment is comparable, data quality is acceptable, the
candidate effect is supported across repeated measurements, and guardrails do
not invalidate the improvement.

### Rejected / not recommended

Use when the candidate does not improve the selected objective, regresses it, or
violates important guardrails.

### NeedsRetest

Use when the candidate may be promising but the available evidence is too weak,
noisy, underpowered, non-counterbalanced, or otherwise insufficient for a strong
recommendation.

### Invalid experiment

Use when comparability, targeting, data quality, configuration, or artifact
conditions make the comparison unsuitable for the intended conclusion.

`NeedsRetest` and `InvalidExperiment` are not software failures. They are valid
outcomes that prevent overclaiming.

## Negative results

A negative tuning result should be kept when the experiment itself is useful.

Preserve:

- the hypothesis;
- the candidate configuration;
- the measurements;
- the reason the candidate was rejected or left uncertain;
- methodological limitations;
- follow-up ideas.

Do not rewrite the narrative to make the candidate look successful. The project
is an evidence tool, not a tweak generator.

## Artifact retention

Keep enough information to reproduce and audit the experiment without forcing
large raw captures into normal Git history.

The source repository should contain:

- methodology and setup notes;
- candidate configuration;
- human-readable summary;
- primary machine-readable comparison artifacts;
- checksums or an artifact manifest;
- a pointer to any separately distributed raw archive.

Large raw traces, run directories, and frame logs can live in a GitHub Release or
other versioned artifact store. Their checksum and contents listing should be
published with the archive.

## Case-study interpretation boundary

The KCD1 case study is one machine, one game, one route, one Proton version, and
one hardware/software configuration. Its CPU-affinity result must not be
generalized to all KCD1 systems or Linux gaming.

The useful conclusion is methodological: a plausible profile can be tested,
found unsupported, and withheld from recommendation. The archived KCD1 A/B run
was paired but used fixed candidate order, so it remains low-confidence
non-validation evidence until repeated with counterbalanced order.

## Reproducibility checklist

Before treating an experiment as complete, verify:

```text
[ ] Workload/scenario is documented.
[ ] Software and hardware context is documented.
[ ] Baseline and candidate are explicit.
[ ] Primary metric/objective was chosen before interpretation.
[ ] Warmup and measured windows are explicit.
[ ] Candidate order is counterbalanced where applicable.
[ ] Repetition count is recorded.
[ ] Data-quality checks pass or limitations are documented.
[ ] Candidate plan/explainability output was inspected.
[ ] Rollback/restore behavior is recorded.
[ ] Raw and derived artifacts are retained.
[ ] Machine-readable results are parseable.
[ ] Human-readable conclusion matches the machine-readable result.
[ ] Generalization is limited to the measured setup.
```

## Related documentation

- `docs/TUNING_WORKFLOW.md`
- `docs/SAFETY.md`
- `docs/DEGRADED_EVIDENCE.md`
- `docs/ARTIFACT_SCHEMA.md`
- `docs/VALIDATION_CORPUS.md`
- `reports/kcd1-case-study/`
