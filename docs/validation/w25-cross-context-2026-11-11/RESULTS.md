# W25 cross-context transfer on this machine (2026-11-11)

Preregistration: `PREREGISTRATION.md` (sha256 `4eadeafd…`, written before any context-B measurement). Context A = the eight W25
blocks (12 CPUs, 12 hogs). Context B = the same machine, workload and KPI with CPU topology reduced to CPUs 0-5 (`taskset`) and
6 hogs, 4 counterbalanced blocks, 20 valid runs (`results.tsv`, `markers-all.ndjson`). No game ran. The ranker is trained on
context A only; nothing from B trained or tuned anything. Means below are per-candidate improvement over the block's nice-0
reference (`improvements.txt`).

| Candidate | A mean (8 blocks) | B mean (4 blocks) | B min..max |
|---|---|---|---|
| nice 19 | 27.77% | 23.66% | 19.22..28.02 |
| nice 10 | 24.29% | 20.79% | 11.57..27.65 |
| nice 6 | 23.39% | 15.88% | 7.85..22.76 |
| nice 3 (conservative baseline) | 17.55% | 8.63% | 2.08..17.07 |

Ranking A: 19 > 10 > 6 > 3. Ranking B: 19 > 10 > 6 > 3. Spearman 1.0, `top1_agrees` true, B regret 0. The A-favourite's B mean
exceeds the conservative baseline's by 15.0 percentage points (rule: at least 2). Under the frozen rule transfer is SUPPORTED
for this context change. Default selection is unchanged (these candidates are outside the legal action descriptor space, see the
W25 results); the context signature still resets learning on a change, which stays the safe behaviour.

Limits, stated plainly: only one changed context feature (CPU topology/affinity), same kernel, scheduler, workload and host; four
blocks in B with visibly wider spread than A; so this is evidence that transfer held for this one change, not a general claim.
Other contexts (other schedulers, kernels, GPUs, hosts) remain `NotPlannedForVerification` for this machine-only phase.

Disclosure: the first context-B attempt produced no data (all 20 runs `rc=1`, `cargo` not on PATH outside the dev shell; the
failed `results.tsv` was overwritten by the rerun under `nix develop`). The reported data is the complete rerun, with no
runs dropped or repeated.
