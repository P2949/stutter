# Core Validation execution ledger

The authoritative requested sequence is [CORE_VALIDATION_PLAN.md](CORE_VALIDATION_PLAN.md).
The milestone remains **IN PROGRESS**. No live acceptance, calibration, kernel
compatibility, or release gate is implied by offline tests.

## 2026-10-21: scope and first safety correction

- Scope freeze recorded in the roadmap; implementation branch:
  `fix/autotune-objective-semantics`, starting from `8a61a33`.
- Regression reproduced before the fix: the 20-versus-30 compile-active-interval
  counterexample failed with exit 101, demonstrating an unsupported throughput
  verdict even when the candidate's generic score improved.
- Compile comparison now remains inconclusive without direct completed-work
  evidence. Foreground, thermal, and power safety checks remain active.
- Runtime activity counters renamed `compile_activity_*`; serde aliases retain
  reads of historical `compile_progress_*` fields. Historical `Direct` labels
  cannot authorize a throughput verdict. Reports explain missing throughput.
- PASS: full library validation with `nix develop --command cargo test -p stutter
  --lib`: 2,665 passed, 0 failed, 5 ignored. This includes the activity matrix,
  compatibility regression, and foreground guardrail tests.
- PASS: `cargo fmt --all --check`, `git diff --check`, and
  `cargo clippy -p stutter --lib -- -D warnings` in the pinned Nix environment.

## Outstanding gates, in execution order

1. GitHub Core Validation milestone and actionable P0/P1/P2/P3 backlog.
2. Direct runnable histogram aggregation, target/task-class semantics, comparator,
   sufficiency/quality handling, and regression-first behavioral tests.
3. Fixed-work compile completion protocol and foreground protection contracts.
4. Exhaustive objective registry, full behavioral matrix, generated documentation,
   dedicated objective CI, and full CI at the committed remote head.
5. Canonical live observation and disposable-process affinity acceptance.
6. Crash/startup recovery matrix and portable evidence bundles; multiple real
   kernel environments and basic profiler-overhead measurements.
7. Provenance and validation coverage tooling; real diagnosis corpus, individual
   threshold/confidence calibration, and empirical score evaluation.
8. Capability tiers and provider graduation; release contract, reproducible
   packaging, installation tutorial, fresh-checkout release acceptance and tag.

Do not claim completion or graduate capabilities without the corresponding
evidence. Missing real environments and unperformed live tests remain explicit
validation debt rather than synthetic substitutes.
