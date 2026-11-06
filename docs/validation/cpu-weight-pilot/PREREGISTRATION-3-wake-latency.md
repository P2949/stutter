# cgroup CPU controls, wake-latency pilot: preregistration (third pilot)

Written and committed before any wake-latency measurement exists. The first two pilots measured frame time
and fell short of a 20 percent floor I chose for them. That floor was not tied to the controller: the foreground
objective that would gate a cgroup operation, `GameRunnableLatency`, keeps a change at a 12.5 percent
improvement of the target's wake-to-run latency (`DEFAULT_MIN_IMPROVEMENT_PERCENT`), and frame time includes
the loop's own CPU work. This pilot measures the wake latency directly and uses the controller's own bar.

## Setup

Same harness as the first two pilots. `fgload` additionally records, for every frame, the wake latency: the time
between the instant it asked to be woken (the frame slot boundary) and the instant it actually ran again. It
prints the wake-latency p50, p99 and p99.9 in microseconds. Foreground, background, pinning and timing are unchanged.

## Conditions

`control` (weights 100:100, `cpu.idle` 0), `w1000` (foreground weight 1000, background 100) and `idle`
(background `cpu.idle` 1). Four blocks of four runs each, in the orders `control, w1000, idle, control` and
`control, idle, w1000, control` alternating, so each block has two controls to measure noise. The harness sets
`cpu.idle` to 0 before it writes any weight.

## Decision rule (fixed now; the threshold is the controller's own)

For a condition to count as a **useful repeatable effect**: its wake-latency p99 improves by at least 12.5 percent
over the same block's control in at least 3 of 4 blocks, the median improvement is at least twice the largest
control-to-control change, and its median compile slowdown is at most 15 percent. Otherwise `DeferredEvidence`.
A useful effect for either condition only opens the design of a typed, reversible, administrator-subtree
operation; it graduates nothing.

## Status: withdrawn, not executed (2026-11-06)

The project owner's updated direction closed the cgroup-CPU candidates and asked for no new pilots before the
production service's own behaviour is verified. This pilot was interrupted after its first control run and is
withdrawn; it is not evidence for or against any condition. The one captured row
(`b1-p0-ctl`, control: frame p99 11.185 ms, wake-latency p50 14 us, p99 1834 us, p99.9 5929 us under contention,
against 46 us p99 for the loop alone) is kept in `results-3-interrupted.txt` as a record only. The rule above was
never applied, and nothing is concluded from it. A future cgroup-resource study would need its own new
preregistration on a real workload (see the production-behaviour work).
