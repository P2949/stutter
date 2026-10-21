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

1. PASS: GitHub [Core Validation milestone](https://github.com/P2949/stutter/milestone/1)
   and 17 actionable issues (#22–#38). The [backlog receipt](CORE_VALIDATION_BACKLOG.json)
   records their URLs. All remain open pending their individual acceptance gates.
2. Local implementation and offline checks PASS for direct runnable histogram
   aggregation, target/task-class semantics, comparator, sufficiency/quality
   handling, and regression-first tests. Remote integration and later live
   acceptance/calibration gates remain open.
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

## 2026-10-21: runnable-latency correctness

- Both semantic regression tests were executed against the unchanged comparator
  before its replacement and failed (exit 101): worse runnable p99 with a better
  generic score returned `Improved`; better runnable p99 with a flat score returned
  `Inconclusive`. Test names are preserved in `objective/tests/runnable.rs`.
- Implemented combined-window scheduler histograms, direct primary p99, sample
  sufficiency, exact severe-tail rate comparison, safety guardrails and no score
  fallback. Target scoping includes selected process descendants and excludes
  unrelated games; no identified target means unavailable evidence.
- The [contract](RUNNABLE_LATENCY_OBJECTIVE.md) specifies task classes, finite
  histogram resolution, provisional thresholds, and remaining live/calibration
  boundaries. Added artifact fields default to missing for historical reports.
- PASS: targeted `cargo test -p stutter --lib runnable` after implementation:
  21 tests, including the semantic regressions, aggregation, corruption/overflow,
  target scoping, guardrails, explanations and backward-compatible reads.
- No remote CI or live-acceptance claim is made by this entry. Intermediate
  failures and final local validation are recorded below.
- Initial broader library run: 2,680 passed, 2 architecture checks failed,
  5 ignored. Failures identified new unchecked unwraps and a panic path;
  replaced them with validated populations and a conservative bound. Added
  process-start-time matching and exited-root protection before revalidation.
- Subsequent library run passed: 2,682 passed, 0 failed, 5 ignored. The broader
  build gate then exposed a socket lifecycle fixture with no histograms and only
  125 scheduler samples, plus a controller transition still using generic score
  before the final objective check. It reported a keep even though the final
  objective check restored the action (6 integration tests passed, 1 failed).
- Controller active state now retains baseline signals and evaluates non-score
  objective contracts at the transition, including generic-score conflicts.
  Completed inconclusive non-score experiments restore their candidates. Added
  socket tests for both sufficient direct histograms and missing-histogram
  restoration, with stream/history dispositions checked against retained state.
  These are controlled fixture integrations through a real socket, not live
  scheduler acceptance evidence.
- A follow-up gate caught an old focus-change test implicitly configuring a fixed
  root while expecting it to follow focus (2,683 library tests passed, 1 failed,
  5 ignored). The follow-focus fixture now has no configured root; a separate
  runtime contract covers explicit-root precedence and non-descendant focus
  group members.
- PASS final local gate in the pinned Nix environment:
  `scripts/smoke/build.sh` (format, all-target build, all-target Clippy with
  `-D warnings`, all-target tests), `scripts/smoke/offline_recommendation.sh`,
  `scripts/smoke/advisor_offline.sh`, and `git diff --check`. Library tests:
  2,685 passed, 0 failed, 5 ignored; lifecycle integration tests: 8 passed.
  Other default-member and integration suites passed. The historical schema-18
  advisor fixture emitted its expected old-schema warning and rendered.

## 2026-10-21: remote integration and fixed-work compile progress

- PR [#39](https://github.com/P2949/stutter/pull/39) merged at
  `7fc8f3873a0aeaa53c7cab130e03c2fd69db0880`. Exact-head PR CI run
  `39597465475` and main run `39598456469` completed successfully.
  Issues #22 and #23 closed. The earlier all-open/no-remote statements above
  describe their historical checkpoints and are superseded by this entry.
  This is CI evidence, not live acceptance.
- Stage B counterexample reproduced before comparator replacement:
  `faster_identical_completed_compile_work_improves_despite_score_regression`
  returned Inconclusive for 31.4 s versus 28.0 s (exit 101).
- Implemented generic fixed-work identity/markers, clean-Git command runner,
  bounded cooperative log reader, reliable monitor events, optional recording
  stream, target-scoped aggregation and fixed-work comparison. See
  [FIXED_WORK_COMPILE.md](FIXED_WORK_COMPILE.md).
- PASS: targeted protocol/reader/runner/artifact suite (8 tests) and targeted
  fixed-work objective suite (8 tests). Initial xtask validation caught an
  outdated CLI command-list assertion (19 passed, 1 failed); added the new
  `fixed-work` command to its expected public command list.
- Broader validation is pending. No live compile performance or calibration
  claim follows from these offline tests.
