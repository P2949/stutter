# E4: owned transient-scope CPU weight on the fixed cold compile (2026-11-11)

Preregistration `PREREGISTRATION.md` (sha256 prefix `c071ff76…`, committed before the run), manifest digest `192c1d731c2c9ee324c8b14744c7b598c08b066eb5996e9f7eb7927eb392912a`. Harness `xtask owned-study`
(source `4548f6c1`). 6 counterbalanced blocks, 30 slots, all 30 valid; every role's cgroup `cpu.weight` was read back from inside its
transient user scope and matched; no game ran; no stress-ng, fixed-work or `stutter-study` scope remained afterwards. Raw: `results.jsonl`,
`*.markers.ndjson`, `run.log`, `analysis.json`.

| Candidate (owned scope weights, compile / hogs) | Mean improvement | Seeded 95% bootstrap interval | Verdict |
|---|---|---|---|
| compile-scope-weight-1000 | 7.13% | [5.25, 8.93] | Beneficial |
| hogs-scope-weight-10 | 5.86% | [2.27, 10.36] | Beneficial |
| hogs-scope-weight-50 | 0.62% | [-2.81, 4.46] | Inconclusive |
| compile-scope-weight-1-control | -323.03% | [-394.91, -259] | Harmful |

Held-out ranking of the four legal candidates: Spearman 1.0, top-1 agrees, regret 0; the training favourite `compile-scope-weight-1000`
scored 6.6% in training and 7.6% on the untouched holdout.

Reading it honestly:
- The harmful control behaved as a control should: starving the compile scope (weight 1 against 100) made it about 4.2x slower.
- Prioritising the compile scope (1000 vs 100) gave +7.1%; deprioritising the pressure scope to 10 gave +5.9% (its interval's lower
  bound, +2.27, is only just above the 2-point threshold); a mild 50 was Inconclusive. The dose-response is monotone as expected.
- These cgroup effects are far smaller than the E3 affinity effect (~43%) on the same workload: relative CPU weight between two
  busy scopes still shares the CPU, while confining the generator to other CPUs removes most of the contention.
- This is a mechanism-plus-KPI result for owned transient scopes under the user manager's delegated `cpu` controller, with a real
  readback and automatic end-of-scope restore. It is NOT an autonomous cgroup-weight qualification: no Supported tuple, no default
  change, no worker policy change, and the earlier negative cpu.weight/cpu.idle pilots are untouched. One workload, one host, 6 blocks.
