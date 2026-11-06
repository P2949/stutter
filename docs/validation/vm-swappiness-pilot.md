# vm.swappiness feasibility pilot

Question: does `vm.swappiness` change the completion time of the fixed cold build
(`cargo build -p xtask`, 4 CPUs, `-j4`) by more than run-to-run noise when the build
runs under real memory pressure?

Method: an unconstrained probe build measured the peak footprint (4.67 GB). Each timed build then ran in a
transient user scope with `MemoryHigh` at 55 percent of that peak (a throttle, never a kill),
at swappiness 10, 60 and 100, four interleaved blocks with alternating order, original value restored
on exit. Raw rows: [vm-swappiness-pilot-results.txt](vm-swappiness-pilot-results.txt).

| swappiness | mean elapsed (s) | vs 60 | swap-out pages (mean) |
|---|---|---|---|
| 10  | 85.27 | +0.45 % | ~84,700 |
| 60  | 84.89 | baseline | ~86,800 |
| 100 | 86.74 | +2.2 % | ~194,000 |

Result: the mechanism is observable (swap-out roughly doubles at 100), but no setting beats the
default; the best case is within noise and 100 is slower. Against the 7.5 percent objective threshold
there is no benefit to graduate. Decision: `VmSwappiness` stays `DeferredEvidence`; no typed `Sysctl`
contract is built from this pilot. One workload, one host, 32 GB RAM with fast NVMe swap; it does not
rule out a benefit on a machine with slow swap or heavier thrashing.

## Second pilot: heavier pressure (MemoryHigh at 35 percent of peak, about 1.63 GB)

Same method, four interleaved blocks, settings 10, 60, 100 and 180. Raw rows:
[vm-swappiness-pilot-35-results.txt](vm-swappiness-pilot-35-results.txt). (The script's "55%" echo line in
that run is a stale label; the value used was 35 percent, 1,634,797,158 bytes.)

| swappiness | mean elapsed (s) | vs 60 | swap-out pages (mean) |
|---|---|---|---|
| 10  | 99.78  | -1.96 % | ~565,000 |
| 60  | 101.78 | baseline | ~655,000 |
| 100 | 101.96 | +0.18 % | ~672,000 |
| 180 | 104.02 | +2.2 %  | ~711,000 |

Swappiness 10 was the fastest setting in every block, so the direction is real and the mechanism is
observable (less swap traffic, fewer anonymous refaults). The size is about 2 percent, below the
frozen 7.5 percent keep threshold of the compile objective. A keep at this effect size would require a
lower threshold, which the plan forbids ("never relax thresholds"), and a contract that could never
keep a change. Decision stays: `VmSwappiness` is `DeferredEvidence`; no `Sysctl` contract is built.
A workload with a larger swap-sensitive working set, or a slower swap device, is the route to
reopen it; that needs a new preregistered hypothesis and fresh calibration and holdout data.

## Third pilot: heaviest pressure (MemoryHigh at 20 percent of peak, about 0.9 GB)

Swappiness 10 against 60 only, four interleaved blocks. Raw rows:
[vm-swappiness-pilot-20-results.txt](vm-swappiness-pilot-20-results.txt). (The "55%" echo line is the same stale label; 20 percent was used.)

| swappiness | mean elapsed (s) | vs 60 |
|---|---|---|
| 10 | 173.28 | +1.2 % |
| 60 | 171.15 | baseline |

Swap traffic fell at 10 (swap-in about 1.47M against 1.68M pages) but completion time did not improve, and
the direction differed between blocks. Across 55, 35 and 20 percent pressure the effect never
approaches the frozen 7.5 percent threshold and does not grow with pressure (+0.45, -1.96, +1.2 percent).
Conclusion: on this host and workload class swappiness is not a lever for completion time. The candidate is
exhausted; `VmSwappiness` stays `DeferredEvidence`.
