# Roadmap

`stutter` is moving from profiler toward trustworthy recommender in small, auditable steps.

## Stages

| Stage | Status | Notes |
| --- | --- | --- |
| Profiler | Completed | Records runnable latency, frame correlation, optional IRQ/GPU/block-I/O/fault context, and data-quality notes. |
| Manual diagnosis | Completed | Reports cautious diagnosis candidates and supporting evidence. |
| Profile benchmarker | Current | `tune` benchmarks explicit CPU-affinity profile sets and records per-candidate artifacts. |
| Profile recommender | Current | `tune` writes recommendation artifacts, and `recommend` compares baseline runs with tune output. |
| Advisor daemon | Current | `advisor --watch-runs` watches completed runs and emits conservative offline recommendations. |
| Limited auto-tuner | Future | Any future automatic action should go through safety classes, preflight, audit logging, verification, and rollback. |
| Broader optimizer | Future | IRQ affinity, uclamp, nice, GPU, SCX, and other tunables require stronger evidence and explicit safety design before implementation. |

## Direction

Near-term work should improve trust, repeatability, and rollback before adding broader system changes:

- Better comparability checks for tune runs.
- More example profile sets and benchmark guidance.
- More offline smoke fixtures for recommendation/advisor behavior.
- Stronger audit views for actions that change system state.
- Clearer separation between observation, recommendation, and application.

## Non-goals For Now

- No broad privileged auto-optimizer.
- No silent system-wide tuning.
- No new low-level probes unless they answer a specific evidence gap.
- No claim that a recommendation proves root cause.
