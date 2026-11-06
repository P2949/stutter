# cgroup `cpu.weight` feasibility pilot: preregistration

Written and committed before any measurement exists.

## Question

When a latency-sensitive foreground workload and a CPU-saturating background compile compete for the same
CPUs, does giving the foreground cgroup a higher `cpu.weight` measurably and repeatably lower its frame
times, at an acceptable cost to the compile? `cpu.weight` is a cgroup-scoped resource control, not a
machine-wide mutation (`ActionEffectScope` is not `SystemWide`); it is a candidate for a new typed,
administrator-subtree-scoped operation, and this pilot decides whether building it is worthwhile.

## Setup

- A delegated user scope (`systemd-run --user --scope -p Delegate=yes`) with two sibling cgroups,
  `fg` and `bg`, created by the harness. Only `cpu.weight` of those two cgroups is ever written; no
  system cgroup, no systemd-managed unit and no other process is touched.
- Both workloads are pinned to CPUs 0-3 with `taskset` so they genuinely contend.
- Foreground: a synthetic frame-paced loop (`fgload`): every 16.667 ms it does a fixed amount of CPU
  work (calibrated to about 6 ms alone) and records the wall time from the start of the work to its
  completion (the frame time) and whether the frame missed its 16.667 ms slot. It runs 40 s.
- Background: the fixed cold build used by the compile objective (`cargo build --offline -q -p xtask -j4`
  into a fresh target directory), started 8 s before the foreground so contention is established.
- Conditions: `fg:bg` weights `100:100` (the default, and the control), `400:100` and `1000:100`; the
  control is run twice per block so control noise is measured. Each block runs the four runs in an order
  that alternates between `100, 400, 1000, 100` and `100, 1000, 400, 100`. Four blocks.

## Metrics

Foreground: frame-time mean, p95, p99, p99.9, and the fraction of frames longer than 16.667 ms.
Background: compile elapsed time. Also the foreground and background `cpu.stat` throttling and the
foreground `cpu.pressure`, read from the cgroup files.

## Decision rule (fixed now)

Compare each non-control weight against the same block's control runs. The pilot finds a **useful
repeatable effect** for a weight only if:
1. foreground frame p99 improves by at least 20 percent in at least 3 of the 4 blocks,
2. the median p99 improvement is at least twice the largest control-to-control p99 change, and
3. the median compile slowdown at that weight is at most 15 percent.

Otherwise the result is `DeferredEvidence` and the operation is not implemented.

## Outcomes

- Useful repeatable effect: design a typed cgroup-resource identity and a dedicated reversible operation
  (administrator-configured subtree only, prepared-before-change root-worker transaction, independent
  calibration and holdout, exact Supported tuple), then calibrate. This pilot alone graduates nothing.
- No useful effect: record the raw data and stop this candidate.
