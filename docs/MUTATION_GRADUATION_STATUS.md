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

## cgroup_placement - Supported (M6)

- Campaign `mut-cgroup-placement-a509104-1793001876163`, source `a5091048`, every stage passed.
- Scope: a stable, explicitly identified game under scheduler pressure, moved into the
  administrator-configured game cgroup (a cpuset on a CPU free of the contention), judged by
  `GameRunnableLatency`; cgroup scope. It does not inherit the CPU-affinity grant: it has its own
  registry contract and its own capability.
- Stages: preflight, mechanical apply/verify/rollback, fault matrix (all 11 checkpoint classes),
  real suspend/resume with an RTC wake, 15 matched pairs per scenario (beneficial, neutral,
  regression), calibration, one holdout evaluation, evidence integration. The kernel matrix is
  skipped by an explicit, reasoned manifest entry (cgroup v2 migration and cpuset are core kernel
  interfaces).
- Thresholds: product defaults (12.5 % / 7.5 % / 1000 samples) kept; no threshold changed.
- Calibration part: 9 beneficial pairs all kept, 11 neutral and 7 regression pairs with 0 false
  keeps, 7 of 7 regressions reverted.
- Holdout (evaluated once, after freezing): 6 of 6 beneficial pairs kept (rate 1.0), 4 neutral and 8
  regression pairs with 0 false keeps, 8 of 8 regressions reverted.
- Bundles: `live_affinity-a5091048-1793001985702`, `live_recovery-a5091048-1793002022484`,
  `live_suspend_resume-a5091048-1793002047770`, `live_affinity_benefit-a5091048-1793004791026`
  (suite names are shared with the affinity harness; the index entries cover provider
  `cgroup_placement`).
- Findings that shaped the harness: `cgroup.procs` is process-granular; delegation needs a
  runner-created subtree the suite joins before exec; interactive suspend is denied outside the login
  session, so the suite falls back to non-interactive `sudo`.
- Limitations: the contention is a controlled `SCHED_FIFO` burst on a disposable game-like process,
  not a real game; the cgroup target is administrator-configured, never inferred; focus inference is
  not exercised. Promotion widens nothing beyond this tuple.

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

None of the three operations can be graduated on this host (a missing prerequisite, a measured absence of benefit, or conflicted ownership); each stays Experimental and
none is promoted to make the matrix look complete.

- **gpu_power_profile - DeferredEvidence (conflicted ownership).** `GameFramePacing` is now Supported
  (`live_frame_objective`), so the objective prerequisite is met. The blocker is ownership: on this host
  `lactd` is active and re-applies its profile every 5 seconds (`apply_settings_timer: 5`); its current
  profile `Daily` sets `performance_level: manual` and `power_profile_mode_index: 2`, and
  `pp_power_profile_mode` shows `POWER_SAVING*` (index 2). That is the exact state a Stutter GPU
  contract would write, so an autonomous write would be overwritten or would fight the owner. The plan
  forbids fighting an owner; a cooperative LACT backend would be a separate, explicitly designed
  operation, and a stable PCI-bound recovery token would still be needed (the card is addressable only as
  `cardN`). Even with a cooperative backend, graduation would need the GPU profile to move frame p99 by
  at least the frozen 50 percent threshold, which the plan forbids lowering; a power-profile change is
  not expected to do that (reasoning only: it was not measured, because measuring it would mean writing
  state that LACT owns). LACT's GPU ownership is not touched. Forced DPM stays HighRisk/manual.
