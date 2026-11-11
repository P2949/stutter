# W24 preregistration (written before any measurement)

Workload and KPI: the W23 cold fixed-work compile (`docs/validation/w23-compile-cohort-2026-11-11/identity.json`), KPI = completion
`elapsed_ns`, lower is better. Contention: `stress-ng --cpu 12` (one hog per logical CPU) started 2 s before each execution and
stopped after it. No game, no other load.

Arms. Study 1 (protect the foreground from background contention): baseline = hogs at nice 0; candidate = the same hogs at
nice 19 (an unprivileged, reversible action on the hogs only; the compile is untouched). Study 2 (harmful control, no
contention): baseline = compile as is; candidate = compile restricted to one CPU (`taskset -c 0`).

Design: 8 blocks per study, each block one execution of each arm, order counterbalanced (blocks 1,4,5,8 baseline first; 2,3,6,7
candidate first). All 8 blocks are run; no early stopping and no peeking.

Statistic: per block, r = candidate/baseline - 1. Mean r and its two-sided 95 % interval with t(7) = 2.365.
Classification, frozen: **Beneficial** if the upper bound < -7.5 %; **Harmful** if the lower bound > +7.5 %; **Equivalent** if
the whole interval lies within +-7.5 %; otherwise **Uncertain**. A block with a failed execution, a source change or a
contaminating game process makes the study **Invalid**. The 7.5 % is the threshold frozen by the objective campaign and is not
changed. Protected guardrail: the compile must complete successfully with unchanged source in every execution.

Expected before running (stated to prevent post-hoc fitting): Study 2 Harmful; Study 1 either Beneficial or Equivalent.
