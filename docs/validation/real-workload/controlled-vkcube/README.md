# Controlled-real vkcube comparison (runner engineering check)

**Evidence kind: `controlled_real`. This is not real-game evidence and says nothing about any game.**

An owned `vkcube` (640x480, FIFO present, so capped at the 155 Hz display) under MangoHud with
per-frame logging, started through `stutter game-run`, 5 s of warmup and 20 s measured per
execution. Baseline is the unmodified launch; treatment is the same launch under `taskset -c 2-3`.
The comparison was frozen before any measurement in
[`../controlled-vkcube-manifest.json`](../controlled-vkcube-manifest.json) (digest
`89983e9f71bfd587ec8fecd8fdaa39a1239b0e0d829fa6c17c025d124cc71436`, commit `91adc9e8`), with the
stutter binary `b8affcf2…` replaced by the exact binary hash recorded in the manifest, six
counterbalanced ABBA pairs, a primary metric of frame p99, a 10 percent minimum effect and a
frame-mean guardrail of 5 percent.

## Result

| | value |
|---|---|
| EngineeringPass | **yes**: 12 of 12 executions produced complete logs, equal frame counts of 3101 in 11 and 2673 in one, no kernel GPU fault, kernel log observed |
| Outcome | **NoBenefit** |
| Primary effect (frame p99, positive is better) | **-6.1 percent**, 95 percent interval -12.7 to +0.4 |
| Baseline frame p99 | 6.55 ms median of runs (mean 6.55) |
| Treatment frame p99 | 6.81 ms median of runs (mean 6.96) |

Pinning a vsync-capped, light workload to two CPUs did not help and may have cost a little: the
interval stays below the 10 percent minimum effect. That is the expected, uninteresting answer for
a workload that is not CPU-bound, which is the point of an engineering check: the runner, the
supervision, the capture, the metrics, the pairing and the classification all worked end to end
on a real MangoHud log, and the verdict is a truthful negative.

The treatment is the runner's unprivileged `taskset` pin of an owned process. It is **not** the
Stutter controller's CPU-affinity operation, so this result says nothing about that operation.

## What happened on the way

* A first attempt (`run1`, discarded) produced no frames: the runner created the capture root with
  default permissions and `game-run` correctly refused a root that others can enter. The fault was
  in the runner; it was fixed (owner-only capture roots, with a test) and the comparison was run
  again from scratch. No measurement was discarded to obtain this result; the failed attempt had no
  data. The comparison itself was run exactly once.
* One treatment execution (slot 9) saw 2673 frames instead of 3101, which is background noise on a
  desktop machine; it is kept, and it is part of why the interval is wide.

## Files

`verdict.json` is the classification, `slot-NNN.json` the 12 executions (arm, exit reason, frame
count, metrics, GPU-health observation). The raw MangoHud logs stay in the local run directory.
