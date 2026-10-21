I re-audited the repository from the current GitHub state rather than carrying the old plan forward unchanged. The biggest change is that the earlier P0 objective-correctness phase is now essentially complete; the project has moved into **recovery correctness, real-system evidence, calibration, and release-boundary work**. The audit also exposed a few new issues that were not visible in the earlier snapshot—especially around crash durability, startup recovery scope, live-evidence export, and branch/repository hygiene.

# Stutter — Updated Repository-Wide Core Validation Plan

## 1. Current conclusion

The correct development sequence has changed because a substantial amount of the previous plan has already been implemented.

The repository currently has:

- 9 branches;
- 25 issues total: 10 open and 15 closed;
- 20 pull requests total: 1 open and 19 closed/merged;
- a completed P0 objective-semantics phase;
- a real clean-source live scheduler observation acceptance path;
- a real multi-process CPU-affinity apply/restore acceptance path;
- an open startup-recovery correctness PR;
- an additional unmerged branch containing crash-acceptance and rollback-before-verification work.

The current branch inventory confirms nine branches, including `main`, six historical Core Validation implementation branches that have already landed, the open `fix/startup-recovery-identity` branch, and the still-unmerged `fix/controller-rollback-before-verify` branch.

The project should therefore **not restart the old plan from objective correctness**.

The new sequence should be:

> **finish recovery correctness → prove crash safety → finish live evidence portability → kernel/overhead validation → build real validation corpus → calibrate diagnosis/score → define supported boundary → release**

The feature freeze should remain in place until this sequence is substantially complete.

---

# 2. Repository state after the new audit

## 2.1 P0 is complete

The previous highest-priority problems have now been addressed.

The execution ledger records that:

- compiler activity was renamed/demoted and can no longer independently claim throughput improvement;
- direct combined-window runnable-latency histograms were implemented;
- `GameRunnableLatency` now has a direct primary p99 measurement rather than generic-score fallback;
- fixed-work compile completion was implemented;
- all nine objectives received behavioral contract coverage;
- objective descriptors became authoritative and machine-readable;
- objective documentation is generated/validated;
- dedicated objective CI exists;
- exact-main P0 CI passed.

Those areas should remain regression-protected, but **they are no longer active development priorities**.

Do not reopen:

- #22;
- #23;
- #24;
- #25;
- #26;
- #37.

---

# 3. Evaluation of all nine branches

| Branch | Current meaning | Action |
|---|---|---|
| `main` | Current authoritative integrated state at `bbcead0...` during this audit | Keep; protect it |
| `fix/autotune-objective-semantics` | P0 work already integrated | Delete remote branch after current safety work is rebased |
| `feat/fixed-work-compile-throughput` | Fixed-work work already integrated; branch is behind `main` with no unique commits | Delete |
| `test/objective-contract-matrix` | Behavioral matrix already integrated | Delete |
| `feat/objective-descriptors` | Descriptor/docs work already integrated | Delete |
| `feat/live-observation-acceptance` | Live observation implementation already integrated; no unique work remains | Delete |
| `feat/live-affinity-acceptance` | Live affinity implementation already integrated; no unique work remains | Delete |
| `fix/startup-recovery-identity` | **Active PR #45**, important correctness work | Keep, finish and merge first |
| `fix/controller-rollback-before-verify` | **Important unmerged follow-up work** based on PR #45 | Keep, rebase after #45, split/rework into next safety PR(s) |

The fixed-work branch, for example, is already entirely behind current `main` with zero commits ahead, and the same is true of the merged live-acceptance branch tips.

### Branch-cleanup rule

Do **not** delete the two active recovery branches yet.

After PR #45 is merged and `fix/controller-rollback-before-verify` is rebased onto the resulting `main`, delete the six already-merged implementation branches.

That should leave approximately:

```text
main
fix/controller-rollback-before-verify
```

plus whatever new branch is actively under development.

After subsequent merges, prefer deleting merged feature branches promptly rather than allowing the repository to accumulate historical branch heads.

---

# 4. Add protection to `main`

The branch information currently reports `main` as unprotected.

For a project whose next work modifies crash recovery and privileged mutation semantics, that is unnecessary risk.

Create a GitHub ruleset for `main`.

Recommended minimum:

```text
block force pushes
block branch deletion
require pull request before merge
require normal CI
require daemon-safety
require schema/fixture gates
require objective-contract/docs gates
```

Do **not** make privileged live-system testing a normal public-PR requirement; it belongs to trusted release/main validation.

For a solo-maintained experimental project, there is no need to impose cumbersome review-count requirements on yourself. The important pieces are preventing accidental force pushes and ensuring ordinary CI cannot be bypassed accidentally.

---

# 5. Evaluation of the 15 closed issues

All 25 repository issues are visible in the current issue history.

## #10 — Supervisor pitch PDF visual check

**Status:** correctly closed.

This belonged to the old academic/supervisor workflow.

PR #21 subsequently removed that project identity.

### Action

None.

Do not recreate this work in the current roadmap.

---

## #11 — Long-term raw artifact storage policy

**Status:** correctly closed for its original purpose.

The historical KCD1 artifact policy was resolved by moving large evidence outside the normal source tree/release workflow.

### New implication

The project now has a **different** evidence-storage problem: live acceptance bundles.

Do not reopen #11.

Handle the new requirement under #27/#34 or a dedicated new issue:

```text
P1: Make live acceptance evidence portable and privacy-verifiable
```

---

## #12 — Confirm CI for supervisor handoff

**Status:** correctly closed and obsolete.

No further work.

---

## #13 — Explicit counterbalanced tune order

**Status:** correctly closed.

The issue required alternating/seeded order, artifact recording and testing. The implementation history records alternating, fixed and `seed:N` support together with persisted `candidate_order` and comparability checks.

### Action

Keep regression coverage.

