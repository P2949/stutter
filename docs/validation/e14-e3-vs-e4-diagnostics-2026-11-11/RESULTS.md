# E14: where did the CPU time go in the E3 (affinity) and E4 (scope-weight) contexts? (2026-11-11)

Preregistration `PREREGISTRATION.md` (sha256 prefix `896406ab`, committed in `a79ea992` before any slot). Two short paired studies on the E3 workload
(cold debug `stutter-config`, 2 jobs, 12 owned hogs), `diagnostics: true`, 3 counterbalanced blocks each, 6 slots each, all 12 valid, no game, no leftovers.
`affinity/`: `manifest-affinity.json`, `analysis-affinity.json`; `scope/`: `manifest-scope.json`, `analysis-scope.json`. Raw `*.perf.csv`, `*.diag.json`, `slots/`, `run.log`.

## Signals and their scopes
perf stat on the compile's own process tree, user-space only (`perf_event_paranoid=2`), all counted at 100% (no multiplexing); derived CPU seconds per wall second
(task-clock / elapsed); SYSTEM-wide CPU "some" stall time from `/proc/pressure/cpu` (kernel microseconds). **Invalid signal:** `context-switches` and
`cpu-migrations` read exactly 0 in every slot, which is impossible for a build; under user-space-only perf the kernel-side software counters are excluded, so
they are reported as `InvalidSignal` and not used (the first summary of this data averaged them as 0; that was corrected before any conclusion, from the
unchanged raw files). No runnable-delay tracepoint was read (it needs tracefs rights the harness lacks). The perf wrapper is identical in every arm.

| Study / arm | Mean elapsed | Elapsed effect | Compile CPU s per wall s | User cycles | User instructions | User IPC | System PSI cpu-some delta |
|---|---|---|---|---|---|---|---|
| affinity: baseline | 10.07 s | n/a | 1.050 | 41.81 G | 33.537 G | 0.802 | 3,802 ms |
| affinity: `hogs-sibling-threads` | 5.90 s | +41.35% (36.0..44.6) Beneficial | 1.689 | 40.83 G | 33.540 G | 0.822 | 3,890 ms |
| scope: baseline (100/100) | 7.00 s | n/a | 1.469 | 40.72 G | 33.537 G | 0.824 | 2,138 ms |
| scope: `hogs-scope-weight-10` | 6.39 s | +8.77% (5.5..12.5) Beneficial | 1.589 | 40.17 G | 33.538 G | 0.835 | 1,670 ms |

## Limited conclusion (descriptive; correlation, across two separate studies)
- The effects replicate E3 (+41.4% vs +42.9%) and E4 (+8.8% vs +5.9%, same direction and class).
- In both studies the elapsed-time gain moves together with the compile's CPU time per wall second (affinity 1.05 to 1.69; scope 1.47 to 1.59), while the compile's total
  user-space work is the same in every arm (instructions 33.54 G; cycles within 3%) and IPC is unchanged. So the arms did the same work and the speedup tracks how much CPU
  the compile got, not how efficiently it ran.
- E4's baseline already gave the compile 40% more CPU per wall second than E3's unscoped baseline (1.47 vs 1.05) and finished 30% sooner (7.00 s vs 10.07 s). That
  explains most of the E3 >> E4 gap in percentages: the two studies compare against different baselines. In absolute terms the best arms end close together (5.90 s affinity,
  6.39 s scope).
- System-wide CPU stall did not shrink in the affinity arm (3.80 s to 3.89 s), as expected while the hogs still compete for six CPUs; the scope arm's lower stall (2.14 s to 1.67 s) is a
  system-level number and is not attributed to the compile.
- Not shown: WHY separating the compile and hogs into equal-weight scopes shifts the CPU share; a per-cgroup versus per-thread fairness explanation fits but was not tested.
  No production authority, tuple or default follows; one workload, one host, 3 blocks per study.
