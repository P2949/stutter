# StutterScore evidence across a workload matrix (post-v0.1)

The v0.1 score studies (a controlled runnable series and a frame series) were both
non-monotonic. This study widens the evidence and **changes nothing**: the weights stay exactly as
in v0.1 (100 x runs over 5 ms, 20 x over 2 ms, 1 x over 1 ms; frame terms unchanged), and
`tune_evaluation` stays Experimental. Retained reports: [docs/evidence/score-matrix/](evidence/score-matrix/);
definition: [`docs/validation/scenarios/score-matrix.toml`](validation/scenarios/score-matrix.toml).

## Design

Four workload profiles x six controlled scheduler-degradation levels (0, 1, 2, 4, 8, 16 ms bursts
of a SCHED_FIFO contender on the target's pinned CPU, 32 ms period) x five repetitions = 120
recordings of 30 s. The level order is a Williams balanced Latin square rotated per repetition
(never always ascending), profiles are interleaved, and every recording waits for storage to settle.

| Profile | Workload | Regime |
| --- | --- | --- |
| vkmark-light | `vkmark` cube 640x480, vsync | light, paced |
| vkmark-heavy | `vkmark` texture 4K, immediate | GPU-bound |
| vkcube-light | `vkcube` 640x480, vsync | light, paced |
| vkcube-heavy | `vkcube` 4K, immediate | CPU-bound submit loop |

A GL benchmark (`glmark2`) was tried first for the second workload and rejected: MangoHud wrote
no frame log for it here. `vkcube` is Vulkan, CLI-started, has a known pid, a deterministic scene,
and a MangoHud-compatible frame stream. Retained per recording: the score, runnable p99, share of
runnable samples above 2 ms and 5 ms, frame p99 and p99.9, share of frames above 16.6 ms and 33.3 ms.

## Finding 1: as shipped, the score is blind to a non-game target

| Median StutterScore | 0 ms | 1 ms | 2 ms | 4 ms | 8 ms | 16 ms |
| --- | --- | --- | --- | --- | --- | --- |
| vkmark-light | 0 | 0 | 0 | 0 | 0 | 482 |
| vkmark-heavy | 0 | 0 | 0 | 0 | 0 | 37 |
| vkcube-light | 0 | 0 | 0 | 0 | 0 | 574 |
| vkcube-heavy | 0 | 0 | 0 | 0 | 0 | 39 |

The runnable terms only count tasks of game classes, and none of these targets is classified as a
game, so the score is exactly zero until a frame exceeds 16.6 ms (only at 16 ms). Runnable p99 of
the same recordings rises steadily (light profiles: 0.2-0.5 -> 1 -> 2 -> 5 -> 9 -> 18 ms). The
v0.1 non-monotonicity is therefore at least partly this classification gate, not the weights.

## Finding 2: with the named target counted, the ordering mostly holds

Counting the generic-class tasks of the explicitly named process as the target (the rule the
diagnosis uses since the post-v0.1 correction), identical weights:

| Median StutterScore | 0 ms | 1 ms | 2 ms | 4 ms | 8 ms | 16 ms | rho vs level | adjacent inversions |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| vkmark-light | 197 | 224 | 2296 | 16683 | 109245 | 138291 | 1.00 | 0 |
| vkcube-light | 145 | 370 | 4547 | 26376 | 172762 | 219645 | 1.00 | 0 |
| vkmark-heavy | 1464 | 1132 | 3706 | 15022 | 83407 | 82408 | 0.89 | 2 |
| vkcube-heavy | 2715 | 2519 | 4002 | 22635 | 111737 | 111061 | 0.89 | 2 |

The light profiles are strictly monotone and agree with runnable p99 and frame p99 (rho 1.0). The
heavy profiles have two small inversions each: at the bottom (0 vs 1 ms, within the baseline noise
of an unpaced or GPU-bound target) and at the top (8 vs 16 ms, a ceiling: the counts saturate).
In the GPU-bound vkmark-heavy profile the target's own runnable p99 does not order the levels at
all (about 0.1 ms throughout) while frame p99 does (0.20 -> 1.58 ms): the two direct metrics
disagree, so no single scalar can match both. Pooled over all 120 recordings the score's rank
correlation with the level is 0.93 (explicit target) against 0.68 (as shipped).

## Answer and consequences

*Does one scalar reliably preserve the ordering of the direct evidence across distinct
workloads?* Not strictly: it does for the light workloads and nearly for the heavy ones once the
target is counted, and not at all for a non-game target as shipped. Therefore:

* weights are retained; `tune_evaluation` stays Experimental; nothing is promoted;
* the classification gate and the choice between a score redesign and objective-specific scores
  are tracked as a future design issue, not changed here.

Limitations: one host with the desktop in use; the degradation is one kind (scheduler contention);
`vkcube` is a trivial scene, so its "heavy" profile is CPU-bound rather than GPU-heavy; two cells
needed a top-up run after a driver defect left them one repetition short (the accepted
recordings of the first run are taken over unchanged, see `study.json`).