Do not reopen.

Later calibration should validate whether the experiment methodology behaves well empirically, but that belongs to P2.

---

## #14 — Order-balance warnings

**Status:** correctly closed.

The implementation records `candidate-order-not-counterbalanced` and propagates the warning into JSON/Markdown/HTML recommendation output.

### Action

No new implementation required.

---

## #15 — eBPF linker warning noise

**Status:** correctly closed.

The wrapper exists and verbosity can be restored explicitly through `STUTTER_EBPF_VERBOSE=1`.

### Action

None.

---

## #16 — Offline supervisor demo

**Status:** correctly closed/historical.

Its modern successor is not another supervisor demo.

The equivalent product work belongs to #36:

```text
minimal first-release tutorial
```

---

## #17 — Automated sanitized evidence export

**Status:** historically completed, but its implementation should not be mistaken for the new live-evidence requirement.

The existing exporter:

- copies `evidence-bundle/`;
- excludes `**/runs/**`;
- generates `MANIFEST.sha256`;
- runs a basic regex privacy scan.

There is one concrete weakness worth fixing:

```bash
rg ... || echo "No obvious local paths found"
```

means discovering a suspicious path prints it but does **not fail the exporter**.

### Action

Keep #17 closed, because its original scope was fulfilled.

Create the stronger live-evidence mechanism described later in this plan.

---

## #22 — Direct `GameRunnableLatency`

**Status:** complete.

Do not revisit except through calibration/regression testing.

---

## #23 — Compile activity versus throughput

**Status:** complete.

Do not reopen.

---

## #24 — Behavioral contracts for all objectives

**Status:** complete.

Do not reopen.

---

## #25 — Machine-readable objective descriptors

**Status:** complete.

Do not reopen.

---

## #26 — Generated/validated objective documentation

**Status:** complete.

Do not reopen.

---

## #28 — Live reversible CPU-affinity acceptance

**Status:** complete.

The current ledger records a clean-source multi-process test involving:

```text
parent + two children
five tasks
original mask 0-11
applied mask 0
independent restore to 0-11
```

with retained rollback state and audit evidence.

Do not reopen #28.

Crash/restart recovery is deliberately separate under #29.

---

## #37 — Fixed-work compile completion

**Status:** complete as a functionality issue.

The real command-timing smoke exists, while full real-world performance calibration remains explicitly unclaimed.

### Action

Do not reopen #37.

Live compile-objective validation belongs to P2 corpus/calibration work.

---

# 6. Evaluation of the ten open issues

## #27 — Live observation acceptance

### Status

**Mostly implemented, but not ready to close.**

The canonical:

```text
xtask live-acceptance observation
```

exists.

A clean-source Linux 6.18.55 run produced:

- 9,946 target scheduler samples;
- 42 target interval records;
- stable process identity;
- complete scheduler histograms;
- zero drops;
- successful validator;
- successful analysis;
- HTML;
- advisor output.

The acceptance validator itself explicitly records:

```text
game_objective = "not_claimed_for_generic_acceptance_workload"
```

because the deterministic workload is currently generic rather than a controlled game-objective target.

### Remaining work

#27 should remain open until:

1. controlled `GameRunnableLatency` evidence exists;
2. source/binary/kernel/probe evidence is exported portably;
3. the final bundle is reproducibly sanitizable;
4. the canonical suite—not merely legacy smoke scripts—is integrated into the trusted live-validation workflow.

---

## #29 — Crash/startup recovery matrix

### Status

**Highest priority remaining issue.**

Its definition of done correctly requires:

- death before/during apply;
- partial apply;
- forced verification failure;
- SIGKILL after apply;
- SIGKILL during verification;
- restart recovery;
- stale PID reuse;
- suspend/resume;
- rollback failure;
- explicit fault state.

PR #45 only addresses part of this.

This issue should remain open until the full real-system matrix is complete.

---

## #30 — Real-kernel compatibility corpus

### Status

Not yet complete.

Do after recovery and portable evidence tooling are stable.

---

## #31 — Diagnosis calibration corpus

### Status

Not yet started at sufficient real-world depth.

Requires the provenance system from #34 first.

---

## #32 — Calibrate diagnosis thresholds

### Status

Not ready yet.

Needs #31 evidence.

---

## #33 — Calibrate `StutterScore`

### Status

Not ready yet.

Needs the same corpus.

---

## #34 — Evidence provenance and real-evidence coverage

### Status

Open and strategically important.

I would move part of this work **earlier than #31**, because the corpus should not be collected before its provenance format exists.

---

## #35 — Supported/experimental/internal capability tiers

### Status

Correctly deferred.

Do after P1/P2.

---

## #36 — First versioned software release

### Status

Correctly deferred.

Do not start release polishing while #29 remains open.

---

## #38 — Profiler overhead

### Status

Open and necessary.

Do after the live harness is stable, but before capability graduation.

---

# 7. Evaluation of all 20 PRs

The repository has 20 PRs in its history, with #45 the only currently open one.

## PR #1

Initial automated-development import/foundation.

### Assessment

Historical foundation only.

No action.

---

## PR #2

Large daemon/packaging/platform expansion.

### Assessment

Its architecture now feeds the current daemon and recovery work.

No wholesale revisit.

Recovery testing should validate the resulting behavior instead.

---

## PR #3

Experimental autotuning expansion.

### Assessment

Conceptually superseded by later objective/safety hardening.

No direct action.

---

## PR #4 / #5

Lint/no-allow/local-CI hygiene.

### Assessment

Useful infrastructure, still serving the project.

No action.

---

## PR #6 / #7

Autotune comparison/scoring architecture cleanup.

### Assessment

Still relevant architecture.

The recent objective work built on top of it.

No reopening.

---

## PR #8 / #9

eBPF/tracepoint edge-case work and follow-up corrections.

