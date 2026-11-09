# Closeout report

Source `4f3baa07aface5cdf355095e2de82ece4ebc4c21`, kernel `6.18.57`. Generated from the integration coverage ledger; it cannot claim more than the ledger records.

## Disposition counts

| disposition | entries |
|---|---|
| EvidenceDeferred | 34 |
| HardwareUnavailable | 23 |
| SafetyAborted | 0 |
| FixtureVerified | 34 |
| RealObserved | 19 |
| CodeComplete | 15 |
| MechanismValidated | 0 |
| BenefitValidated | 0 |
| AutonomySupported | 0 |

## Entries

| id | tools | packages | status | dispositions | not wired |
|---|---|---|---|---|---|
| 5.1 | GameMode | W2 W6 W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.2 | Ananicy, Ananicy-cpp, community rules | W6 W8 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.3 | System76 Scheduler | W6 W12 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.4 | TuneD | W6 W11d W13 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.5 | Oracle bpftune | W6 W9 W11f | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.6 | sched_ext, scx_lavd, scx_bpfland, other SCX schedulers | W6 W9 W11b W12 | code_complete | FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.7 | scx_loader, scxctl, scxtui | W6 W11b W13 | code_complete | CodeComplete, FixtureVerified, HardwareUnavailable, EvidenceDeferred | no real scx_loader here, so no real switch/restore or matched-workload campaign; not reachable from the daemon, providers or remote callers by construction (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false) |
| 5.8 | DAMON, DAMOS, damo | W9 W11c W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | no quota-goal feedback, no damo parity manifest, no hot/cold region summary, not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no real-hardware readback here |
| 5.9 | hwloc | W8 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.10 | perf, perf_event_open, Intel VTune, AMD uProf, LIKWID | W7 W8 W9 W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | the PMU sensor starting only on escalation is by design (cost-bounded sensors); the architecture events are catalogued and resolved but not opened by the runtime; importer formats are fixture-verified from the documented layouts (the tools are absent here) |
| 5.11 | BCC, bpftrace, trace-cmd, ftrace, KernelShark, rtla, cyclictest | W8 W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | importer formats for rtla are fixture-verified only (no rtla on this host); kernel noise is diagnostic, never an application outcome and never authority |
| 5.12 | MangoHud, Gamescope, Wayland presentation, DRM/KMS | W2 W3 W4 W9 | code_complete | FixtureVerified, EvidenceDeferred, RealObserved | Qualified production-time/flush clock bound and further installed-service/phase/crash qualification remain deferred; no new frame-primary support claim. |
| 5.13 | LACT, CoreCtrl | W6 W9 W11d | code_complete | FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.14 | OpenTuner | W10 W15 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.15 | Kernel Tuner, OtterTune, Bayesian/SMBO, contextual bandits | W13 W15 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.16 | TLP, auto-cpufreq, power-profiles-daemon | W6 W9 W11d | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | Trusted production campaign/token factory and worker/journal integration are absent; the scoped hold mechanism is fake-bus tested, and OfflineAuthorization has a fixture-only constructor. |
| 5.17 | thermald, CPUFreq, intel_pstate, amd_pstate | W9 W11d W12 | code_complete | CodeComplete, FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | sensor readings are deliberately not decision inputs (architecture guard `decision_code_never_reads_sensor_readings`); no new EPP benefit campaign (the prior negative pilot is retained, not rerun); thermald absent on this host; RAPL energy is root-only here |
| 5.18 | irqbalance | W6 W9 W11e | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.19 | numad, automatic NUMA balancing, numactl | W8 W9 W11c | code_complete | CodeComplete, FixtureVerified, HardwareUnavailable, EvidenceDeferred | set_mempolicy/mbind apply only to the calling thread or its own address space, so no foreign-process policy operation exists by design; no numad coexistence beyond the owner probe; not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no real multi-node migration readback |
| 5.20 | systemd-oomd, uresourced | W6 W9 W12 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.21 | cgroups v2, systemd resource control, OpenRC | W5 W6 W11a W14 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); systemd1 SetUnitProperties adapter and a real OpenRC host acceptance are not done and are not represented as passed; no per-knob real-device campaign |
| 5.22 | PSI | W7 W9 | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| 5.23 | PM QoS | W9 W11d | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | no provider proposes it and no worker operation carries it: by design a HighRisk manual/offline operation behind the HIGH_RISK_APPLY_IMPLEMENTED=false denial; no real-device mechanism campaign; active-config snapshots do not read the aggregate target |
| 5.24 | resctrl, Intel RDT, AMD QoS | W9 W11c | code_complete | CodeComplete, FixtureVerified, HardwareUnavailable, EvidenceDeferred | no MBA, no task assignment, not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no real-hardware readback here |
| 5.25 | GCC/Clang PGO, AutoFDO, LLVM BOLT | W13 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.26 | ionice, BFQ, Kyber, mq-deadline, fio, blktrace, biosnoop, biolatency | W9 W11a W11e W13 | code_complete | CodeComplete, FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); no per-process offender I/O accounting source beyond the compile/browser helper selectors; no real-device scheduler campaign; no blktrace/biosnoop importer |
| 5.27 | congestion controls, socket policy, iperf3, wrk, wrk2, h2load | W4 W9 W11f W13 | code_complete | CodeComplete, FixtureVerified, HardwareUnavailable, EvidenceDeferred, RealObserved | no OpenTelemetry reader (not required by the plan row); no local offered-load network execution and no exact installed-production acceptance (tools absent, no disposable service); not worker-routed (HighRisk manual/offline behind HIGH_RISK_APPLY_IMPLEMENTED=false); socket-level options and qdisc selection are not implemented as operations; Prometheus import and compare-load remain read-only |
| 5.28 | hyperfine, benchstat, Phoronix Test Suite, sysbench, stress-ng | W1 W10 W13 | code_complete | FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | nothing |
| 5.29 | taskset, numactl, nice, chrt, systemd-run, cpupower | W5 W6 W11g | code_complete | CodeComplete, FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| extra.mpi | Intel MPI autotuning concept | W15 | code_complete | FixtureVerified, EvidenceDeferred | nothing |
| extra.diag | tuna, iostat, pidstat, numastat, blktrace | W8 W9 W13 | code_complete | FixtureVerified, HardwareUnavailable, EvidenceDeferred | nothing |
| extra.kernels | PREEMPT_RT, Zen, Liquorix, CachyOS, custom kernels | W14 | code_complete | FixtureVerified, RealObserved, HardwareUnavailable, EvidenceDeferred | nothing |
| extra.boot | nohz_full, isolcpus, rcu_nocbs, housekeeping, IRQ/RCU isolation | W14 | code_complete | FixtureVerified, RealObserved, EvidenceDeferred | nothing |
| extra.workloads | low-latency audio, desktop and browser interactivity, fixed-work compilation | W4 W12 | code_complete | FixtureVerified, RealObserved, EvidenceDeferred | nothing |

