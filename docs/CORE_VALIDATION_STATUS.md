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
- Initial full gate: format/build/Clippy passed; library tests reported 2,664
  passed, 35 failed, 5 ignored. Thirty-two failures exposed an unintended global
  quality downgrade when old runs lacked optional fixed-work markers. The
  quality rule now keeps missing compile evidence distinct from unrelated
  scheduler evidence. Remaining failures required the intentional API export,
  updated help snapshots, and extracting the cooperative probe descriptor to
  respect the existing catalog size limit. Historical fixtures were preserved.
- Added future-timestamp, oversized-line and stale-marker reader cases, plus
  cross-window PID and process-start-time mismatch rejection.
- PASS final local build gate: format, all-target build, Clippy `-D warnings`,
  all-target tests (2,699 library passed, 0 failed, 5 ignored; 8 lifecycle
  integration passed; other integration/default-member suites passed).
  `xtask` suite: 20 passed. Offline recommendation and advisor smokes passed;
  the historical schema-18 fixture rendered with its expected warning.
- PASS real command timing smoke at `a20ab25`: two package-clean `stutter-core`
  builds emitted valid ordered matching markers, 392,871,489 ns and
  391,818,357 ns. [Raw evidence](evidence/fixed-work-20261021/README.md) retains
  source, manifest, commands, markers and compiler output. No tuning or
  foreground monitor ran; this is not live objective acceptance or calibration.

## 2026-10-21: objective behavioral matrix

- Added a reusable matrix for all nine objective kinds, with an exhaustive
  objective-specific fixture match. It varies primary improvement/regression,
  unchanged values, missing signals, target disappearance, low experiment
  quality, zero samples, thermal/throttle/CPU/GPU regressions and generic-score
  conflict. Additional cases cover score normalization under 10x sample
  imbalance and independent I/O/IRQ worst-latency changes at constant counts.
- [Coverage and applicability](OBJECTIVE_CONTRACT_MATRIX.md) identifies dedicated
  runnable sufficiency/tail and fixed-work identity/foreground/memory cases.
  This records existing score fallbacks and guards; it does not claim new
  browser-specific or direct foreground measurement.
- Added the named `Objective behavioral contracts` CI step. Generated objective
  documentation and the authoritative descriptor registry remain pending.
- PASS local pinned environment: 43 objective tests, full library suite
  (2,705 passed, 0 failed, 5 ignored), format check, library/test Clippy with
  `-D warnings`, and `git diff --check`. Exact-head remote CI remains required.
- PR #40 at `242bc2d8fa4f1c8d6fa781e9313fb418c1c723d1` has run
  `39630897805` in progress at this checkpoint. No merge/main success is claimed.

## 2026-10-21: objective descriptors and documentation

- PR #40 passed complete exact-head CI run `39630897805` and merged as
  `14cf35966809111480831de7f6561c872086e8be`. Main run `39632255939`
  is in progress; main success is not yet established. PR #41 now targets main.
- Added exhaustive `ObjectiveKind::descriptor()` contracts and an enumerated
  objective registry, including primary metric/quality, required evidence,
  explicit fallback policy, guards, autonomous eligibility, maturity and limits.
  Eligibility does not enable automation or bypass separate action/safety policy.
- Registry contracts are exposed through the API, source-bound optional decision
  metadata and summaries, CLI candidate dry-run, selected-objective TUI output,
  and analyzer-labeled report metadata. Historical decisions remain readable
  without metadata; analyzer contracts do not invent historical measurements.
- `xtask objective-docs --write/--check` generates all objective documentation.
  Added CI documentation gate and tests that reject stale documents and invalid
  CLI mode combinations. Source enumeration is checked against declared variants.
- Initial descriptor library run: 2,706 passed, 0 failed, 5 ignored. Subsequent
  compatibility test passed but the new TUI summary exposed an outdated exact
  expectation (2,706 passed, 1 failed, 5 ignored); updated it to include contract
  fields. Report metadata integration then exposed an advisor fixture initializer
  missing the new field; updated that fixture without changing historical data.
- Extended Clippy coverage to xtask exposed four pre-existing helper lint errors;
  made equivalent closure/conditional simplifications, then the extended Clippy
  check passed. Final full validation is pending.
- PR #41 passed exact-head CI run `39631474272` at
  `69728cfbf93491a6d16181203d23d266c6ccb666` and merged as
  `8093474b6e99b8e49e67e1f6ea3289cce750b41d`. Issue #24 is closed.
  Main run `39632742284` was queued at observation; main validation remains open.
- Final full build gate PASS: format, build, all-target Clippy and tests;
  library 2,707 passed, 0 failed, 5 ignored; lifecycle integration 8 passed;
  other integration/default-member suites passed. xtask suite: 22 passed,
  including drift rejection and mutually exclusive CLI modes. Remaining
  xtask-only lint/doc/offline checks are in progress at this checkpoint.
- PASS final xtask-only Clippy `-D warnings`, generated objective-doc check,
  offline recommendation and historical-schema advisor smokes. The old schema-18
  artifact rendered with the expected version warning. No live-validation or
  capability-graduation claim is made by this descriptor/documentation slice.

## 2026-10-21: canonical live observation harness

- Main CI runs for PR #40 (`39632255939`, `14cf359`) and PR #41
  (`39632742284`, `8093474`) completed successfully. PR #42 remains in progress;
  P0 completion is not yet claimed.
- Added `xtask live-acceptance observation`, with independent observation-only
  config, bounded subprocesses and workload cleanup, deterministic periodic
  wakeups plus single-CPU contention, process identity checks, direct histogram
  population checks, zero-drop requirement, artifact validation and JSON/HTML/
  advisor pipeline. Missing prerequisites return nonzero BLOCKED, never a skip.
- PASS development-tree privileged functional attempt on Linux 6.18.55: 4,965
  target samples, 21 interval records, positive interval p99 and zero drop
  counters; validator, report and advisor completed. Local original evidence:
  `/tmp/stutter-live-root-attempt-20261021`. This generic Service workload does
  not establish game-objective acceptance; portable export and richer controlled
  objective evidence remain open. No mutation provider was exercised.
- The unprivileged attempt returned nonzero BLOCKED with a retained receipt.
  The preliminary privileged binary reported revision `69728cf` while HEAD was
  `8ff469d`; its digest was retained but that run is not clean-source acceptance.
  Strengthened canonical preflight to require a clean tree and matching binary
  Git revision. A new committed-source run remains required.
- Initial xtask test list had the two new command names in the wrong sort order;
  corrected the expectation. Subsequent validation: 23 xtask tests passed,
  xtask Clippy passed, and 121 architecture tests passed. Revalidation after the
  stronger provenance preflight is pending.