### Assessment

Treat the pair as the historical stabilization of that implementation.

Keep regression coverage.

Kernel-matrix work in #30 will now test those assumptions against more actual kernels.

---

## PR #18

Large supervisor/FYP handoff.

### Assessment

Historical.

Its academic/product wrapper was deliberately superseded later.

---

## PR #19

Dependency/linker/build hygiene.

### Assessment

Complete.

---

## PR #20

Reproducible Nix development environment and physical loader testing.

### Assessment

Important current foundation.

Keep.

---

## PR #21

Repository conversion back to a general experimental open-source project.

### Assessment

Complete.

Do not continue spending time on repository-identity cleanup.

---

## PR #39

Corrected game runnable-latency and compile activity semantics.

### Assessment

P0 cornerstone.

Complete.

---

## PR #40

Implemented fixed-work completion-based compile throughput.

### Assessment

Complete functionality.

Needs later empirical calibration, not redesign.

---

## PR #41

Implemented behavioral contracts for all objectives.

### Assessment

Complete.

---

## PR #42

Implemented machine-readable objective descriptors and checked/generated documentation.

### Assessment

Complete.

---

## PR #43

Canonical live scheduler observation.

### Assessment

Merged and useful, but it only partially completes #27.

The generic service workload proves the measurement pipeline, not the direct game objective.

---

## PR #44

Real multi-process CPU-affinity apply/restore.

### Assessment

Merged and sufficient to close #28.

Does **not** prove crash recovery.

---

## PR #45 — OPEN

### Title

`Require verified identity and correct format for startup affinity recovery`

### Assessment

This is the immediate PR to finish.

It fixes two real correctness failures:

1. automatic startup affinity recovery could use legacy numeric TIDs without verified process identity;
2. a historical affinity `records` file could be misinterpreted as an empty profile restore file and falsely report successful recovery.

The PR adds format/version routing, complete task/process identity validation, stale-identity handling and keeps explicit manual legacy restoration separate. Its own description correctly states that SIGKILL/restart recovery, failure injection and suspend/resume remain outstanding.

### Important scope correction

The implementation's special strict startup identity mode currently applies to the CPU-affinity handler.

The startup rollback registry still registers:

```text
nice
ioprio
uclamp
cgroup
IRQ affinity
CPU power
GPU power
VM knobs
```

in addition to affinity.

Task-targeted handlers such as `nice`, `ioprio` and `uclamp` still contain a legacy-unverified restoration path. `NiceRollbackHandler`, for example, explicitly accepts `UnknownLegacy` and continues restoration.  The equivalent pattern exists for I/O priority and uclamp.

That does not invalidate PR #45's CPU-affinity fix, but it means **“strict startup recovery” must not be described as generic across every rollback provider yet.**

---

# 8. Newly discovered high-priority issue: controller rollback durability window

This is the most important new finding from reviewing the active branches.

Current `main` performs approximately:

```text
write journal = Applying
        ↓
executor.apply_candidate()
        ↓
action runner:
    apply
    verify
        ↓
returns rollback token
        ↓
write journal = Applied(token)
```

The current live experiment lifecycle writes `Applying`, calls the executor, and only writes `Applied` with the rollback token after the executor returns.

The low-risk executor invokes the audited action path, which includes verification before it returns.

Therefore a process crash can occur:

```text
system mutation completed
        ↓
rollback token exists in process memory
        ↓
verification starts
        ↓
SIGKILL
        ↓
controller journal still says Applying with no durable token
```

Startup recovery intentionally treats `Applying` without a token as a fault because it cannot safely infer the rollback operation.

That is a real crash-recovery gap.

---

# 9. `fix/controller-rollback-before-verify` is valuable, but currently incomplete

The branch introduces a `JournaledActionInput`/`apply_journaled_action` transaction.

Its after-apply hook writes:

```text
Applied
+ rollback token
+ applied_pending_verify
```

before the action runner enters verification.

The runner is already structured appropriately: immediately after successful `apply()`, it executes the after-apply hook; if that hook fails, the runner attempts rollback before it proceeds to verification.

The journal write itself is genuinely durable rather than merely buffered:

```text
temporary file
→ serialize
→ file.sync_all()
→ atomic rename
→ parent-directory sync
```



That is a good foundation.

However, the branch's production `live_experiment/lifecycle.rs` still contains the old outer:

```text
write Applying
→ executor.apply_candidate()
→ write Applied(token)
```

sequence.

So the new transaction primitive is not yet the single production path used by the live controller.

### Conclusion

Do **not** merge that branch as though the controller crash hole is completely fixed.

Rebase it after #45 and finish the production integration first.

---

# 10. Another crash window: death during `apply()`

Even persisting the token before verification does not completely satisfy #29.

`CpuAffinityProfileAction::apply()` currently delegates to profile application, receives the result, and only then constructs its rollback token.

The underlying profile application does something useful:

```text
plan
→ preflight
→ persist restore state
→ perform actual affinity/priority mutations
→ verify
```

The restore state is written before the first mutation.

That means the data required for recovery may physically exist during a mid-apply crash, but the controller journal does not yet contain the token that points to it.

This must be addressed if #29 is going to truthfully claim crash safety during apply.

---

# 11. Revised immediate development sequence

# Phase A — Finish PR #45

## Step A1 — Rebase/update PR #45 against current `main`

Ensure its one logical change remains easy to review.

Run:

```text
full fmt/build/clippy/test
objective contracts
objective docs check
daemon safety
schema snapshots
offline recommendation
offline advisor
```

The PR's head already has successful validation according to its current state, but merge should still be against current `main`.

---

## Step A2 — Clarify its scope

Documentation should say:

> automatic **CPU-affinity** startup recovery now requires verified identity.

Do not say:

> all startup rollback providers are now identity-safe.

unless that is actually implemented before merge.

