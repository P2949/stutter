# Core Validation execution ledger

The authoritative requested sequence is the updated [CORE_VALIDATION_PLAN.md](CORE_VALIDATION_PLAN.md).
Its repository inventory is the user's audit snapshot; this ledger records
subsequent implementation and verification against current state.
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
   records their creation URLs. P0 issues #22–#26 and #37 are closed.
2. PASS P0 at verified main `37ed5ce`: direct runnable histograms and comparator,
   fixed-work compile primary/activity safety, nine-objective contracts,
   exhaustive registry, generated documentation and visible objective CI.
3. PASS canonical observation success path on Linux 6.18.55, merged in PR #43
   with main CI success at `0be8ad5`. Controlled game-objective evidence and
   portable full-bundle export remain outstanding; #27 remains open.
4. PASS live affinity success path across three processes/five tasks. PR #44
   merged at `bbcead0`; exact-main CI run `39638305695` passed. #28 is closed.
   No provider graduation is implied.
5. IN PROGRESS: crash/startup recovery matrix, including identity/restore-format
   correctness fixes. Real controller SIGKILL/restart remains outstanding.
6. Portable evidence bundles; multiple real
   kernel environments and basic profiler-overhead measurements.
7. Provenance and validation coverage tooling; real diagnosis corpus, individual
   threshold/confidence calibration, and empirical score evaluation.
8. Capability tiers and provider graduation; release contract, reproducible
   packaging, installation tutorial, fresh-checkout release acceptance and tag.

Do not claim completion or graduate capabilities without the corresponding
evidence. Missing real environments and unperformed live tests remain explicit
validation debt rather than synthetic substitutes.

## 2026-10-21: revised recovery-first plan and provider authorization

- Adopted the user's updated repository-wide plan, preserving completed P0 and
  affinity issues. Recovery correctness precedes portable evidence, kernel and
  overhead validation, provenance/corpus, calibration, capability tiers and release.
- PR #45 exact-head CI `39639609726` passed at
  `a39f6c8313e036a96e98763a5de694bf3f87e45c`; clarified its CPU-affinity-only
  scope and merged as `7d97ec64a8b7e7854d4dfcdd1e31b2d3efdf8d35`.
  Exact-main CI `39672453590` is in progress; no main-success claim yet.
- Rebased the preserved controller transaction/crash-helper branch onto that
  main. Fixed the hook return types; the journal-at-verification regression
  passed. Production manager ownership and the during-apply gap remain open;
  that branch has not been represented as completed crash acceptance.
- Created #46 for explicit automatic recovery capability per token. Before the
  gate, a real temporary-file sysfs regression failed (exit 101): the startup
  executor accepted an unqualified arbitrary-path token. Red evidence:
  `/tmp/stutter-recovery-capability-red.log`. No host sysfs or user state touched.
- Added exhaustive per-token capability decisions and a strict affinity-only
  startup registry, with no manual catch-all. Unauthorized providers are rejected
  before dispatch and retain fault/manual recovery evidence. Explicit manual
  registry behavior stays separate. Full new validation is pending below.
- Created and read back active main ruleset `25590967`: force pushes/deletion
  blocked, PR required with zero mandatory reviews, and strict up-to-date GitHub
  Actions checks for `fmt build clippy test`, `daemon safety gates`, and
  `schema and fixture gates`. The build job includes objective contracts/docs.
  No bypass actors or privileged public-PR live gate were configured.
- The expanded targeted suite exposed empty manual-operation tokens producing
  an empty recovery hint (26 passed, 1 failed). Added an explicit retained-journal
  inspection instruction for that malformed/empty-token case before rerunning.
- Confirmed all seven superseded Core Validation/recovery branch tips are
  ancestors of `7d97ec6`, then deleted their remote/local heads. Also removed
  the merged local historical project-cleanup branch and advanced local main by
  fast-forward. Remote inventory now has protected main plus the preserved active
  controller branch; this new capability branch will be published for review.
- Created #47 (durable rollback before verification) and #48 (prepared rollback
  before first mutation), preserving the distinct gaps rather than claiming the
  existing after-apply hook covers mid-apply death. Portable export stays tracked
  under #27/#34. Added an explicit startup-recovery contract step to daemon CI.
