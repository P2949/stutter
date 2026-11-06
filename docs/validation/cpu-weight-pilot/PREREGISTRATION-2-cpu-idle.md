# cgroup `cpu.idle` feasibility pilot: preregistration (second pilot)

Written and committed before any `cpu.idle` measurement exists. The first pilot
([RESULTS.md](RESULTS.md)) showed `cpu.weight` is capped near 10 percent. This pilot tests a different
mechanism: setting `cpu.idle` to 1 on the background cgroup, which gives its tasks the idle scheduling
policy so the foreground cgroup preempts them at wakeup. It is the same harness and the same workloads.
No threshold is changed.

## Setup

Identical to the first pilot: a delegated user scope with sibling `fg` and `bg` cgroups; only `cpu.weight`
and `cpu.idle` of those two cgroups are written; `fgload` (6.1 ms per 16.667 ms frame, 40 s) against a cold
`cargo build -p xtask -j4`, both pinned to CPUs 0-3, the compile started 8 s earlier.

## Conditions

`control` (weights 100:100, `cpu.idle` 0 for both) and `idle` (weights 100:100, `cpu.idle` 1 on `bg`). Four
blocks, each four runs in the order `control, idle, idle, control` or `idle, control, control, idle`, so each block
has two controls to measure noise and two treated runs.

## Decision rule (fixed now, unchanged from the first pilot)

The pilot finds a **useful repeatable effect** only if foreground p99 improves by at least 20 percent in at
least 3 of 4 blocks, the median improvement is at least twice the largest control-to-control change, and the
median compile slowdown is at most 15 percent. Otherwise `DeferredEvidence`, and no operation is built.