---

## Step A3 — Add one startup-recovery registry safety contract

Before or immediately after #45, add a test enumerating all token kinds registered for automatic startup recovery.

Each must declare one of:

```text
VerifiedAutomatic
ManualOnly
NotRecoverable
```

Do not allow a provider silently to inherit automatic startup recovery merely because its handler happens to be registered.

---

## Step A4 — Merge PR #45

After:

```text
exact-head CI PASS
mergeable
full local gate PASS
scope wording correct
```

merge it.

Then require main CI to pass.

---

# Phase B — Restrict startup recovery to providers that actually have crash-safe identity semantics

This should happen before #29 closes.

Create a new issue, approximately:

```text
P1: Require explicit crash-recovery capability for every rollback token
```

## Step B1 — Add a startup-recovery capability descriptor

Conceptually:

```rust
enum AutomaticRecoveryCapability {
    VerifiedAutomatic,
    ManualOnly,
    Unsupported,
}

struct RollbackRecoveryDescriptor {
    token_kind: &'static str,
    automatic_recovery: AutomaticRecoveryCapability,
    identity_contract: &'static str,
    idempotent: bool,
}
```

---

## Step B2 — Start with a conservative whitelist

Initially:

```text
CPU affinity:
    VerifiedAutomatic

nice:
    ManualOnly

ioprio:
    ManualOnly

uclamp:
    ManualOnly

cgroup:
    ManualOnly until proven

IRQ:
    ManualOnly until proven

CPU power:
    ManualOnly until proven

GPU power:
    ManualOnly until proven

VM:
    ManualOnly until proven
```

This matches the current product boundary: CPU affinity is the only low-risk provider with real acceptance evidence.

It prevents the generic rollback registry from accidentally granting crash-recovery maturity to experimental providers.

---

## Step B3 — Provider graduation later

When a provider gains:

```text
strict identity/resource validation
idempotent rollback
fault injection
real apply/verify/rollback
crash/restart acceptance
```

promote its recovery capability.

This should become part of #35's provider graduation criteria.

---

# Phase C — Rebase the unmerged crash branch

After #45:

```bash
git checkout fix/controller-rollback-before-verify
git fetch origin
git rebase origin/main
```

Resolve it against the merged startup-recovery implementation.

Do not retain duplicate startup fixes from the old branch history.

---

# Phase D — Split the crash branch into two PRs

This will make review much safer.

## PR A — Make rollback token durable before verification

Suggested branch:

```text
fix/controller-durable-rollback-before-verify
```

### Step D1 — Use one journaled action transaction in production

The live low-risk controller should use the same transaction that crash acceptance uses.

Required sequence:

```text
Preflighted
    ↓
Applying
    ↓
action.apply()
    ↓
rollback token exists
    ↓
durably write Applied(token)
    ↓
verify
    ↓
Verifying/Measuring
```

There must no longer be one transaction protocol for crash tests and another for actual autotune.

---

## Step D2 — Eliminate the redundant post-executor journal write

Once the action runner's after-apply hook owns rollback-token persistence, the surrounding live manager should not independently rewrite that same phase after the entire executor returns.

One component should own the state transition.

---

## Step D3 — Test the exact ordering

Use a test action whose `verify()` reads the journal.

At the instant `verify()` begins it must prove:

```text
state == Applied or Verifying
rollback_token != None
token restore state exists
journal durable write completed
```

The current branch already begins doing this; keep the test permanently.

---

## Step D4 — Test after-apply journal failure

Fault-inject:

```text
apply succeeds
journal Applied write fails
```

Expected:

```text
runner immediately rolls back
rollback succeeds
no verification
controller enters safe fault/error state
audit explains the journal failure
```

---

## Step D5 — Test audit/journal consistency

For every terminal path ensure:

```text
journal
audit
history
daemon state
```

agree about whether mutation occurred and whether rollback succeeded.

---

# Phase E — Make CPU-affinity apply crash recoverable during the mutation itself

This is required for the stronger interpretation of #29's “during apply” criterion.

The clean design is a two-stage action transaction.

## Step E1 — Separate rollback preparation from mutation

Conceptually:

```text
prepare()
    snapshot original state
    write restore state
    return PreparedRollbackToken

journal Prepared(token)

commit()
    perform actual mutations

verify()
```

This makes the most important invariant:

> **No first system mutation occurs until enough rollback state is durable to recover after SIGKILL.**

---

## Step E2 — Give autotune experiments unique restore paths

Current normal CPU-affinity candidate construction uses `restore_path: None`, which falls back to a shared default restore location.

For live autonomous experiments, prefer an experiment-specific path:

```text
~/.local/state/stutter/autotune/experiments/<experiment-id>/restore.json
```

Benefits:

- no ambiguity with previous state;
- no merging unrelated rollback state;
- token can reference the exact experiment;
- evidence can retain the exact transaction;
- cleanup becomes deterministic.

Manual profile application can retain its existing compatibility behavior.

---

## Step E3 — Add an explicit `Prepared` controller state if useful

A clearer state machine would be:

```text
Planned
Preflighted
Prepared          ← restore state durable; no mutation yet
Applying          ← mutation can now happen
Applied           ← mutation complete
Verifying
Measuring
Keeping
Reverting
Reverted
Clean
Faulted
```

Then startup behavior becomes much less ambiguous.

---

## Step E4 — Define startup behavior for each phase

### `Prepared`

Rollback state exists, but mutation may not have happened.

Safe response:

```text
restore idempotently
→ verify
→ clean
```

### `Applying`

Token exists.

Assume partial mutation is possible:

```text
restore
→ independently verify
→ clean
```

### `Applying` without token

This should become impossible for crash-recoverable providers.

If encountered from historical data:

```text
Faulted
manual intervention
```

### `Applied` / `Verifying` / `Measuring`

Restore from durable token.

