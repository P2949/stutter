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

## nice - DeferredEvidence (M6)

- Campaign `mut-nice-a60588a-1792988880352`, source `a60588a9`. Scope tested: a stable game under
  scheduler pressure; the provider demotes (nice +10) the sustained same-user CPU hogs measured
  competing with it (`autotune/offenders`), judged by `GameRunnableLatency`.
- Passed: preflight, mechanical apply/verify/rollback, all 11 fault-matrix checkpoint classes, real
  suspend/resume. Restoring a demotion needs privilege (`RLIMIT_NICE` is 0 for an ordinary user), so
  startup recovery of nice tokens goes through the privileged worker and the validation controllers
  raise their own limit with `CAP_SYS_RESOURCE`.
- Failed (ValidationFailure): benefit calibration, 30 pairs per scenario, no threshold set kept the
  beneficial pairs without a false keep; in the beneficial pairs the demotion made the game's p99
  runnable wait no better and usually worse (a woken task waits for the running hog's slice
  regardless of the hog's weight under EEVDF).
- The other nice scopes (compile/background demotion judged by `DesktopInteractivity`, browser
  negative nice) stay Experimental: their protected-workload objectives are not Supported.

## ioprio - DeferredEvidence (M6, evaluated without a campaign)

- The current provider applies `idle` I/O priority to the selected important tree, which is not a
  defensible optimisation. The plan's redesign needs the offending I/O actor, a protected-target
  outcome and a mechanism check.
- The existing block-I/O telemetry (`block_io_overlap_count`, `block_io_worst_latency_ns`) is an
  overlap *correlation* and carries no per-process offender identity, and the only Supported
  objective, `GameRunnableLatency`, measures CPU wake latency, which an I/O priority cannot change;
  `IoLatency` is Experimental and describes the subsystem, not the protected outcome. No new probe
  was added because the missing signal is an objective, not an actor.
- Consequence: `IoPrio` stays Experimental; no campaign was run because its preflight (a Supported
  or Validating objective the operation can influence) cannot pass.

## cpu_power_epp - not Supported (M7, evaluated)

- Prepared transaction, cpufreq resource identity (policy, related CPUs, driver, attribute, legal
  values, boot), verified startup recovery, the EPP-only provider semantics and the per-operation
  system-wide policy are implemented and pass the 11-checkpoint fault matrix against a faithful
  per-task cpufreq fixture.
- Benefit study (this host, Intel i5-10600K, intel_pstate active, desktop with no power-supply
  limits): a fixed 12-thread and 3-thread SHA-256 workload, counterbalanced `balance_performance` and
  `performance`, 5 repetitions each. 12 threads: 4205/4228/4205/4213/4212 ms vs
  4183/4198/4203/4195/4200 ms (about 0.3 percent); 3 threads: no difference beyond noise. The Supported
  objectives are a 12.5 percent runnable-latency criterion and a 10 percent compile-throughput
  criterion; neither can be met by a 0.3 percent effect, and a frequency preference cannot shorten a
  CPU wake queue.
- Consequence: per the roadmap, `cpu_power_epp` remains Experimental and M7 is not complete on this
  hardware; governor mutation is not used as a fallback.

## M8 dispositions (evaluated against the roadmap's prerequisites)

None of the three operations can pass its own prerequisite on this host; each stays Experimental and
none is promoted to make the matrix look complete.

- **gpu_power_profile - DeferredEvidence.** Only `pp_power_profile_mode` could ever be considered, and
  its primary objective is `GameFramePacing`, which is itself Experimental (no Validating objective,
  no frame-pacing graduation evidence). The host GPU (AMD, `pp_power_profile_mode` present) is
  addressable only as `cardN`; a stable PCI/render-node bound recovery token would have to be built
  and qualified before any campaign, and without a Validating objective there is nothing for the
  campaign to decide with. Forced DPM stays HighRisk/manual.
- **vm_swappiness - DeferredEvidence.** The campaign needs real memory/swap pressure and a Supported
  direct workload objective. The only Supported objective measures CPU wake latency, which
  `vm.swappiness` does not influence, and `CompileThroughputWithForegroundProtection` is only
  Validating. The runtime-sysctl token also needs a boot-aware retirement rule that no consumer yet
  needs. Dirty-ratio knobs stay HighRisk/manual.
- **irq_known_device - DeferredEvidence.** The post-v0.1 IRQ study stays invalid (`INVALID`: no
  frame effect could be produced) and is not used. A new target-impact experiment needs controllable
  interrupt traffic on a known non-storage device class; this host has no peer to generate network
  or USB traffic against, so the effect on the target cannot be demonstrated. Storage and unknown
  IRQs stay HighRisk/manual.
