# Closeout report

Source `04c6988856835e1ea1da3d6c714de6a6070360ff`, kernel `6.18.57`. Generated from the integration coverage ledger; it cannot claim more than the ledger records.

## Disposition counts

| disposition | entries |
|---|---|
| NotPlannedForVerification | 10 |
| EvidenceDeferred | 31 |
| HardwareUnavailable | 0 |
| SafetyAborted | 0 |
| FixtureVerified | 34 |
| RealObserved | 21 |
| CodeComplete | 34 |
| MechanismValidated | 4 |
| BenefitValidated | 0 |
| AutonomySupported | 0 |

## Entries

| id | tools | packages | status | dispositions | not wired |
|---|---|---|---|---|---|
| 5.1 | GameMode | W2 W6 W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.2 | Ananicy, Ananicy-cpp, community rules | W6 W8 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| 5.3 | System76 Scheduler | W6 W12 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| 5.4 | TuneD | W6 W11d W13 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| 5.5 | Oracle bpftune | W6 W9 W11f | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| 5.6 | sched_ext, scx_lavd, scx_bpfland, other SCX schedulers | W6 W9 W11b W12 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.7 | scx_loader, scxctl, scxtui | W6 W11b W13 | code_complete | RealObserved, CodeComplete, FixtureVerified, EvidenceDeferred, NotPlannedForVerification | no real scx_loader here, so no real switch/restore or matched-workload campaign; not reachable from the daemon, providers or remote callers by construction (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false) |
| 5.8 | DAMON, DAMOS, damo | W9 W11c W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred, NotPlannedForVerification | not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no real-hardware readback here |
| 5.9 | hwloc | W8 | code_complete | CodeComplete, FixtureVerified, RealObserved | nothing |
| 5.10 | perf, perf_event_open, Intel VTune, AMD uProf, LIKWID | W7 W8 W9 W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred, NotPlannedForVerification | the PMU sensor starting only on escalation is by design (cost-bounded sensors); the architecture events are catalogued and resolved but not opened by the runtime; importer formats are fixture-verified from the documented layouts (the tools are absent here) |
| 5.11 | BCC, bpftrace, trace-cmd, ftrace, KernelShark, rtla, cyclictest | W8 W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | importer formats for rtla are fixture-verified only (no rtla on this host); kernel noise is diagnostic, never an application outcome and never authority |
| 5.12 | MangoHud, Gamescope, Wayland presentation, DRM/KMS | W2 W3 W4 W9 | code_complete | CodeComplete, EvidenceDeferred, FixtureVerified, NotPlannedForVerification, RealObserved | Qualified production-time/flush clock bound and further installed-service/phase/crash qualification remain deferred; no new frame-primary support claim. |
| 5.13 | LACT, CoreCtrl | W6 W9 W11d | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred, NotPlannedForVerification | nothing |
| 5.14 | OpenTuner | W10 W15 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred, NotPlannedForVerification | nothing |
| 5.15 | Kernel Tuner, OtterTune, Bayesian/SMBO, contextual bandits | W13 W15 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred, NotPlannedForVerification | nothing |
| 5.16 | TLP, auto-cpufreq, power-profiles-daemon | W6 W9 W11d | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | The trusted factory exists: `campaign_authorization::mint_authorization` verifies a root-written receipt (every path component root-owned and not group or world writable, no symlinks; boot, user, scope and expiry checked) and journals the issue durably before returning an authorization, and the runner has `receipt::write_receipt` to produce one. The runner request `IssueOfflineReceipt` (scope and lifetime bounded, derived campaign, user and path, denied unless the campaign authorization grants that action) writes the receipt; no shipped campaign grants it yet, and the adapters are HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false, so nothing reaches them at runtime; the scoped hold mechanism is fake-bus tested. |
| 5.17 | thermald, CPUFreq, intel_pstate, amd_pstate | W9 W11d W12 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | sensor readings are deliberately not decision inputs (architecture guard `decision_code_never_reads_sensor_readings`); no new EPP benefit campaign (the prior negative pilot is retained, not rerun); thermald absent on this host; RAPL energy is root-only here |
| 5.18 | irqbalance | W6 W9 W11e | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| 5.19 | numad, automatic NUMA balancing, numactl | W8 W9 W11c | code_complete | CodeComplete, FixtureVerified, NotPlannedForVerification | set_mempolicy/mbind apply only to the calling thread or its own address space, so no foreign-process policy operation exists by design (`memory_policy` states that as a typed refusal and observes the policy read-only from numa_maps); a running numad denies the migration action before any page is touched; not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no real multi-node migration readback |
| 5.20 | systemd-oomd, uresourced | W6 W9 W12 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.21 | cgroups v2, systemd resource control, OpenRC | W5 W6 W11a W14 | code_complete | CodeComplete, EvidenceDeferred, FixtureVerified, MechanismValidated, RealObserved | not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); the systemd1 SetUnitProperties adapter (runtime-only, Stutter-owned `stutter-*` units, five bounded properties, readback and restore) is fake-bus verified and gated behind OfflineAuthorization so no runtime path invokes it; the OpenRC cgroup layout (`service_manager`) is fixture-verified only and a real OpenRC host acceptance is not done and is not represented as passed; no per-knob real-device campaign |
| 5.22 | PSI | W7 W9 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.23 | PM QoS | W9 W11d | code_complete | CodeComplete, EvidenceDeferred, FixtureVerified, MechanismValidated, RealObserved | no provider proposes it and no worker operation carries it: by design a HighRisk manual/offline operation behind the HIGH_RISK_APPLY_IMPLEMENTED=false denial; no real-device mechanism campaign; active-config snapshots do not read the aggregate target |
| 5.24 | resctrl, Intel RDT, AMD QoS | W9 W11c | code_complete | CodeComplete, FixtureVerified, NotPlannedForVerification | not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no real-hardware readback here |
| 5.25 | GCC/Clang PGO, AutoFDO, LLVM BOLT | W13 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| 5.26 | ionice, BFQ, Kyber, mq-deadline, fio, blktrace, biosnoop, biolatency | W9 W11a W11e W13 | code_complete | CodeComplete, EvidenceDeferred, FixtureVerified, MechanismValidated, RealObserved | not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no real-device scheduler campaign on a production disk (a loop-device mechanism test exists) |
| 5.27 | congestion controls, socket policy, iperf3, wrk, wrk2, h2load | W4 W9 W11f W13 | code_complete | CodeComplete, EvidenceDeferred, FixtureVerified, MechanismValidated, RealObserved | no OpenTelemetry reader (not required by the plan row); the local offered-load workflow (`xtask benchmarks offered-load`, wrk2, loopback only) has not been run here because wrk2 is not installed (it reports BlockedResource), and there is no exact installed-production acceptance; not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); socket options and qdisc replacement are deliberately not operations: `net_capability` gives the read-only congestion-control and qdisc inventory and typed refusals that state why (a socket option needs the owning descriptor; replacing a root qdisc cannot be restored exactly); Prometheus import and compare-load remain read-only |
| 5.28 | hyperfine, benchstat, Phoronix Test Suite, sysbench, stress-ng | W1 W10 W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.29 | taskset, numactl, nice, chrt, systemd-run, cpupower | W5 W6 W11g | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| extra.mpi | Intel MPI autotuning concept | W15 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| extra.diag | tuna, iostat, pidstat, numastat, blktrace | W8 W9 W13 | code_complete | CodeComplete, FixtureVerified, EvidenceDeferred | nothing |
| extra.kernels | PREEMPT_RT, Zen, Liquorix, CachyOS, custom kernels | W14 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred, NotPlannedForVerification | nothing |
| extra.boot | nohz_full, isolcpus, rcu_nocbs, housekeeping, IRQ/RCU isolation | W14 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| extra.workloads | low-latency audio, desktop and browser interactivity, fixed-work compilation | W4 W12 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |

## Unavailable physical proof and what unblocks it

* **5.2** (Ananicy, Ananicy-cpp, community rules): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.3** (System76 Scheduler): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.4** (TuneD): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.5** (Oracle bpftune): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.7** (scx_loader, scxctl, scxtui): Not planned for verification (the packaged scx_loader service with a real polkit authority is not in the pinned nixpkgs; scx schedulers themselves are installed; development targets this machine only)
* **5.8** (DAMON, DAMOS, damo): Not planned for verification (the kdamond is owned by an in-kernel module on this host and a second one is refused; a kdamond-free host is different hardware/configuration; development targets this machine only)
* **5.9** (hwloc): none: a real hwloc XML export agrees with the native topology (stutter topology-check, 40 objects)
* **5.10** (perf, perf_event_open, Intel VTune, AMD uProf, LIKWID): perf and LIKWID installed; Intel VTune and AMD uProf are vendor tools not planned for verification (development targets this machine only); the optional six-event group yielded no usable counts on this PMU
* **5.11** (BCC, bpftrace, trace-cmd, ftrace, KernelShark, rtla, cyclictest): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.12** (MangoHud, Gamescope, Wayland presentation, DRM/KMS): W21 done: a real instrumented renderer (vkcube + VK_LAYER_STUTTER_present) binds and is ValidatedDirect with 0 gaps (docs/validation/renderer-producer-2026-11-11/RESULTS.md). Not planned for verification (needs a photodiode, high-speed camera or display hardware timestamp, which this machine lacks; development targets this machine only): the independent stamp-accuracy reference, so direct-frame keep stays denied and the frame objective stays measurement-only
* **5.13** (LACT, CoreCtrl): Not planned for verification (CoreCtrl is an AMD-GPU tool; this host's GPU is under quarantined LACT policy)
* **5.14** (OpenTuner): Not planned for verification (OpenTuner, Kernel Tuner, OtterTune and SMBO libraries are not packaged for this host and add no capability the typed search does not already cover; development targets this machine only)
* **5.15** (Kernel Tuner, OtterTune, Bayesian/SMBO, contextual bandits): Not planned for verification (OpenTuner, Kernel Tuner, OtterTune and SMBO libraries are not packaged for this host and add no capability the typed search does not already cover; development targets this machine only)
* **5.16** (TLP, auto-cpufreq, power-profiles-daemon): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.17** (thermald, CPUFreq, intel_pstate, amd_pstate): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.18** (irqbalance): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.19** (numad, automatic NUMA balancing, numactl): Not planned for verification (needs different hardware; development targets this machine only): this host has a single NUMA node and no numad or numactl: a real node-to-node migration readback and locality/KPI campaign needs a multi-node machine and an isolated authorized workload
* **5.21** (cgroups v2, systemd resource control, OpenRC): Real-device campaigns per knob and the systemd1 SetUnitProperties and installed OpenRC backend acceptance: need an authorized delegated subtree with an isolated workload and direct KPI on a real OpenRC host
* **5.23** (PM QoS): Real-device mechanism and benefit campaign: needs an authorized, isolated workload with a direct latency KPI; the lease itself is system-wide and stays manual-only.
* **5.24** (resctrl, Intel RDT, AMD QoS): Not planned for verification (needs different hardware; development targets this machine only): resctrl is not mounted on this host: needs an x86 CPU with RDT/QoS allocation and a mounted resctrl filesystem for a real-hardware readback and a target-versus-bystander campaign
* **5.25** (GCC/Clang PGO, AutoFDO, LLVM BOLT): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **5.26** (ionice, BFQ, Kyber, mq-deadline, fio, blktrace, biosnoop, biolatency): tools installed; a scheduler mechanism campaign needs an explicitly disposable block device, none is designated on this host
* **5.27** (congestion controls, socket policy, iperf3, wrk, wrk2, h2load): tools installed; a loopback wrk2 offered-load run is captured (docs/validation/phase-tools-2026-11-11/RESULTS.md); the controlled A/B network benefit campaign remains
* **5.28** (hyperfine, benchstat, Phoronix Test Suite, sysbench, stress-ng): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **extra.diag** (tuna, iostat, pidstat, numastat, blktrace): tools installed; a real capture or campaign from each is the remaining evidence, recorded per slice in docs/validation/phase-tools-2026-11-11/RESULTS.md
* **extra.kernels** (PREEMPT_RT, Zen, Liquorix, CachyOS, custom kernels): Not planned for verification (RT, Zen, Liquorix and CachyOS kernels need reboots into other kernels; development targets this machine only)

## Verified subcapabilities

28 items, each with a code anchor and a positive and a negative test: 11 operations, 14 observations, 3 typed refusals (a refusal explains an unsafe or impossible scope; it is not an actuator).

| id | dossier | kind |
|---|---|---|
| damos/quota-goal-feedback | 5.8 | operation |
| damos/hot-cold-region-summary | 5.8 | observation |
| damos/damo-parity-manifest | 5.8 | observation |
| damos/target-identity-recheck | 5.8 | operation |
| power/campaign-token-factory-and-worker-journal-integration | 5.16 | operation |
| power/scope-bound-authorization | 5.16 | operation |
| power/runner-request-that-issues-receipts | 5.16 | operation |
| numa/process-scoped-memory-policy-refusal-contract | 5.19 | refusal |
| numa/numad-coexistence-behaviour | 5.19 | operation |
| cgroup/systemd-set-unit-properties-adapter | 5.21 | operation |
| cgroup/openrc-backend | 5.21 | observation |
| resctrl/mba-allocation | 5.24 | operation |
| resctrl/verified-task-assignment | 5.24 | operation |
| blockio/per-process-offender-attribution | 5.26 | observation |
| blockio/blktrace-biosnoop-importers | 5.26 | observation |
| net/socket-level-options | 5.27 | refusal |
| net/qdisc-selection | 5.27 | refusal |
| net/local-offered-load-workflow | 5.27 | operation |
| sched_ext/loader-settle-and-versioned-ops-match | 5.7 | operation |
| frames/stamped-calibration-never-stale | 5.12 | observation |
| frames/authenticated-epoch-bound-source | 5.12 | observation |
| frames/producer-identity-and-epoch-binding | 5.12 | observation |
| frames/fail-closed-input-and-counters | 5.12 | observation |
| frames/verified-stamp-accuracy-claims | 5.12 | observation |
| frames/decision-time-revalidation | 5.12 | observation |
| frames/executable-image-binding | 5.12 | observation |
| frames/trusted-capture-provenance | 5.12 | observation |
| frames/unique-monotonic-reference-pairs | 5.12 | observation |