### `Reverting`

Retry rollback idempotently.

---

# Phase F — Complete the real crash matrix

Once transaction ordering is correct, turn the crash harness into a proper PR.

The current branch already has a useful process-level helper: it restricts itself to a disposable sibling process tree, requires three processes/five tasks, checks PID/starttime identity and intentionally pauses so the controller can be SIGKILLed after apply or during verification.

Build on that.

## Step F1 — Crash before apply

Expected:

```text
no mutation
no rollback needed
safe terminal state
```

---

## Step F2 — Crash after rollback preparation but before mutation

Expected:

```text
startup sees prepared token
restore is safe/no-op
state verified
journal clean
```

---

## Step F3 — Crash during apply

Terminate after one or more task mutations.

Expected:

```text
startup recovery restores entire original set
all original masks independently re-read
no replacement process touched
journal clean
audit records crash recovery
```

---

## Step F4 — Crash immediately after apply

This specifically proves the rollback-before-verify fix.

---

## Step F5 — Crash during real verification

The current branch already supports pausing here.

Expected restore exactly as above.

---

## Step F6 — Crash after verification before measurement phase

Another transition boundary worth covering.

---

## Step F7 — Crash during rollback

Start rollback, kill controller part way through, restart.

The `Reverting` state should make rollback safely repeatable.

---

## Step F8 — Force verification failure without process death

Expected:

```text
verify error
→ automatic rollback
→ independent original-state verification
→ journal clean/reverted
```

---

## Step F9 — Partial apply error

Use controlled injection.

Expected:

```text
partial rollback token
→ rollback attempted
→ terminal audit
```

The action runner already has this model; live acceptance should now prove it at the controller boundary.

---

## Step F10 — Rollback failure

Expected:

```text
Faulted
future mutation blocked
rollback token retained
manual restore command retained
audit/history preserved
```

Do not clean the journal on rollback failure.

---

## Step F11 — PID/TID replacement

Reproduce PID/TID reuse with differing starttime.

Expected:

```text
replacement process untouched
recovery reports identity mismatch/stale record
```

---

## Step F12 — Suspend/resume

During an active experiment:

```text
suspend
resume
```

Require:

- current experiment invalidated;
- unsafe measurement window discarded;
- active mutation restored or explicitly revalidated according to policy;
- stabilization window before new experimentation;
- no immediate “keep” based on pre-suspend data.

---

## Step F13 — Close #29 only after real evidence exists

Synthetic/fault-injection unit tests are necessary but insufficient.

Retain a clean-source real-system evidence bundle.

---

# Phase G — Finish #27 properly

The basic observation harness is already good.

Do not rewrite it.

## Step G1 — Add controlled game-objective acceptance

The current generic workload proves:

```text
BPF
scheduler tracking
histograms
artifacts
reporting
```

but not:

```text
GameRunnableLatency objective semantics on a real capture
```

Add a deterministic game-like target acceptance scenario.

It must produce enough directly scoped samples for:

```text
foreground/game runnable p99
severe-tail rate
sample sufficiency
```

and then run the same objective machinery used by autotune.

---

## Step G2 — Add a live fixed-work compile objective scenario

#37's functionality is complete, but now prove it end-to-end:

```text
foreground latency helper
+
fixed-work compile command
+
ordered completion markers
+
objective signal production
+
foreground guards
```

This is validation, not a reason to reopen #37.

---

# Phase H — Build proper portable live-evidence export

This is a new concrete requirement exposed by the current implementation.

## Step H1 — Create one canonical exporter

For example:

```text
xtask evidence-export --input <acceptance-run> --output <bundle>
```

or:

```text
xtask live-acceptance export
```

Do not make every suite invent its own export logic.

---

## Step H2 — Use an explicit allowlist

Include:

```text
receipt
environment metadata
Stutter commit
binary digest
Cargo.lock digest
kernel release
distribution
CPU model
probe/tracepoint preflight
drop counters
target identity in sanitized form
schema versions
objective signal summary
journal
sanitized audit
sanitized history
validation result
```

Include raw event data only where deliberately required.

---

## Step H3 — Privacy must be a gate

Replace the current:

```bash
rg suspicious-patterns || echo ...
```

with:

```text
scan
↓
matches?
  yes → FAIL EXPORT
  no  → PASS
```

Scan at least:

```text
/home/<user>
/root
usernames
hostnames
email addresses
repository absolute paths
temporary absolute paths
tokens/secrets
unexpected environment variables
```

Prefer structured sanitization before regex detection.

---

## Step H4 — Add `MANIFEST.sha256`

Every exported file should be represented.

Then add:

```text
xtask evidence-verify <bundle>
```

which:

1. verifies checksums;
2. verifies schemas;
3. verifies required files;
4. reruns privacy checks;
5. verifies internal references;
6. verifies source/binary provenance.

---

## Step H5 — Define raw versus sanitized retention

Recommended:

```text
raw local bundle:
    temporary/private
    contains complete debugging evidence

sanitized repository/release bundle:
    portable
    minimal
    checksummed
    privacy validated
```

Large real captures should not be committed directly to Git.

---

# Phase I — Modernize live CI integration

Current CI still has a manual `live-smoke` job running the older:

```text
sleep_tree_churn.sh
record_minimal.sh
```

rather than the new canonical observation/affinity/recovery suites.

## Step I1 — Keep ordinary hosted CI unchanged

Every PR should still receive:

```text
fmt
build
clippy
tests
objective contracts
objective docs
daemon safety
schema/fixture tests
```

---

## Step I2 — Reclassify the old hosted live-smoke

Keep it if useful, but name it clearly:

```text
hosted-loader-smoke
```

It tests loader/basic integration.

It does **not** establish product acceptance.

---

## Step I3 — Add trusted live-validation workflow

Prefer:

```text
workflow_dispatch
+
trusted self-hosted machine
```

or a documented local release-gate process.

Never execute arbitrary public PR code with root/BPF privileges on a trusted self-hosted runner.

Only:

```text
main
specific owner-selected commit
release candidate
```

should run there.

---

## Step I4 — Canonical live suites

Eventually:

```text
xtask live-acceptance observation
xtask live-acceptance affinity
xtask live-acceptance recovery
xtask live-acceptance overhead
```

Upload only sanitized bundles.

---

# Phase J — Close #27

Close it once:

- generic observation acceptance passes;
- controlled game-objective acceptance passes;
- relevant report/diagnosis/advisor pipeline passes;
- portable evidence export passes;
- retained exact-source real-kernel evidence exists.

---

# Phase K — #30 real-kernel compatibility corpus

Now the acceptance harness and evidence format are stable enough to use repeatedly.

## Step K1 — Define statuses

Use:

```text
ObservedCompatible
ObservedDegraded
ObservedIncompatible
Untested
```

Never call an untested kernel supported.

---

## Step K2 — Capture at least several materially distinct kernels

For every environment run:

```text
tracepoint preflight
BPF loader
canonical observation acceptance
drop validation
artifact validation
```

Preserve tracepoint format dumps.

---

## Step K3 — Separate mandatory and optional probes

Example:

| Capability | Requirement |
|---|---|
| `sched_wakeup` | required |
| `sched_switch` | required |
| scheduler auxiliary traces | optional/degraded |
| IRQ | optional |
| block | optional |
| DRM/KMS | optional/degraded |

One unavailable optional probe should not make the entire kernel “unsupported.”

---

## Step K4 — Add regression fixtures for new layouts

When a materially new compatible kernel layout is encountered:

```text
capture
sanitize
fixture
decoder/preflight regression test
```

---

# Phase L — #38 profiler overhead

Do before product graduation.

## Step L1 — Define probe configurations

Measure:

```text
no Stutter
scheduler core only
core + IRQ
core + block I/O
core + GPU/display
full supported observation set
```

---

## Step L2 — Capture

For each:

```text
Stutter CPU usage
Stutter RSS
event rate
ring-buffer drops
write errors
target throughput/FPS
target frame-time statistics
target scheduler statistics
```

---

## Step L3 — Use repeated counterbalanced runs

Do not compare:

```text
off once
on once
```

Use the existing A/B methodology.

---

## Step L4 — Define acceptable overhead

Do not invent the limit before measuring.

Collect data first.

Then establish supported operating guidance.

---

# Phase M — Start #34 before the larger P2 corpus

This changes the order of the existing P2 issues slightly.

The provenance schema should exist **before** collecting dozens of real captures.

## Step M1 — Add evidence provenance enum

```rust
enum EvidenceKind {
    Synthetic,
    FixtureReal,
    ControlledReal,
    RealWorkload,
}
```

---

## Step M2 — Add capture metadata

Every validation corpus entry should carry:

```text
evidence_kind
source_commit
artifact_schema
kernel
CPU
GPU where relevant
workload
workload_version
scenario
expected_primary_cause
expected_confidence
control_type
probe set
drop counters
notes
sanitization revision
```

---

## Step M3 — Add validation coverage command

For example:

```text
cargo run -p xtask -- validation-coverage
```

Report by:

```text
diagnosis kind
objective
provider
```

and evidence strength:

```text
synthetic
fixture-real
controlled-real
real-workload
```

---

## Step M4 — Use explicit denominators

A “real evidence percentage” is only meaningful if the denominator is explicit.

For example:

```text
9 objective kinds
12 diagnosis kinds
1 currently supported mutable provider
```

Then report actual coverage.

---

# Phase N — #31 real diagnosis corpus

Collect each diagnosis systematically.

For every cause:

```text
positive case
negative control
mixed/confounded case
```

## Scheduler delay

```text
periodic latency-sensitive target
+
controlled CPU contention
```

Control:

```text
same target without contention
```

---

## Block I/O

```text
controlled blocking storage workload
```

Control:

```text
similar CPU load without storage pressure
```

---

## IRQ overlap

Use a controlled, reproducible interrupt-heavy workload where practical.

Do not label mere overlap as causality.

---

## GPU-bound

Capture:

```text
known GPU saturation
```

and control:

```text
CPU-bound workload with moderate GPU usage
```

---

## Gaming

Use both:

```text
controlled game-like scenario
real Proton/game capture
```

The controlled case provides repeatability.

The real case provides ecological validity.

---

## Compile

Use the fixed-work protocol plus an interactive foreground workload.

---

## Browser/desktop

Capture actual interactive latency cases after gaming/compile paths are mature.

---

# Phase O — #32 threshold calibration

Calibrate one diagnosis at a time.

Do **not** change every threshold in one PR.

For scheduler delay, for example:

```text
0.5 ms
1.0 ms
1.5 ms
2.0 ms
2.5 ms
3.0 ms
4.0 ms
```

For every threshold calculate:

```text
true positives
false positives
false negatives
cause-selection stability
confidence stability
```

The goal is not simply maximum sensitivity.

For an evidence-oriented tuner, false confident diagnoses are particularly expensive.

---

# Phase P — Calibrate confidence separately from detection

Distinguish:

```text
"We measured substantial scheduler delay"
```

from:

```text
"Scheduler delay caused the visible frame spike"
```

Those are different claims.

Confidence must reflect:

- directness;
- temporal correlation;
- competing explanations;
- signal quality;
- missing probes;
- drop counters;
- repeatability.

---

# Phase Q — #33 evaluate `StutterScore`

Only after the corpus is meaningful.

## Step Q1 — Write the semantic contract

A defensible definition is:

> StutterScore is a scalar ranking proxy for degradation between comparable controlled measurements.

It is **not** an absolute percentage of user-perceived stutter.

