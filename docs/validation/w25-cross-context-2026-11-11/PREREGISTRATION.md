# W25 cross-context transfer preregistration (written before any context-B measurement)

Question (PLAN.md W25 "context transfer"): does a ranking learned in context A transfer to a different context B on this
machine, or must learning reset when the context signature changes?

Context A (already measured, `../w25-held-out-ranking-2026-11-11/results.tsv`, 8 blocks): 12 CPUs, 12 `stress-ng` hogs, default
CFS-class scheduling, compile KPI `elapsed_ns` of the cold `stutter-config` build (-j2).
Context B (new): the same machine, same workload and KPI, but CPU topology reduced to CPUs 0-5 (`taskset -c 0-5` on the hogs
and the compile) with 6 hogs. This changes the CPU-topology and cgroup/affinity context features and the contention ratio's
absolute capacity; scheduler, kernel, workload and KPI are unchanged. (A scheduler change via a loaded sched_ext scheduler is
not used: it would change the desktop's scheduler for minutes while the session is occupied.)

Candidates: hogs' nice in {3, 6, 10, 19}; reference arm nice 0, every block. `improvement(c) = (t_ref - t_c) / t_ref`.
Context B blocks: 4, each runs reference and all four candidates once, arm order rotated by block index. No early stopping.
The ranker is trained on ALL eight context-A blocks only; no context-B data is used to train or tune anything.

Reported: Spearman(A order, B order), `top1_agrees`, B regret (never above 0), B mean improvement of the ranker's favourite
versus the conservative baseline (nice 3, the smallest step).

Decision rule, frozen: transfer is SUPPORTED only if the A-favourite is B's favourite or its B regret is within 1 percentage
point of the best, AND its B mean improvement exceeds the conservative baseline's by at least 2 percentage points. Otherwise
transfer is NOT supported and resetting learning on a context change stays the correct behaviour. Either outcome is reported;
neither changes default selection (the candidates are not in the legal action descriptor space, see the W25 results). A failed
execution or a game running makes the block Invalid.
