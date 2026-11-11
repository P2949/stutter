# W25 preregistration (written before any measurement)

Question: does a ranking learned on independent training blocks pick a better candidate than the conservative baseline on an
untouched holdout, so that default selection could change? If not, the default ranking stays and the negative result is reported.

Scenario and KPI: the W24 contended cold compile (`stress-ng --cpu 12` hogs, KPI compile `elapsed_ns`, lower is better).
Legal candidate set (the action descriptor): the hogs' nice value in {3, 6, 10, 19}. Reference arm in every block: nice 0
(no action). Per block, `improvement(c) = (t_ref - t_c) / t_ref`.

Blocks: 8, each running the reference and all four candidates once, arm order rotated by block index. Blocks 1-4 are TRAINING,
blocks 5-8 are the untouched HOLDOUT and are not looked at until all 8 ran. No early stopping.

Ranker under test: candidates ordered by mean training improvement (this is `held_out_ranking`'s predicted order).
Conservative baseline: the default ranking, which proposes no action change (nice 3, the smallest step, is its only action).
Reported unchanged from `stutter::api` semantics: `top1_agrees`, `holdout_regret` (never above 0), Spearman of the two orderings.

Decision rule, frozen: the ranker changes default selection only if its training favourite is the holdout's favourite or its
holdout regret is within 1 percentage point of the best AND its holdout mean improvement exceeds the conservative baseline's by
at least 2 percentage points. Otherwise default selection is kept. A failed execution or a game running makes the block Invalid.