## Unavailable physical proof and what unblocks it

* **5.2** (Ananicy, Ananicy-cpp, community rules): Needs on the host: absent
* **5.3** (System76 Scheduler): Needs on the host: absent
* **5.4** (TuneD): Needs on the host: absent
* **5.5** (Oracle bpftune): Needs on the host: absent
* **5.7** (scx_loader, scxctl, scxtui): scx_loader is not installed or running on this host: needs org.scx.Loader on the system bus and a sched_ext kernel for a real switch/restore readback and the matched-workload comparison campaign
* **5.8** (DAMON, DAMOS, damo): DAMON admin sysfs is root-only for the controller user on this host: needs an authorized root-side run with an isolated workload, direct KPI and OOM/pressure guards for a real readback and any reclaim study
* **5.9** (hwloc): Needs on the host: absent
* **5.10** (perf, perf_event_open, Intel VTune, AMD uProf, LIKWID): perf CLI, VTune, uProf and LIKWID are absent on this host, and the optional six-event group failed to give usable counts here: real captures from each tool and a PMU that supports the wider event group are needed to call their importers and the architecture events RealObserved
* **5.11** (BCC, bpftrace, trace-cmd, ftrace, KernelShark, rtla, cyclictest): all external diagnostic tools are absent on this host: importer formats are fixture-verified from the documented layouts and need a real capture from each tool to be called RealObserved
* **5.12** (MangoHud, Gamescope, Wayland presentation, DRM/KMS): Verified producer timestamps and flush/error bounds meeting 2 ms, plus source-bound service acceptance; preserve fail-closed behavior. No threshold relaxation or GPU profile experiments.
* **5.13** (LACT, CoreCtrl): Needs on the host: LACT present, CoreCtrl absent
* **5.14** (OpenTuner): Needs on the host: not installed
* **5.15** (Kernel Tuner, OtterTune, Bayesian/SMBO, contextual bandits): Needs on the host: not installed
* **5.16** (TLP, auto-cpufreq, power-profiles-daemon): Needs on the host: all absent
* **5.17** (thermald, CPUFreq, intel_pstate, amd_pstate): thermald is absent on this host; a per-policy EPP benefit campaign is not repeated because the prior ~0.3% EPP pilot is negative and retained; RAPL energy is root-only here
* **5.18** (irqbalance): Needs on the host: absent
* **5.19** (numad, automatic NUMA balancing, numactl): this host has a single NUMA node and no numad or numactl: a real node-to-node migration readback and locality/KPI campaign needs a multi-node machine and an isolated authorized workload
* **5.21** (cgroups v2, systemd resource control, OpenRC): Real-device campaigns per knob and the systemd1 SetUnitProperties and installed OpenRC backend acceptance: need an authorized delegated subtree with an isolated workload and direct KPI on a real OpenRC host
* **5.23** (PM QoS): Real-device mechanism and benefit campaign: needs an authorized, isolated workload with a direct latency KPI; the lease itself is system-wide and stays manual-only.
* **5.24** (resctrl, Intel RDT, AMD QoS): resctrl is not mounted on this host: needs an x86 CPU with RDT/QoS allocation and a mounted resctrl filesystem for a real-hardware readback and a target-versus-bystander campaign
* **5.25** (GCC/Clang PGO, AutoFDO, LLVM BOLT): Needs on the host: clang, gcc, llvm-bolt, perf2bolt absent on PATH
* **5.26** (ionice, BFQ, Kyber, mq-deadline, fio, blktrace, biosnoop, biolatency): blktrace/biosnoop/biolatency/fio absent; a real-device scheduler mechanism and target-versus-bystander campaign needs an explicitly disposable authorized block device (none is designated on this host)
* **5.27** (congestion controls, socket policy, iperf3, wrk, wrk2, h2load): HTTP/RPC/network physical proof requires an owned local disposable service, versioned benchmark tools (iperf3, wrk, wrk2, h2load are absent here), fixed offered load and direct latency/error/completion guards; the CPU fixed-work transport evidence is not a network qualification, and bpftune is absent so no cooperative strategy was exercised
* **5.28** (hyperfine, benchstat, Phoronix Test Suite, sysbench, stress-ng): Needs on the host: all absent
* **extra.diag** (tuna, iostat, pidstat, numastat, blktrace): Needs on the host: all absent
* **extra.kernels** (PREEMPT_RT, Zen, Liquorix, CachyOS, custom kernels): Needs on the host: RT, Zen, Liquorix and CachyOS kernel specialisations
