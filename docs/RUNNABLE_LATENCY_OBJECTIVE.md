# Game runnable latency contract

`GameRunnableLatency` compares runnable-to-running scheduler delay for
latency-sensitive tasks in the selected game process tree. Its primary signal is
the p99 of the **combined scheduler-event population** in each measurement
window. The generic stutter score never replaces that signal.
The core scheduler probes supply wakeup-to-`sched_switch` delays. Optional
`sched_stat_wait` totals remain separate diagnostics; preemption-only waits are
outside this histogram population.

## Population and window

Primary task classes are `Game`, `GameRenderThread`, `GameWorkerThread`,
`GameHelper`, and `WineServer`. Gamescope, compositors, Steam runtime, generic
helpers, and unrelated workloads do not contribute to the primary population.
Frame pacing and foreground latency remain guardrails.

The runtime observation builder scopes records by process membership. An explicit
root takes precedence and includes descendants resolved through the active task
snapshot's process-parent relationships. Without an explicit root, the selected
focus group's member PIDs and their tracked descendants define membership.
Records lacking a process PID cannot establish membership. Without an identified
target, there is no direct runnable signal. The root must still exist in the
active task snapshot; retained samples from an exited root cannot establish a
current population. Baseline and candidate must identify the same nonzero root
and process start-time ticks. Missing identity stamps or PID reuse make the
comparison inconclusive. Existing workload-identity and target-loss gates still
apply; an integer PID alone is not permission to mutate a process.

The rolling window retains interval histograms, not interval p99 values. All
contributing nonempty histograms must have the canonical bucket layout, counts
matching scheduler samples, consistent strict tail counters and observed maxima.
Missing, mixed-layout, corrupt, or overflowing evidence invalidates the entire
population, rather than silently dropping the inconvenient interval. Window
expiry removes expired populations before aggregation.

Percentiles use nearest rank over summed bucket counts. Values are conservative
bucket upper bounds capped by the observed window maximum. For the unbounded
overflow bucket, the observed maximum supplies that bound. This is a direct
scheduler-event histogram measurement with finite resolution, not an exact raw
sample percentile. Retained-sample truncation does not invalidate a complete
histogram. p50, p95, p99, maximum, sample count, and strict >1/>2/>5 ms counts are
preserved; p99.9 is exposed only at 10,000 or more events. Tail comparisons divide
by the runnable population, using exact integer cross multiplication.

## Comparison and safety

Both populations require direct histogram evidence from
`target-game-scheduler-histogram-v1` and at least 1,000 scheduler events. Missing
evidence, incompatible targets, or insufficient samples produce `Inconclusive`.
Low-quality/invalid windows and target disappearance cannot authorize an
improvement.

Provisional conservative thresholds are a p99 reduction of at least 12.5% for
`Improved` and a p99 increase of at least 7.5% for `Regressed`. Smaller changes
are inconclusive. These are engineering defaults pending empirical calibration,
not claims of statistical significance. Any increase in the strict >5 ms event
rate rejects a candidate. Frame p99 and foreground-tail regressions, new thermal
degradation/throttling, and new CPU/GPU power limiting also reject improvement.
Existing guardrail availability semantics are retained; this does not certify
that unavailable hardware telemetry was checked successfully.

Decision metadata identifies `runnable_p99_ns`, both values and sample counts,
tail counts, selected roots, quality, no fallback, and any triggered guardrail or
unavailable reason. All added artifact fields are optional/defaulted. Old
artifacts retain missing runnable evidence as unavailable, never as zero.
Insufficient or unavailable evidence lowers decision confidence even when the
underlying histogram source itself is direct.

The runtime controller retains baseline objective signals and uses each
non-score objective's comparator at the keep/revert transition. A generic-score
regression cannot preempt a runnable-primary improvement, and a generic-score
improvement cannot authorize a runnable-primary regression. A completed
non-score experiment without demonstrated improvement restores its candidate;
inconclusive evidence does not leave the mutation active indefinitely. The final
keep operation independently rechecks the same objective contract. Decision
stream and history reasons describe the objective used for that transition.

## Validation boundary

Before switching the comparator, both semantic counterexamples failed:

- 2 ms → 4 ms runnable p99 with a 20% generic-score improvement incorrectly
  returned `Improved`.
- 4 ms → 2 ms runnable p99 with a flat generic score incorrectly returned
  `Inconclusive`.

The objective tests now cover these conflicts, absent/approximate/insufficient
data, target mismatch/loss, small changes, population-normalized severe tails,
safety guardrails, explanations, and old artifacts. Window tests cover unequal
interval populations, truncation, class selection, expiration, malformed
histograms and overflow. Observation-builder tests exercise explicit-root,
descendant, focus, unrelated-process and no-target membership.

This contract still needs source-bound live acceptance, profiler-overhead
characterization and empirical threshold calibration before maturity can
graduate. Offline tests do not close those gates.