---

## Step Q2 — Test monotonic behavior

Induce controlled scheduler delay:

```text
0
1 ms
2 ms
4 ms
8 ms
16 ms
```

Check whether score ranking behaves sensibly.

---

## Step Q3 — Compare with independent signals

Correlate with:

```text
runnable p99
runnable severe-tail rate
frame p99
frame p99.9
>16.6 ms frames
>33 ms frames
```

---

## Step Q4 — Do not change weights just because they look arbitrary

Existing coefficients should remain if they rank degradation reliably.

Change them only when the real corpus demonstrates a materially better model.

---

# Phase R — Do not let score calibration undo P0

Even after calibration:

```text
GameRunnableLatency
```

must remain driven by runnable latency.

```text
CompileThroughputWithForegroundProtection
```

must remain driven by completed fixed work.

Generic score is:

```text
supporting evidence
guardrail
general objective
```

—not the hidden primary for every objective.

---

# Phase S — #35 capability maturity framework

After P1/P2 evidence exists, classify functionality.

Recommended states:

```text
Supported
Experimental
Internal
Disabled
```

## Supported requires

```text
explicit semantic contract
unit/fixture regression tests
real-system acceptance
known failure behavior
stable artifact compatibility
diagnostics
rollback evidence where mutable
```

---

# Phase T — Provider graduation contract

Every mutable provider should need:

```text
[ ] explicit action descriptor
[ ] safety class
[ ] policy gate
[ ] scope limit
[ ] preflight
[ ] dry-run
[ ] prepared/durable rollback state where crash relevant
[ ] apply
[ ] verification
[ ] rollback
[ ] independent rollback verification
[ ] strict startup recovery contract or explicit ManualOnly
[ ] audit
[ ] journal
[ ] partial-apply test
[ ] verification-failure test
[ ] rollback-failure test
[ ] live apply/restore evidence
[ ] crash/restart acceptance
[ ] provenance evidence
```

CPU affinity should be the first provider to satisfy the entire list.

---

# Phase U — Initial capability classification

Tentatively, once the remaining gates pass:

## Supported candidate

```text
doctor
record
bench
report
diagnose
advisor/recommend
tune evaluation
manual/reversible CPU-affinity workflow
audit
restore
```

## Experimental

```text
always-on autotune
medium-risk mutation
remote state-changing control
nice/ioprio automation
uclamp
IRQ affinity
CPU/GPU power
VM knobs
system-wide scheduler changes
```

Do not promote those because their implementation exists.

Promote them when their evidence exists.

---

# Phase V — #36 first versioned release

Only start this after:

```text
#27 closed
#29 closed
#30 sufficient initial coverage
#38 complete
P2 calibration has produced a defensible core
#35 capability tiers complete
```

---

## Step V1 — Publish release contract

Document:

```text
supported commands
experimental commands
supported architecture
kernel compatibility model
privilege requirements
artifact compatibility
known limitations
```

---

## Step V2 — Reproducible build

Release should provide:

```text
binary/artifacts
source archive
SHA-256 checksums
dependency/SBOM information if practical
release notes
```

---

## Step V3 — Keep installation scope narrow

Initially:

```text
Nix
source build
```

is enough.

Do not delay the first release to create packaging for every distribution.

---

## Step V4 — Minimal tutorial

A new user should be able to perform:

```text
install
→ stutter doctor
→ record workload
→ generate report
→ inspect diagnosis
→ optionally perform reversible CPU-affinity experiment
→ restore
```

---

## Step V5 — Fresh-checkout release candidate gate

From a clean checkout of the exact candidate commit:

```text
Nix environment validation
full build
all tests
Clippy
objective contracts
objective docs
schema validation
offline smokes
canonical observation acceptance
affinity acceptance
crash recovery acceptance
evidence verification
package build
install package
doctor smoke
record/report smoke
```

Only then create:

```text
v0.1.0
```

---

# 12. Recommended new issues

The existing ten open issues are mostly well chosen, but this audit exposed work that deserves explicit tracking.

I would add approximately four new issues.

## New P1 — Durable rollback before verification

```text
Ensure live controller persists a verified rollback token before action verification starts.
```

Acceptance:

```text
production path uses journal transaction
journal fsync/rename complete before verify
journal-write failure automatically rolls back
SIGKILL after apply recovers
```

---

## New P1 — Crash-safe rollback preparation

```text
Persist provider rollback state before first mutable syscall.
```

Acceptance:

```text
CPU-affinity restore snapshot durable before mutation
experiment-specific restore path
crash during apply restores original state
```

---

## New P1 — Startup recovery provider capability registry

```text
Automatic crash recovery must be explicitly authorized per rollback token/provider.
```

Acceptance:

```text
CPU affinity VerifiedAutomatic
unverified providers ManualOnly
no automatic legacy task restoration by accidental registry inclusion
```

---

## New P1/P2 — Portable live evidence export

```text
Export exact-source live acceptance evidence with checksums, schema verification and fail-closed privacy scanning.
```

This can alternatively be folded into #27 and #34 if you want to keep the issue count smaller.

---

# 13. Recommended PR sequence from the repository as it exists now

The most practical sequence is:

## PR #45 — finish and merge

```text
startup CPU-affinity identity
restore-format routing
scope wording
```

---

## Next PR — startup recovery capability restrictions

Small, safety-focused.

```text
VerifiedAutomatic vs ManualOnly
block legacy/unproven automatic recovery
```

---

## Next PR — durable rollback token before verify

Rebase/split `fix/controller-rollback-before-verify`.

Wire the transaction into production.

---

## Next PR — crash-safe rollback preparation

Close the mid-apply hard-crash gap.

---

## Next PR — controller SIGKILL/restart acceptance

Use real disposable processes.

Complete the majority of #29.

---

## Next PR — failure/suspend recovery matrix

