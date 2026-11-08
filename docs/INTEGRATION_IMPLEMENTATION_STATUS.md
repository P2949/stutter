# Integration implementation status

This is the human-readable companion of [`integration-coverage.json`](integration-coverage.json), the
machine-readable ledger of every tool and mechanism in the current plan. It records what exists at
source `e37984c1`, what is *not yet wired*, and which evidence grade each entry has. The grades are
independent and never inherited from another tool or service backend:

* **G0** code and fixtures, **G1** real observation, **G2** safe manual experiment, **G3** exact
  autonomous tuple. Runtime availability, mechanism proof, workload benefit and Supported autonomous
  authority are four separate statuses.
* A status of `first_slice` means real code and tests exist for part of the entry. `not_started` means
  none. `code_complete` is reserved for entries whose full contract is implemented and fixture-tested;
  no entry has reached it.

## Summary at this source

| | count |
|---|---|
| ledger entries | 34 (29 dossiers and 5 extras) |
| `first_slice` | 26 |
| `not_started` | 8 |
| `code_complete` | 0 |
| entries with a G1 real observation | a handful (GameMode, LACT, systemd-oomd, sched_ext state, placement) |
| entries with G2 or G3 evidence for something new | 0 |

The Supported autonomous boundary is unchanged: CPU-affinity profile and cgroup placement for a stable
game under scheduler pressure judged by `GameRunnableLatency`. Nothing in this ledger widens it.

## What is wired and what is not

| Area | Wired into the live runtime | Present but not wired |
|---|---|---|
| Process safety | `xtask` stages run under a supervised process group with deadlines | detached-child scopes, parent-crash orphan reconciliation, the validation runner's own executor |
| Frame source | with `autotune.frame_capture_root` set the runtime discovers a verified `game-run` launch for the focused target, tails its log and evaluates trust every tick | off by default; no live game has been observed through the service; phase windows and the run engine are missing |
| Owner probes | the planner denies an operation whose domain a detected owner holds, with a 30 s cached probe | the probe is process-name based, so it is a hint; a typed receipt model (exact resource, owner class, expiry, drift) exists and fails closed, but nothing yet issues receipts from an exact adapter or checks them in the worker before commit; D-Bus adapters are missing |
| Sensors | system PSI and CPU power (driver, governor, EPP, requested and actual frequency, boost, AC, throttle events) , the user-slice OOM guard counters, NUMA allocation locality, DAMON counts and the resctrl inventory sampled into every observation | the sensor registry and PSI sensor exist and are tested, but the runtime does not use them |
| Generic KPIs | none | typed contract and bounded ingestion exist with the known defects listed in the plan (weighted aggregates, peer start time, future timestamps) |
| Experiment statistics | none in the live manager | immutable `ExperimentDesign` and `ExperimentEvidence`, a Student-t interval over independent blocks, a fixed-N rule that refuses early looks, a Bonferroni-adjusted sequential rule, and a tested demonstration that unadjusted peeking inflates false positives while the adjusted design holds its budget; the live experiment manager does not use them yet and candidate memory is unchanged |
| Real workloads | `xtask real-workload validate`, `evaluate`, `inventory`, `run` (supervised, resumable); one controlled vkcube comparison run, EngineeringPass and NoBenefit | a real-game cohort (BlockedResource), fault-injection runs, a service-attached run |

## Real-host observations

Observed on the development host at this source (G1 only, no experiments):

* `gamemoded` and `systemd-oomd` are running; LACT runs as the Nix-wrapped `.lact-wrapped`, which the
  probe now recognises (earlier it reported LACT as not running).
* sched_ext is present and disabled; DAMON's sysfs interface is present; resctrl is not mounted;
  `/dev/cpu_dma_latency` exists.
* MangoHud, Gamescope and vkcube are installed. The only Steam title has no repeatable unattended
  benchmark, so the real-game cohort is `BlockedResource`
  ([inventory](validation/real-workload/inventory-2026-11-07.json)).
* Most diagnostic and benchmark tools (perf, bpftrace, trace-cmd, fio, wrk, hwloc, tuned, bpftune and
  others) are not on this host's `PATH`; their entries remain code deliverables with a typed
  unavailable disposition.

## Rules this ledger keeps

* An entry is never marked complete because a process-name probe exists.
* Absent hardware or tooling is a runtime-availability fact, not a PASS.
* GPU electrical and profile mutation stays quarantined on this host; nothing here reruns the LACT or
  `vkmark` pilot.
