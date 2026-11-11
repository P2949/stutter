# E3 preregistration: owned CPU-affinity candidates on the fixed cold compile (written before any measurement)

Manifest: `manifest.json` (frozen digest `956f128af15a1fa83607273b9ed7176b269b1d497758dafcf30c3bdfddf05440`, produced by `xtask owned-study validate`). Harness: `xtask owned-study`
(source commit recorded in RESULTS.md); its unit tests passed and a 4-slot A/A ran first (`../e2-owned-study-aa-2026-11-11/`).

Question: among legal, owned, unprivileged CPU-affinity placements of the compile and of the owned pressure generator, which
change the elapsed time of the identical cold `stutter-config` build (W23 identity), and does a ranking learned on training blocks
hold on untouched holdout blocks?

Scenario: 12 `stress-ng --cpu` hogs (owned, started by the harness under the existing supervisor) run for the whole compile.
Candidates (all subsets of the actual effective CPU set 0-11; SMT siblings are N and N+6 on this i5-10600K):
- `compile-primary-threads`: compile on CPUs 0-5 (one thread per core), hogs unrestricted.
- `hogs-sibling-threads`: hogs confined to CPUs 6-11, compile unrestricted.
- `partition-primary-vs-sibling`: compile on 0-5, hogs on 6-11.
- `compile-two-cpus-control`: compile on CPUs 0-1 (a deliberately valid bad control; expected harmful).
Reference: `baseline`, nothing restricted. Nothing touches the daemon, the worker, the compositor or any process not started by the
harness; kernel readback of every role's CPU mask is checked inside the target before it execs, and a mismatch makes the slot Invalid.

Design: 6 blocks, each runs the baseline and all four candidates once, arm order rotated by block and reversed on even blocks.
Blocks 1-3 are TRAINING, blocks 4-6 the untouched HOLDOUT; no early stopping, no extra blocks. A game or launcher running, a failed
build, wrong markers or a readback mismatch makes a slot Invalid and it is reported, not repeated. Cold build every slot.
Primary KPI: `WORKLOAD_COMPLETE` `elapsed_ns`, lower is better; per block, improvement = (t_baseline - t_candidate) / t_baseline.

Decision rule, frozen: per candidate, the seeded 95% percentile bootstrap interval of the mean block improvement over all valid
paired blocks (10,000 resamples, seed from the manifest digest). Practical threshold 2 percentage points: Beneficial if the lower
bound is above +2, Harmful if the upper bound is below -2, Equivalent if the interval lies inside (-2, +2), otherwise Inconclusive.
Ranking: `held_out_ranking` over training vs holdout means of the legal candidates (top-1 agreement, regret, Spearman).
Every outcome, including Inconclusive or Harmful, is reported. The effect is offline, owned-scope evidence only: it grants no
Supported tuple, changes no default selection and is not worker authority.