- PASS full pinned-environment format/build/all-target Clippy/test gate: 2,717
  library tests, 5 ignored, 8 lifecycle integration tests and other suites.
  PASS 23 xtask unit tests plus descendant-cleanup integration, xtask Clippy,
  generated objective docs, offline recommendation, and schema-18 advisor smoke
  with its expected older-schema warning. Final expanded startup suite passed
  all 27 tests, including all-ten-token registry coverage and all-nine-denied-
  token fault/audit/history/daemon-state retention. Autotune safety script passed.
  This is provider authorization/identity validation, not full real crash-matrix
  acceptance; #29, #47 and #48 remain open.
- PASS exact-main PR #45 CI run `39672453590` at
  `7d97ec64a8b7e7854d4dfcdd1e31b2d3efdf8d35`: build, daemon safety and
  schema/fixture gates all succeeded; optional manual live-smoke was skipped.

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
- PR #42 passed complete exact-head CI run `39632958408` at `8ff469d`
  and merged as `37ed5ce82e95f6335acbff2b25a7e07efe8f0169`; issues #25
  and #26 closed. Main run `39634172998` is still in progress. Issue #37
  closed after verifying its implementation/contracts and PR #40/main CI evidence.
- Stronger harness validation PASS: 23 xtask tests, xtask Clippy `-D warnings`,
  format and diff checks. Committed harness as `1e0760d`, rebuilt stutter and
  xtask, and ran ten-second privileged acceptance from a clean tree. PASS:
  binary version matched `1e0760d`; 9,946 target scheduler samples across 42
  interval records; stable recorded process identity, complete histogram
  populations, zero drop counters and successful validator/analysis/HTML/advisor.
  [Sanitized summary](evidence/live-observation-20261021/README.md) retains selected
  provenance and result. Original full local bundle remains at
  `/tmp/stutter-live-clean-source-20261021`.
- A subsequent negative attempt was BLOCKED by the clean-tree gate after evidence
  files were added; it is not additional permission-check evidence. The earlier
  unprivileged doctor refusal remains the recorded permission-path test.
- Issue #27 remains open pending remote integration and the remaining acceptance
  evidence requirements. Game-objective controlled capture, portable full export,
  independent affinity/recovery suites, overhead and multiple kernels remain open.

## 2026-10-21: P0 integration and live affinity progress

- P0 decision-model integration gate PASS at current remote main
  `37ed5ce82e95f6335acbff2b25a7e07efe8f0169`: exact-main run
  `39634172998` completed successfully. Direct histogram semantics, activity
  safety/fixed-work primary, nine-objective behavioral contracts, exhaustive
  descriptors, generated documentation and visible objective CI are merged.
  This does not complete the Core Validation milestone or lift its feature freeze.
- PR #43 remains in progress. Added independent `live-acceptance affinity`
  suite using the existing CPU-affinity action and audited runner with an
  explicit new restore path. Existing callers retain their default paths.
  The suite snapshots task identities and original masks, validates dry-run,
  durable rollback records and applied kernel masks, rolls back, independently
  rechecks all original masks and checks the audit phases. Post-apply validation
  errors still attempt rollback through the known isolated restore token.
- Added refusal tests for stale root identity and pre-existing restore state.
  The first compile caught tuple iteration over the task snapshot; fixed it to
  iterate task values. Intermediate full library run passed (2,707 passed,
  5 ignored). Final checks and real affinity acceptance remain pending.
- Final local validation PASS: full format/build/all-target Clippy/test gate
  (2,709 library passed, 5 ignored; 8 lifecycle integration passed; other suites
  passed), 23 xtask tests and extended stutter/xtask all-target Clippy. Committing
  and rebuilding before real affinity acceptance is required by provenance policy.
- PR #43 passed exact-head CI run `39634530958` and merged as
  `0be8ad5eb3c603a84689576aafc70263a2d5c001`. Main run `39635725995`
  is in progress. Observation evidence/export follow-ups remain open.
- First real affinity smoke at clean `3d5b2c9` passed: three tasks in one process
  changed from mask `0-11` to online CPU `0` and independently restored `0-11`,
  with ten audit records. This was a thread-scope smoke, not complete descendant-
  process acceptance. Original local evidence: `/tmp/stutter-affinity-clean-20261021`.
