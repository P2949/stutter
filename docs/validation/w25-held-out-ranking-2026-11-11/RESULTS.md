# W25 result: held-out ranking on measured blocks (2026-11-11)

Run as preregistered (`PREREGISTRATION.md`, committed before any measurement; rule unchanged). 40 executions, 0 failed, 0 invalid
blocks, no game running. Blocks 1-4 training, 5-8 untouched holdout, evaluated only after all eight ran. Raw data:
`results.tsv`, `markers-all.ndjson`; harness `study.sh`.

Mean improvement over the nice-0 reference of the same block (compile time, higher is better):

| candidate (hogs' nice) | training mean | holdout mean |
|---|---|---|
| 3 (conservative baseline) | 17.28 % | 17.81 % |
| 6 | 21.46 % | 25.31 % |
| 10 | 24.57 % | 24.01 % |
| 19 | 28.29 % | 27.24 % |

Held-out ranking (same quantities as `held_out_ranking`: 4 common candidates): training favourite = holdout favourite (nice 19,
`top1_agrees` true), `holdout_regret` 0.00, Spearman 0.80. The ranker's holdout mean improvement exceeds the conservative
baseline's by 9.4 points (rule needs 2), so by the frozen rule the learned ranking beat the conservative one on the holdout.

**Default selection is NOT changed.** The measured candidates (renicing foreign background tasks) are not in Stutter's legal
action descriptor space: the worker performs no such operation, and an optimizer may not create one. The result validates the
ranking method on independent blocks (including that it does not mis-rank the harmful end here, and that a sparse or
mismatched dataset yields no ranking: `held_out_ranking` returns `None` below three common candidates, covered by the existing
search tests). Applying it to default selection needs real candidates from a legal action, which is W27's scope. This is one
scenario, one workload and one host; it is not transfer evidence across contexts.

## Cross-context transfer (2026-11-11)

Update: a second context on this machine (CPU topology reduced to CPUs 0-5, 6 hogs) was then measured and the ranking transferred under the frozen rule: see `../w25-cross-context-2026-11-11/RESULTS.md`. What follows is the earlier statement, which still holds for other hosts, schedulers, kernels and GPUs.

Transfer across contexts (other CPU topologies, schedulers, kernels, GPUs, hosts) needs those contexts. Development targets this
machine only (the user's standing scope for this phase), so transfer to another host is `NotPlannedForVerification`. What is
covered here: within this one context a learned ranking is evaluated on an untouched holdout (above), and a changed context
signature resets learning, a wrong objective unit, a sparse dataset (fewer than three common candidates returns no ranking) and
a stale owner fall back to the default ranking, as the search fixtures test. The default ranking is unchanged.
