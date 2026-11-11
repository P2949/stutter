# E3: owned CPU-affinity candidates on the fixed cold compile (2026-11-11)

Preregistration `PREREGISTRATION.md` (sha256 `da565dfc…`, committed in `4548f6c1` before the run), frozen manifest `manifest.json`
(digest `956f128af15a1fa83607273b9ed7176b269b1d497758dafcf30c3bdfddf05440`). Harness: `xtask owned-study` (source `4548f6c1`, 14 unit tests). 6 counterbalanced blocks, 30 slots, all
30 valid, 0 invalid; every role's CPU mask was read back from inside the target and matched; no game ran; no stress-ng or
fixed-work process remained afterwards. Raw: `results.jsonl`, `*.markers.ndjson`, `run.log`, `analysis.json`.

| Candidate (owned placement) | Mean improvement | Seeded 95% bootstrap interval | Verdict |
|---|---|---|---|
| compile-primary-threads | 8.99% | [3.94, 15.69] | Beneficial |
| compile-two-cpus-control | -2.17% | [-9.24, 3.44] | Inconclusive |
| hogs-sibling-threads | 42.87% | [40.08, 45.92] | Beneficial |
| partition-primary-vs-sibling | 43.46% | [40.8, 46.38] | Beneficial |

Held-out ranking of the four legal candidates (training blocks 1-3, holdout 4-6): Spearman 1.0, top-1 agrees, regret 0. The training
favourite `partition-primary-vs-sibling` scored 44.7% in training and 42.2% on the untouched holdout (the no-change reference is 0).

What this does and does not show:
- Confining the owned pressure generator to the SMT-sibling CPUs 6-11 shortens the identical cold compile by about 43% under 12-hog
  contention; adding the matching compile placement on 0-5 adds nothing measurable (the two intervals overlap). This is the
  legal-placement analogue of the W24 nice-19 effect, obtained with an unprivileged child-affinity mechanism on processes the study owns.
- Pinning only the compile to the primary threads helps modestly (+9.0%, interval above +2).
- The `compile-two-cpus-control` candidate was intended as a harmful control and was NOT shown harmful: Inconclusive (-2.2%, interval
  -9.2..+3.4 spans zero). A plausible reason is that the build runs with `-j2`, so two CPUs are nearly sufficient, but that was not tested.
  The preregistered expectation that it would be harmful is not supported by this data and is recorded as such.
- Scope: one workload, one host, one pressure shape, 6 blocks. It is offline evidence from an owned harness. It does not move
  any operation into `AUTONOMOUS_CONTRACTS`, grant a Supported tuple, alter default selection, or show a game benefit: the production
  worker's `CpuAffinityProfile` contract applies to a game under GameRunnableLatency, a different workload and objective.
