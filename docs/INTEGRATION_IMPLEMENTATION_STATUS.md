# Integration implementation status

This is the human-readable companion of [`integration-coverage.json`](integration-coverage.json), the
machine-readable ledger of every tool and mechanism in the current plan. It records what exists at
the verification baseline named in the ledger's `source_sha` (currently `fc1bb2e7`; [receipt](validation/source-ci-2026-11-11-fc1bb2e7.json)); the deployed production source is still `4f3baa07` ([receipt](validation/source-ci-2026-11-09-4f3baa07.json)) and nothing since then is reachable from the daemon. The wiring table also describes newer development changes and links their evidence below; it does not silently promote that verification pin. Live observations keep their own source pins. The production-deployed build is a separate, older pin (`4f3baa07`, as stated above) and is not described by this file. The grades are
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
| `first_slice` | 0 |
| `not_started` | 0 |
| `code_complete` | 34 (after the subcapability audit: 5.16 receipt-verified authorization factory with the runner request that issues receipts, 5.8 DAMOS quota goals, region summary and damo parity manifest, 5.19 memory-policy refusal and numad guard, 5.21 systemd SetUnitProperties adapter and OpenRC service-manager backend, 5.24 resctrl MBA and task assignment, 5.26 offender attribution and biosnoop/blkparse importers, 5.27 network inventory, refusals and offered-load workflow; plus 5.10 PMU importers and event catalog, 5.11 external diagnostic importers and bounded probes, 5.17 per-policy EPP with thermal protection, 5.7 sched_ext strategy switch, 5.23 PM QoS lease and manual action, extra.workloads KPI templates, 5.9 hwloc cross-check, 5.3 and 5.5 typed capability probes, 5.4 TuneD profile and artifact, extra.boot and extra.kernels cross-boot campaign, 5.12 frame-source path, 5.6 sched_ext sensor, comparison and semantics, 5.25 offline optimization workflow, 5.18 IRQ attribution, 5.13 GPU identity observer, 5.2 role hints and conflict audit, 5.15 measurement import and Bayesian policy, 5.14 OpenTuner-style ensemble, extra.mpi search method, 5.1 GameMode, 5.20 OOM guards, 5.22 PSI, 5.28 benchmark importers, extra.diag diagnostic parsers, 5.29 native controls and user intent) |
| entries with a G1 real observation | 20 carry `RealObserved` (see the ledger for each one's exact observation and source pin), for example GameMode, LACT, systemd-oomd, sched_ext state and a real `scx_loader` run, placement |
| entries with G2 or G3 evidence for something new | 0 |

The Supported autonomous boundary is unchanged: CPU-affinity profile and cgroup placement for a stable
game under scheduler pressure judged by `GameRunnableLatency`. Nothing in this ledger widens it.

## What is wired and what is not

| Area | Wired into the live runtime | Present but not wired |
|---|---|---|
| Process safety | xtask stages now require an owned kernel cgroup, with a private birth journal, pidfd TERM/grace/KILL, independent owner-crash reaper and inode-bound cleanup; marker-clearing and setsid children are covered; output capture and cumulative CI deadlines remain bounded | the privileged runner now creates an owned cgroup for every non-delegated action, bounds its logs and deadlines, and contains a delegated suite subtree (`contain_end`, plus an independent `DelegatedReaper` that kills the subtree when its owner dies, with real-kernel live tests including an owner `SIGKILL`); what is not claimed is installed-service qualification of these primitives or any campaign that grants the new offline operations |
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
| Subcapability gate | `docs/integration-subcapabilities.json` lists plan-derived subcapabilities (a baseline that may not be dropped) each with a code anchor and a positive and a negative test; `xtask closeout-report` and an xtask test verify every anchor and test exists, so clearing a ledger `missing_required` list is not proof. Items are typed `operation`, `observation` or `refusal` and the report counts refusals separately: a typed refusal explains an unsafe or impossible scope and is not an actuator. `closeout-report --strict` additionally needs a clean tree and only documentation changes since the ledger `source_sha` | verified by `the_repository_subcapability_manifest_verifies_against_its_anchors_and_tests` and the negative manifest test |
| Closeout report (W19) | `xtask closeout-report` renders every ledger entry with its disposition, the counts, and the unavailable physical proof with its unblock condition; it refuses an unknown disposition, `CodeComplete` on a non-`code_complete` entry, a physical-proof disposition without evidence, and `HardwareUnavailable` without an unblock condition, and exits non-zero while any entry is not `code_complete` or `code_complete` lists a missing required subcapability | the run against `4f3baa07` reported 0 of 34 entries not `code_complete` and exited zero, but that gate did not check subcapabilities; after the audit and the work that followed, every entry is `code_complete` with an empty `missing_required` list again and the report exits zero; the rendered local report is `docs/validation/closeout-report-2026-11-09.md` and records every unavailable physical proof with its unblock condition |
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
* sched_ext is present and disabled (the kernel enable/disable path was observed with `scx_simple` and `scx_bpfland`; the 5.7 adapter was also run against a real `scx_loader` 1.1.3 built from crates.io on a *private* D-Bus with a permissive stub polkit authority, so the loader's own authorization layer, a packaged system service and any workload benefit are not exercised, and the kernel scheduler switch itself was global); DAMON's sysfs interface is present; resctrl is not mounted;
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

## Status and disposition semantics

`status = code_complete` (the implementation is finished and fixture-tested, with no missing required
subcapability) and the `CodeComplete` disposition mean the same thing and are required together: an
entry carries both or neither, and `xtask closeout-report` refuses either alone. All 34 entries
carry both. Every other disposition is a separate rung of the evidence ladder and is never implied
by `CodeComplete`: `FixtureVerified` (fixtures), `RealObserved` (a real observation), `MechanismValidated`
(a physical mechanism check) and `BenefitValidated` (a measured benefit) each need their own evidence,
and `HardwareUnavailable` / `EvidenceDeferred` record what cannot be shown here and why.

The subcapability manifest (`integration-subcapabilities.json`) is checked against a plan baseline
that is frozen in the closeout code, not only in the JSON; every item names the file that declares its
positive and negative test, which must be a `#[test]` function there, and `closeout-report --strict`
(or `--run-tests`) executes each non-ignored test and requires it to pass exactly once. Ignored (live)
tests are verified to exist but need their own privileged resources.

## Evidence added 2026-11-11 (W21–W27), without any promotion

| phase | result | record |
|---|---|---|
| W21 renderer producer | a real renderer (vkcube + opt-in Vulkan layer) binds `ValidatedDirect`, 2400 frames, 0 gaps; direct-frame keep denied (no W22 reference) | `validation/renderer-producer-2026-11-11/` |
| W22 stamp accuracy | `NotPlannedForVerification` (needs photodiode/camera/display timestamp hardware) | ledger 5.12 |
| W23 workload and KPI | cold fixed-work compile, CV 1.62 % over 12 executions | `validation/w23-compile-cohort-2026-11-11/` |
| W24 protected benefit | background nice 19 Beneficial (-29.5 %), one-CPU control Harmful (+72.3 %); synthetic contention, real-game study stays `BlockedResource` | `validation/w24-protected-compile-2026-11-11/` |
| W25 selection | held-out ranking beat the conservative baseline by 9.4 points; default selection unchanged (candidates are not a legal Stutter action) | `validation/w25-held-out-ranking-2026-11-11/` |
| W26 overhead | installed services 2.60 % of one core, constant RSS/FDs, 2–4 kB/s writes | `validation/w26-observer-cost-2026-11-11/` |
| W27 cgroups | owned-scope `cpu.weight`/`memory.high` readback; `io` and `cpuset` not delegated; OpenRC `NotPlannedForVerification` | `validation/w27-cgroup-readback-2026-11-11/` |
| W29 | multi-node NUMA, RDT, kernel-owned kdamond: `NotPlannedForVerification` | ledger 5.19, 5.24, 5.8 |
| W30 | local `wrk2` offered-load, `hyperfine` import, real hwloc cross-check captured | `validation/phase-tools-2026-11-11/` |

Later the same day W28 (live sched_ext coexistence), W29/W31 blockers and the W32 review were recorded: `validation/w28-w29-w31-host-facts-2026-11-11/`, `validation/w32-review-2026-11-11/`. The real-game frame benefit study is `NotPlannedForVerification` (needs a photodiode, camera or display-timestamp reference this machine lacks).
Later still, W28's owner-conflict refusal was shown on live host state with the production `probe`, `knob_is_honored` and `owner_denial` (`xtask owner-refusal-probe`): with `scx_bpfland` loaded nice, uclamp and cgroup cpu.weight are not honored and the planner refuses Nice/Uclamp (`SchedulerSemanticsUnverified`); GameMode (`gamemoded` running) independently owns Nice (`ExternallyOwnedResource`). Evidence: `validation/w28-owner-refusal-live-2026-11-11/`. This supersedes the earlier "not applicable" closure; the limit is that it is the production decision functions on real owner state, not the candidate pipeline inside the service.
