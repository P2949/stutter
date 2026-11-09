# Integration implementation status

This is the human-readable companion of [`integration-coverage.json`](integration-coverage.json), the
machine-readable ledger of every tool and mechanism in the current plan. It records what exists at
the verification baseline named in the ledger's `source_sha` (currently `6638a5b3`; [receipt](validation/source-ci-2026-11-09.json)). The wiring table also describes newer development changes and links their evidence below; it does not silently promote that verification pin. Live observations keep their own source pins. The production-deployed build is a separate, older pin (`e37984c1`) and is not described by this file. The grades are
independent and never inherited from another tool or service backend:

* **G0** code and fixtures, **G1** real observation, **G2** safe manual experiment, **G3** exact
  autonomous tuple. Runtime availability, mechanism proof, workload benefit and Supported autonomous
  authority are four separate statuses.
* A status of `first_slice` means real code and tests exist for part of the entry. `not_started` means
  none. `code_complete` is reserved for entries whose full contract is implemented and fixture-tested;
  entries in that state are listed in the summary below.

## Summary at this source

| | count |
|---|---|
| ledger entries | 34 (29 dossiers and 5 extras) |
| `first_slice` | 8 |
| `not_started` | 0 |
| `code_complete` | 26 (5.8 DAMOS scheme, 5.24 resctrl allocation group, 5.23 PM QoS lease and manual action, extra.workloads KPI templates, 5.9 hwloc cross-check, 5.3 and 5.5 typed capability probes, 5.4 TuneD profile and artifact, extra.boot and extra.kernels cross-boot campaign, 5.12 frame-source path, 5.6 sched_ext sensor, comparison and semantics, 5.16 power profile hold, 5.25 offline optimization workflow, 5.18 IRQ attribution, 5.13 GPU identity observer, 5.2 role hints and conflict audit, 5.15 measurement import and Bayesian policy, 5.14 OpenTuner-style ensemble, extra.mpi search method, 5.1 GameMode, 5.20 OOM guards, 5.22 PSI, 5.28 benchmark importers, extra.diag diagnostic parsers, 5.29 native controls and user intent) |
| entries with a G1 real observation | a handful (GameMode, LACT, systemd-oomd, sched_ext state, placement) |
| entries with G2 or G3 evidence for something new | 0 |

The Supported autonomous boundary is unchanged: CPU-affinity profile and cgroup placement for a stable
game under scheduler pressure judged by `GameRunnableLatency`. Nothing in this ledger widens it.

## What is wired and what is not

