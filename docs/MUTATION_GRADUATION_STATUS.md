# Mutation graduation status

One section per operation that went through `xtask mutation-campaign` (see
[MUTATION_CAMPAIGN.md](MUTATION_CAMPAIGN.md)). A disposition is `Supported`,
`DeferredEvidence` or `ExcludedFromAutonomy`, derived from the recorded stage outcomes and never
forced. Bundles stay with the operator; the entries in [`evidence/index.json`](evidence/index.json)
pin them by manifest digest.

## cpu_affinity_profile - Supported (M4)

- Campaign `mut-cpu-affinity-profile-a1592e7-1792949588311`, source `a1592e7c`, strict verify PASS.
- Scope: a stable, explicitly identified game under scheduler pressure, judged by
  `GameRunnableLatency`; local process tree. Nothing else is covered.
- Stages: preflight, mechanical apply/verify/rollback, fault matrix (all 11 checkpoint classes, via
  the final control path), real suspend/resume with an RTC wake, 15 matched pairs per scenario
  (beneficial, neutral, regression; counterbalanced by which physical CPU carries the contention),
  calibration, one holdout evaluation, evidence integration. The kernel matrix is skipped by an
  explicit, reasoned manifest entry.
- Thresholds: the grid (improvement 10-40 %, regression 5-15 %, minimum samples 1000) was swept on the
  calibration part; every point separated the scenarios, so the product defaults (12.5 % / 7.5 % /
  1000 samples) were kept as the smallest change. No threshold was changed.
- Calibration part: 9 beneficial pairs all kept, 11 neutral and 7 regression pairs with 0 false
  keeps, 7 of 7 regressions reverted.
- Holdout (evaluated once, after freezing): 6 of 6 beneficial pairs kept (rate 1.0), 4 neutral and 8
  regression pairs with 0 false keeps, 8 of 8 regressions reverted.
- Bundles: `live_affinity-a1592e7c-1792949696173`, `live_recovery-a1592e7c-1792949732247`,
  `live_suspend_resume-a1592e7c-1792949757429`,
  `live_affinity_benefit-a1592e7c-1792965995429`.
- Limitations: the runnable p99 is a histogram bucket bound (a step is about 25 %), so the neutral
  pairs sit away from the thresholds rather than on them; the contention is a controlled
  `SCHED_FIFO` burst on a disposable game-like process, not a real game; focus inference is not
  exercised (the target is explicit). Promotion widens nothing beyond this tuple.

## uclamp - DeferredEvidence (M6)

- Campaign `mut-uclamp-afdf763-1792973347207`, source `afdf7637`. Scope tested: a stable game under
  scheduler pressure, `sched_util_min` 128 on the game's own tasks, judged by `GameRunnableLatency`.
- Passed: preflight, mechanical apply/verify/rollback, all 11 fault-matrix checkpoint classes (the
  generic harness, medium-risk policy unlock, in-process privileged service), real suspend/resume
  with an RTC wake.
- Failed (ValidationFailure): benefit calibration. After 30 pairs per scenario no threshold in the
  grid kept the beneficial pairs without a false keep. A raised utilisation floor changes the CPU
  frequency a task asks for, not how long a woken task waits for a CPU, so the objective the
  campaign is allowed to use cannot show a benefit; the holdout was never spent.
- Consequence: `Uclamp` stays Experimental, has no registry contract, and its prepared transaction
  and verified recovery remain in place for a future objective that measures what it changes.