- **vm_swappiness - DeferredEvidence.** `CompileThroughputWithForegroundProtection` is now Supported
  (frozen 7.5 percent threshold, `live_compile_objective`), so the objective prerequisite is met. Three
  feasibility pilots of the fixed cold build under cgroup memory pressure (55, 35 and 20 percent of
  peak; [record](validation/vm-swappiness-pilot.md)) found the mechanism observable but no
  completion-time benefit near the threshold (+0.45, -1.96, +1.2 percent for 10 against 60), so no
  `Sysctl` contract was built. Reopening needs a new preregistered hypothesis (a more swap-bound
  workload or a slower swap device). A foreground-memory-latency objective for swappiness was also
  considered and not built: on this host memory pressure never occurs naturally (PSI memory
  `some`/`full` avg10, avg60 and avg300 are all 0.00; no OOM kills; 27.3 of 31.2 GB available; 2 GB of
  the 32 GB swap in use), and 21.3 of the 26.3 million pages swapped out since boot (36 pilot builds, summed from the result files) came from the
  throttled pilot builds themselves, so there is no workload on which such an objective could fire or a
  benefit be demonstrated. Dirty-ratio knobs stay HighRisk/manual.
- **irq_known_device - DeferredEvidence.** The post-v0.1 IRQ study stays invalid (`INVALID`: no
  frame effect could be produced) and is not used. A feasibility pilot on the host's `igc` NIC
  ([record](validation/irq-affinity-pilot.md)) physically moved the queue interrupts off the probe CPU
  (76k to 157k per trial down to 0) under real download traffic and found no wake-latency benefit
  (p99 21 us default against 22 us steered). Storage and unknown IRQs stay HighRisk/manual.

## M9 bounded autonomous controller - Supported

- Capability `bounded_autotune_controller`, graduated by the `live_autotune_soak` bundle
  `live_autotune_soak-93c9bc59-1793329436496` (indexed in [`evidence/index.json`](evidence/index.json),
  manifest digest `184dae38...698e`, source commit `93c9bc59`, evidence kind `controlled_real`).
- Scope: it authorises exactly `AUTONOMOUS_CONTRACTS` (CPU-affinity profile and cgroup placement,
  each for a stable, explicitly identified game under scheduler pressure, judged by
  `GameRunnableLatency`) and nothing else: no third operation, no system-wide operation, not the
  daemon/service integration beyond the packaged bounded service, which graduated separately as
  `always_on_autotune` on targeted acceptance (see [SERVICE_GRADUATION.md](SERVICE_GRADUATION.md)).
- Qualification (the coverage-driven contract, see [AUTOTUNE_SOAK.md](AUTOTUNE_SOAK.md)): 8.03 h of
  wall-clock time, 113 of 113 rounds clean, 7 complete clean 16-round cycles (6 required), 92 resolved
  experiments (50 kept, 42 reverted), 21 controller restarts, 14 privileged-worker restarts, 14
  suspend/resume cycles, 240 workload/focus transitions. Every dangerous round kind ran clean at least
  6 times (controller crash 14, controller stop 7, worker kill 14, suspend 14, workload kill 7, focus
  swap 7, long 7); `cpu_affinity_profile` resolved 50 (29 kept, 21 reverted) and `cgroup_placement`
  42 (21 kept, 21 reverted). All five zero-violation checks passed: `rounds_clean`, `final_state`,
  `support_matrix`, `conflicts_and_cooldowns`, `faults_audited`.
- Companion gates at the same source: the full local `ci` and `ci-live` (including the real-controller
  SIGINT stress, 12 of 12 iterations), a 10,000-seed deterministic fault sweep (about 700,000 seeded
  state-machine events, passed in 3596 s), and a clean 60 minute formal smoke (17 of 17 rounds).
- Earlier runs found two real defects, kept as supporting evidence only and never combined with the
  qualification: a prepared-apply/workload-exit race that left the journal at `applying` (fixed in
  #177) and a lost SIGINT in the monitor select loop (fixed in #178; 199 of 200 rounds clean over
  14.1 h before the failing `controller_stop` round).
- Limitations: disposable game-like workloads, not real games; focus inference is not exercised
  (targets are explicit); the soak drives `stutter autotune --mode apply-medium-risk`, not the packaged
  service, which runs `apply-low-risk`. Promotion widens nothing beyond the two contracts.
