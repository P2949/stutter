# Core Validation — current execution map

Scope: take the repository from **Core Validation** to a defensible first supported
release (`v0.1.0`). **Feature freeze** is in force: no new mutation providers,
IRQ/VM/power tuning, scheduler integrations, daemon modes, remote mutation APIs,
UI surfaces, unrelated crate splits, or telemetry backends. Remaining work is
validation, evidence, calibration, safety closure, and release engineering.

Authoritative tracking is GitHub issues (milestone
[Core Validation](https://github.com/P2949/stutter/milestone/1)). Dated history,
red tests, commits, CI IDs and evidence receipts live in
[CORE_VALIDATION_STATUS.md](CORE_VALIDATION_STATUS.md).
[CORE_VALIDATION_BACKLOG.json](CORE_VALIDATION_BACKLOG.json) is the historical
receipt of the initial #22–#38 backlog and is not an active tracker.
Objective maturity is owned by `ObjectiveKind::descriptor()` and generated into
[AUTOTUNE_OBJECTIVES.md](AUTOTUNE_OBJECTIVES.md).

## Settled foundation

Objective correctness (direct runnable latency, fixed-work compile primary,
descriptor registry); startup identity/layout hardening; provider-specific
automatic-recovery authorization; rollback token persisted before verification
and before the first affinity mutation; failed-apply fault reporting; the physical
recovery matrix and real suspend/resume (PASS, indexed); portable evidence tooling;
the capability-tier registry; the release package and verifier.

Transaction under test:

```text
durable restore snapshot → Applying(token) → first mutable syscall
  → Applied(token) → verify → Measuring / later lifecycle
```

Do not redesign it unless a real acceptance test exposes a defect.

## Remaining sequence (dependency order)

Offline readiness (done once merged, then freeze one evidence base commit):
DRM-aware kernel compatibility (#30), observation vs causal confidence (#32),
frame-aware `StutterScore` evaluation (#33), exact-release evidence gate (#36), and the
representative real-workload overhead path (#38). Do not collect privileged or kernel
evidence against code that is still changing.

| Step | Needs | Work | Issues |
| --- | --- | --- | --- |
| 1 | root/`CAP_BPF` | Observation, controlled `GameRunnableLatency`, controlled overhead (A-F) plus a representative real-workload overhead run, controlled runnable `StutterScore` | #27, #38, #33 |
| 2 | three real kernels | `live-acceptance kernel-compat` on at least three distinct kernel releases | #30 |
| 3 | real recordings | Diagnosis corpus to the evidence floors (per family: real positives, matched negatives, mixed/confounded) | #31 |
| 4 | corpus | `calibrate --family scheduler\|io\|irq\|gpu`: threshold and observation/causal-confidence decisions (retain is a valid outcome) | #32 |
| 5 | frame-producing workload | `score-evaluate --require-frames` over 0/1/2/4/8/16 ms | #33 |
| 6 | steps 1-5 | Evidence maturity closure (`index.json`, `fixture-coverage`) and capability tiers (`capability-check`) | #34, #35 |
| 7 | maintainer | Freeze the release candidate commit (`RC`); no evidence-index or doc commits onto it | #36 |
| 8 | exact `RC` | Offline acceptance, fresh live suites for every Supported capability, `release-verify --require-acceptance --acceptance-bundle ...`, package-install smoke, tag `v0.1.0` at `RC` | #36 |

Exported bundles are verified again (`evidence-verify`) and indexed with `evidence-index`;
index entries are never hand-written. Exact-release acceptance consumes external bundles,
not the committed index. Maintainer decisions that stay explicit: whether CPU affinity
graduates, whether the documented evidence limitations are acceptable for release, and
authorization of the final tag.

## Rules

- Physical acceptance runs only from a clean committed tree whose binary revision
  matches source; evidence retains Cargo.lock digest and kernel/environment.
- Acceptance mutates only disposable, owned process trees. Restoration is proved
  by independent kernel readback, never by a rollback function's return value.
- Process identity is `(pid, starttime_ticks)`; a numeric PID alone is never identity.
- Privileged/destructive acceptance never runs for arbitrary public PR code.
- Tag `v0.1.0` only on the exact commit that passed fresh-checkout and real-system
  acceptance.