- Expanded the helper to parent plus two separate child processes (five tasks),
  with explicit child readiness and parent-owned pipes that deliver EOF on parent
  death. The action suite now requires three processes, checks rollback process
  IDs, and preserves a create-new pre-rollback journal copy. Added an independent
  process-cleanup integration test. Complete multi-process real acceptance is
  still pending the committed-source rerun.
- Expanded full local gate passed again; xtask unit suite 23 passed and Clippy
  passed. Final new descendant-cleanup integration test/lint check is pending.
- PASS descendant-cleanup integration: after SIGKILL of the disposable parent,
  both child process identities ceased running within the bounded check. Final
  xtask all-target Clippy and format/diff checks passed. This is helper cleanup,
  not controller crash-after-apply recovery acceptance.
- PASS clean-source multi-process affinity acceptance at `9130ea1`: parent and
  two children, five tasks; each original mask `0-11`, applied mask `0`, and
  independently restored mask `0-11`. Dry-run unchanged, durable original/task/
  process identities checked, pre-rollback journal archived and ten audit events
  verified. Ran as the normal user, independently of BPF/observation support.
  [Retained evidence](evidence/live-affinity-20261021/README.md) includes the
  receipt, selected environment, journal and sanitized audit. Full original
  bundle: `/tmp/stutter-affinity-process-tree-20261021`.
- PR #43 main CI run `39635725995` completed successfully at exact merge
  `0be8ad5eb3c603a84689576aafc70263a2d5c001`. Affinity remote integration,
  controller crash/restart recovery, fault matrix and graduation remain open.

## 2026-10-21: automatic recovery identity and format correction

- Before crash acceptance, two real-child regressions exposed unsafe/incorrect
  startup paths. Both failed against the unchanged implementation (exit 101):
  unverified legacy identity was restored by numeric TID and reported Recovered;
  an identified historical affinity file was parsed as an empty profile file,
  discarded, and reported Recovered with zero restored tasks. Test resources
  were owned disposable children and isolated files; no user workload/state used.
- Initial regression compilation caught a private CpuMask helper reference;
  changed the test to use the public range representation before reproducing
  the actual semantic failures. Red-run log: `/tmp/stutter-recovery-before-v2.log`.
- Added supported-version/layout validation for restore loaders, preserving
  historical affinity routing. Automatic startup recovery now validates complete
  nonzero task/process identity across the entire affinity/priority record batch
  and enough original identities for the token before any mutation. Invalid
  state faults recovery and remains retained; explicit manual legacy behavior
  stays separate. Fully identified stale records remain safe skips.
- PASS first post-fix targeted suite (3 tests) and full library run (2,711 passed,
  5 ignored). Added batch and simulated replacement-start-time real-child tests;
  final full gate is in progress. These are real kernel restore checks, not a
  controller crash/restart acceptance claim. Issue #29 remains open.
- Refined automatic recovery to use a strict rollback registry whose validator
  consumes the exact parsed state used by the restore operation, closing the
  separate-preflight/reload gap. Manual registry behavior remains explicit and
  separate. The first production build caught a test-only type re-export;
  made the internal restore-state type available to the production validator.
- PASS refined full format/build/all-target Clippy/test gate (2,714 library
  passed, 5 ignored; 8 lifecycle integration passed; other suites passed), xtask
  unit/descendant-cleanup integration, and explicit manual compatibility across
  profile schema 1 and affinity schemas 1–3. Final offline smokes are pending.
- PR #44 passed full exact-head CI run `39637052613` and merged as
  `bbcead07661aaa68cf96aa881c7d6c5198022f32`. Main run `39638305695`
  remains in progress at this checkpoint; #28 remains open pending integration.
- PASS final offline recommendation and historical-schema advisor smokes; the
  schema-18 artifact rendered with its expected warning. The two original red
  semantic tests ran against production base `ced5f1dc454c06d59f416ccb279c4271ddb89733`
  before the loader/recovery corrections, using only added test fixtures.
- PR #44 main CI run `39638305695` completed successfully at exact current main
  `bbcead07661aaa68cf96aa881c7d6c5198022f32`; #28 closed after reviewing its
  multi-process, identity, durable-token, kernel-readback and audit criteria.
  Recovery #29 and the remaining milestone gates stay open.
