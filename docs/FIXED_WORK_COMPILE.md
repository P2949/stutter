# Fixed-work compile completion contract

`CompileThroughputWithForegroundProtection` compares elapsed nanoseconds for
successful, identical completed builds. Lower is better. Activity intervals and
normalized stutter score cannot supply the primary signal.

## Producer and identity

`cargo run -p xtask -- fixed-work --identity manifest.json --markers markers.ndjson`
executes the manifest's argv directly in its working directory. The manifest is a
`FixedWorkIdentity`: `kind` (`compile` or `custom`), `source_tree`, `target`,
`build_state`, `cache_policy`, `job_count`, `command` (argv array),
`working_directory`, `toolchain`, and `environment`. All descriptions must be
nonempty; job count must be positive. Use an immutable source identity.

The command runner requires `source_tree = git:<HEAD tree hash>` and a clean Git
worktree before and after execution. It checks the source and process identity;
it does not establish that declared cache, toolchain, environment, or job policy
was actually enforced. The harness must enforce and accurately record those
conditions. Ignored build outputs are outside the source-tree check. External
producers can use SHA-256 identities but must verify their source themselves.
Do not relabel custom work as compilation.

The runner emits schema-version-1 `WORKLOAD_START`, followed by
`WORKLOAD_COMPLETE` only after successful exit and unchanged source/target.
Otherwise it emits `WORKLOAD_FAILED` and returns an error. Elapsed time uses
CLOCK_MONOTONIC and includes marker emission and command launch/wait overhead;
post-build source verification is outside the measured interval. This is not
compiler-internal CPU time. Bound commands externally when a timeout is needed.

## Recording and evaluation

Create the regular marker file before starting the monitor. Pass
`--workload-progress-log markers.ndjson` to record/monitor. The reader tails only
new appends, polls every 100 ms, buffers partial lines, and refuses replacement,
truncation, future timestamps, malformed JSON, and lines over 64 KiB. Allow the
completion marker to be polled before stopping recording. Input errors terminate
the monitor through its normal shutdown path.

Use a stable target ancestor containing the runner and command, with monitor
`--pid` and runner `--target-root-pid` selecting that same root. The default
runner target is its own PID; separate invocations therefore need an explicit
persistent ancestor for a comparable live experiment.

Exactly one start/completion pair must fit wholly inside each measurement
window. Both markers must match execution, workload, source, target PID and
process-start ticks, unit counts, and exact positive monotonic elapsed time.
Failed, duplicate, missing, incomplete, custom-kind or mismatched work remains
unavailable. Baseline and candidate must have identical workload identities,
units and sources, distinct execution IDs, and ordered non-overlapping times.
No target identity means no direct compile evidence.

The comparator requires foreground protection evidence, rejects foreground,
frame, thermal and CPU/GPU power regressions, and rejects an increase in observed
memory PSI some avg10. Invalid memory evidence is inconclusive; absent optional
memory evidence is not a measured zero. Foreground protection currently uses the
existing derived foreground signal; it is not a new direct foreground histogram.
A symmetric 10% elapsed-time threshold is provisional, not statistical confidence
or empirically calibrated policy. Smaller differences are inconclusive.

## Artifacts and validation limits

`workload_progress.json` is an optional reliable NDJSON recording stream.
Historical runs without it remain readable and lack completion evidence.
Optional objective fields preserve missing values rather than inventing zeros.
The cooperative source appears as `workload_progress` in the probe registry.

Protocol, command failure/source mutation, reader, aggregation, artifact and
objective tests provide offline correctness evidence. Live compile/foreground
acceptance, repeated controlled performance measurements and calibration remain
separate gates. A successful command timing smoke alone proves none of those.
