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
and before the first affinity mutation; failed-apply fault reporting.

Transaction under test:

```text
durable restore snapshot → Applying(token) → first mutable syscall
  → Applied(token) → verify → Measuring / later lifecycle
```

Do not redesign it unless a real acceptance test exposes a defect.

## Remaining sequence

| Step | Work | Issues |
| --- | --- | --- |
| PR 53 | Canonical controller SIGKILL/restart acceptance: suite identity, `(pid, starttime)` restart identity, six transaction checkpoints, one `LiveExperimentManager` `Measuring` checkpoint | close #47, advance #48/#29 |
| PR 54 | Interrupted rollback, partial-apply, verify-failure, rollback-failure, stale/replacement identity | close #48 |
| PR 55 | Opt-in suspend/resume acceptance (`--allow-system-suspend`) | close #29 |
| PR 56 | Portable evidence bundle, fail-closed privacy export, `evidence-verify` | advance #27/#34 |
| PR 57 | Controlled real `GameRunnableLatency` acceptance | close #27 |
| next | Real-kernel compatibility matrix | #30 |
| next | Profiler overhead characterization | #38 |
| next | Provenance/coverage extension, diagnosis corpus, per-family calibration, `StutterScore` evaluation | #34, #31, #32, #33 |
| next | Capability tiers and provider graduation | #35 |
| final | Release contract, package, fresh-checkout acceptance, tag `v0.1.0` | #36 |

Issue order: recovery #47 → #48 → #29; evidence #27 → #34 (foundation) → #30 → #38;
validation #34 → #31 → #32 → #33; product #35 → #36.

## Rules

- Physical acceptance runs only from a clean committed tree whose binary revision
  matches source; evidence retains Cargo.lock digest and kernel/environment.
- Acceptance mutates only disposable, owned process trees. Restoration is proved
  by independent kernel readback, never by a rollback function's return value.
- Process identity is `(pid, starttime_ticks)`; a numeric PID alone is never identity.
- Privileged/destructive acceptance never runs for arbitrary public PR code.
- Tag `v0.1.0` only on the exact commit that passed fresh-checkout and real-system
  acceptance.