| Area | Wired into the live runtime | Present but not wired |
|---|---|---|
| Process safety | xtask stages now require an owned kernel cgroup, with a private birth journal, pidfd TERM/grace/KILL, independent owner-crash reaper and inode-bound cleanup; marker-clearing and setsid children are covered; output capture and cumulative CI deadlines remain bounded | privileged-runner adoption/delegation, bounded runner logs and whole-runner deadlines remain incomplete; the new primitive's validation is separate from installed-service qualification |
| OpenRC | installed per-user identities, observation-only controller and independently restricted worker policy; bounded supervisor templates and rendered hook fixtures; real unprivileged dummy supervisor lifecycle passed | [development slice](validation/openrc-2026-11-08.md); full product/recovery/delegation fixtures and independent installed-backend smoke remain open; no inherited systemd qualification |
| Frame source | opt-in verified `game-run` discovery, current-target reconciliation, clock mapping and per-window loss; producer binds to the sole renderer in an exhaustive scan; competition is checked per bounded batch and ambiguity taints the window until a clean epoch or proven boundary | live transient-controller observations exist, but clock estimates around 482–507 ms failed the unchanged 2 ms gate; no qualified direct frame-primary service experiment or real-game benefit |
| Owner probes | the planner denies an operation whose domain a detected owner holds, with a 30 s cached probe | the probe is process-name based, so it is a hint; a typed receipt model (exact resource, owner class, expiry, drift) exists and fails closed, but nothing yet issues receipts from an exact adapter or checks them in the worker before commit; the read-only D-Bus client now reads GameMode, scx_loader and power-profiles-daemon state directly, but no D-Bus write path exists |
| Offline delegation safety | SCX/profile adapter authorization has a fixture-only constructor; a caller-selected campaign string cannot authorize production execution; SCX readback requires exact kernel ops and mode | trusted campaign authorization, journal/worker integration and exact resource ownership are unfinished; the planning/fake-bus mechanisms do not grant a usable production mutation path |
| Sensors | runtime SensorHub uses the budgeted registry for cheap system/cgroup PSI, CPU power/idle, NUMA, DAMON/resctrl, network, storage and OOM diagnostics, with gated PMU escalation | availability and overhead remain sensor-specific; missing hardware never becomes a successful reading |
| PMU intervals | cumulative counter/time snapshots now yield matching interval deltas; missing reads, resets, overflow, malformed timing and partial thread sets cannot masquerade as direct complete evidence | [readback investigation](validation/pmu-2026-11-08.md): existing four-event set counted; optional branch group failed to yield usable counts and remains disabled in runtime; full 5.10 contract is still partial |
| Generic KPIs | optional `kpi_contract`/`kpi_socket_dir` config attaches a nonblocking exact-process/executable/boot collector to runtime diagnostics; explicit batch semantics, source/context retirement and loss handling; no action objective consumes it | a separate live-BPF transient-service observer passed at `1d5a3ef0`; final installed-production KPI acceptance remains open; standalone `kpi-serve` and experimental templates remain observation-only |
| Context key | the candidate-memory key now uses a context signature joining the system inventory hash, the topology-v2 fingerprint and the boot environment context hash (kernel, preemption, isolation, microcode, scheduler; not the generation), recomputed at most once per minute | **effect on deployment:** a build with this change starts with candidate memory that does not match the old keys, so cooldown and score history from before it are not reused once; this is by design and will be visible in the W19 deploy |
| Parser robustness (W16) | seeded fuzz-style tests for the registration protocol, the KPI ingest, CPU lists and command-line parameters, the hyperfine importer, the artifact manifest, the ELF build-id reader and the frame-log reader: none panics, state stays bounded, and only exact valid requests are accepted | found and fixed one real defect: a 12-byte CPU list `0-4294967295` would have expanded to about 16 GiB; `parse_cpu_list` now refuses any id above 65535 |
| Observer effect (W7) | the always-on sensors cost about 0.6 ms per evaluation in a debug build (PSI, CPU power, NUMA, DAMON, resctrl, user-slice OOM guard) and the owner probe 3.3 ms per scan of this host's 343 processes, once per 30 s; both are asserted below generous ceilings in tests | measured on the development host in a debug build, not under the installed service; the exact-build smoke at W19 re-measures idle CPU with these sensors included |
| Closeout report (W19) | `xtask closeout-report` renders every ledger entry with its disposition, the counts, and the unavailable physical proof with its unblock condition; it refuses an unknown disposition, `CodeComplete` on a non-`code_complete` entry, a physical-proof disposition without evidence, and `HardwareUnavailable` without an unblock condition, and exits non-zero while any entry is not `code_complete` | a draft run against this source reports 8 of 34 entries not `code_complete`, so the closeout gate is not met; the report is local and is not committed |
| Experiment statistics | existing LiveExperimentManager has a separate observation-only study interface: freeze contract/design/guards before collection, consume collector-origin raw blocks, recompute metrics and intervals, enforce counterbalanced independent pairs and stopping rules; keep evidence and candidate memory are unchanged | no new live keep/apply authority; disk imports cannot construct trusted capture objects; broader workload calibration, holdout and service acceptance remain open |
| Real workloads | existing supervised real-workload runner plus transient-controller frame observations and `xtask kpi-observation run --out …`, eight real fixed-work executions through runtime KPI ingestion/raw-block reconstruction | real-game cohort remains BlockedResource; frame-primary clock proof, further faults, phase/recovery acceptance and final installed build remain open |

Development measurement results: [controlled frame controller](validation/controller-frame-2026-11-08/RESULTS.md),
[KPI observation contract and workflow](KPI_OBSERVATION.md). These do not change the 23/34 ledger
count or grant a new Supported tuple. The fixed-work harness supplies its own monitor ticks and is
not installed-service or scheduler qualification.

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