```text
partial apply
verify failure
rollback failure
PID reuse
rollback interruption
suspend/resume
```

Close #29.

---

## Next PR — portable live evidence bundles

Implement exporter/verifier/privacy gate.

---

## Next PR — controlled game-objective live acceptance

Finish #27.

---

## Next PR — live workflow integration

Trusted self-hosted/local release gate using canonical suites.

---

## Next PR — kernel compatibility evidence

Start #30.

---

## Next PR — profiler overhead

Complete #38.

---

## Next PR — provenance + coverage schema

Start/complete #34.

---

## Following PRs — real validation corpus

#31.

---

## Following PRs — individual diagnosis calibration

#32, preferably one diagnosis family per PR.

---

## Following PR — score evaluation

#33.

---

## Following PR — maturity/provider framework

#35.

---

## Final Core Validation sequence — release preparation

#36.

---

# 14. Branch cleanup point

After:

```text
PR45 merged
controller crash branch rebased
any useful commits extracted
```

delete:

```text
fix/autotune-objective-semantics
feat/fixed-work-compile-throughput
test/objective-contract-matrix
feat/objective-descriptors
feat/live-observation-acceptance
feat/live-affinity-acceptance
fix/startup-recovery-identity
```

after their respective merged state is confirmed.

Eventually delete:

```text
fix/controller-rollback-before-verify
```

after its replacement PR(s) merge.

The goal should be:

```text
main
+
only genuinely active development branches
```

rather than using branches as historical archives. Git already preserves the merged history.

---

# 15. Core Validation completion definition — revised

The milestone should now require all of the following.

## Objective correctness

- [x] direct runnable objective
- [x] compile activity separated from throughput
- [x] fixed-work completion
- [x] nine-objective contract matrix
- [x] descriptor registry
- [x] generated/checked documentation

## Observation

- [x] canonical real scheduler capture
- [x] clean-source provenance
- [x] complete histograms
- [x] zero-drop test
- [x] report/advisor pipeline
- [ ] controlled direct game-objective capture
- [ ] portable verified bundle

## Mutation

- [x] real CPU-affinity apply
- [x] real independent restore
- [x] multi-process target
- [ ] durable rollback token before verification in production
- [ ] rollback prepared before first crash-sensitive mutation

## Recovery

- [x] startup affinity identity fix merged
- [ ] automatic recovery capability restricted per provider
- [ ] crash before apply
- [ ] crash during apply
- [ ] crash after apply
- [ ] crash during verify
- [ ] crash during rollback
- [ ] forced verify failure
- [ ] partial apply
- [ ] rollback failure
- [ ] stale PID/TID replacement
- [ ] suspend/resume

## Portability

- [ ] multiple real kernel environments
- [ ] compatibility evidence corpus
- [ ] optional-probe degradation verified

## Measurement integrity

- [ ] basic profiler overhead characterized

## Validation

- [ ] provenance schema
- [ ] coverage reporting
- [ ] real/fixture-real diagnosis corpus
- [ ] threshold calibration
- [ ] confidence calibration
- [ ] StutterScore evaluation

## Product

- [ ] capability tiers
- [ ] provider graduation rules
- [ ] bounded supported surface
- [ ] first-release contract
- [ ] fresh-checkout release acceptance
- [ ] `v0.1.0`

Only then should the feature freeze be lifted.

---

# 16. What not to work on yet

Continue postponing:

```text
new power providers
new VM tunables
new IRQ mutation features
scheduler replacement
major remote-control expansion
another UI
large crate restructuring
new telemetry integrations
```

There is already enough architecture.

The remaining work is about converting existing architecture into **demonstrated guarantees**.

---

# 17. Immediate next steps

If development resumes right now, the correct order is:

1. Finish reviewing PR #45.
2. Make its CPU-affinity-only strict-recovery scope explicit.
3. Add the automatic-recovery provider capability restriction.
4. Merge #45 and verify `main`.
5. Rebase `fix/controller-rollback-before-verify`.
6. Wire its journaled transaction into the **actual production live-controller path**.
7. Prove rollback state is durable before verification.
8. Split CPU-affinity mutation into durable preparation → mutation so SIGKILL during apply is recoverable.
9. Turn the branch's crash controller into canonical `xtask live-acceptance recovery`.
10. Run after-apply and during-verification SIGKILL tests.
11. Add mid-apply, rollback, verification failure, stale-identity and rollback-failure cases.
12. Add suspend/resume acceptance.
13. Close #29.
14. Build the new fail-closed portable evidence exporter.
15. Add controlled game-objective live acceptance.
16. Close #27.
17. Clean the six stale merged branches and protect `main`.
18. Build the kernel corpus (#30).
19. Characterize overhead (#38).
20. Implement provenance/coverage (#34).
21. Collect real diagnosis corpus (#31).
22. Calibrate diagnoses (#32).
23. Evaluate StutterScore (#33).
24. Define supported capability/provider tiers (#35).
25. Perform the fresh-checkout release gate and create the first versioned release (#36).

That is now the shortest route from the current repository to a defensible first product milestone.

The key change from the previous plan is that **measurement semantics are no longer the bottleneck**. They have been substantially fixed and protected. The most consequential remaining risk is now the mutation lifecycle:

> **Can Stutter survive being killed at every point where it has changed the machine, know exactly what it changed, prove it is restoring the same resource it originally touched, and recover without trusting stale identity?**

Until that answer is convincingly “yes” for CPU affinity, expanding autotune further would still be premature. Once it is yes, the project is ready to move from safety validation into empirical calibration and then a narrow first release.

The biggest practical takeaway is that I would work almost exclusively on **PR #45 → durable rollback transaction → crash/restart acceptance** next. That chain contains the highest-severity remaining correctness work; kernel breadth, calibration, and release work all become much more useful after it is closed.
