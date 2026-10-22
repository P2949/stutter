# Live core acceptance

Build as the normal development user in the pinned environment:

```sh
nix develop --command cargo build -p stutter -p xtask
```

The canonical observation entry point is:

```sh
cargo run -p xtask -- live-acceptance observation \
  --stutter-bin target/debug/stutter --out /tmp/new-acceptance-directory --seconds 10
```

The output directory must not exist; its parent must exist. Duration is bounded
between 3 and 60 seconds. Run the built xtask binary under sudo when the host
requires BPF/tracefs privileges, preserving the pinned toolchain PATH for metadata
collection. Build before the privilege transition. The harness never escalates
itself and never treats missing privilege as a successful skip.

Observation uses an isolated empty user config and no mutation commands. It
runs doctor and tracepoint-layout dump, then starts a disposable helper with two
threads on one CPU selected from its current allowed set. One thread runs CPU
work while the other wakes every millisecond. The affinity choice affects only
the helper and its inherited threads; it does not change any existing workload.
The helper is bounded and is killed/reaped by its owning child handle on errors.

Recording disables optional hwmon, CPU-frequency, faults, block and stat-wait
probes so the required scheduler path is independently testable. Commands have
deadlines and preserve separate stdout/stderr logs. The harness requires the
same live process identity throughout recording, a matching recorded process
identity, a tracked wakeup thread, positive target samples and interval p99,
complete histogram populations, current artifact schemas and zero drop counters.
It then invokes the validator, JSON analysis/diagnosis, HTML report and advisor.
No exact timing or particular diagnosis is required.

`result.json` records PASS, FAIL or BLOCKED; both FAIL and BLOCKED return nonzero.
A preflight failure is BLOCKED. Failure after recording begins is FAIL. Reports
from a core-only run can honestly have Medium quality or CollectMoreData advisor
output because optional correlation evidence was deliberately not collected.
The receipt's maximum interval p99 is a diagnostic invariant, not an aggregated
window percentile or a performance comparison.

The local evidence directory contains selected kernel/distribution/CPU/toolchain
metadata, source commit and dirty flag, binary digest, doctor/tracepoint output,
recording, reports and the result. It does not dump the environment. Raw recording
and command logs can still contain local paths or process details: inspect and
sanitize before publication. Portable sanitized export, a controlled game-objective fixture, and multiple-kernel evidence remain
follow-up requirements. A development-tree PASS does not close those gates.

The independent affinity, crash/recovery, suspend/resume and overhead suites are
still outstanding. This command cannot graduate mutation support or claim a
release boundary.

Canonical runs require a clean committed source tree and a stutter binary whose
reported Git revision matches that tree. The binary's SHA-256 and version are
retained. Commit the harness first, rebuild both binaries, then run acceptance;
otherwise preflight is BLOCKED. The earlier development-tree attempt is useful
functional evidence but does not satisfy this stronger provenance gate.

## Independent affinity suite

`xtask live-acceptance affinity --stutter-bin target/debug/stutter --out NEW_DIR`
uses the existing CPU-affinity action and audited runner on a caller-owned
five-task, three-process disposable helper. It runs without eBPF and does not require root
when the host permits changing affinity of same-user tasks. Choose a host with
more than one allowed CPU so a real mask change can be observed.

It snapshots process/task start times and original masks, chooses an online CPU
from the current allowed set, performs an audited dry-run and checks that no
mask or restore state changed. Apply uses an explicit new restore path in the
evidence directory. The suite checks durable original-state records, independently
reads applied masks from procfs, rolls back, and independently checks every
original mask and task identity again. It retains runner audit phases plus
explicit acceptance rollback and final readback records. Post-apply evidence
failures still trigger rollback. User audit and restore files are untouched.

This suite does not prove recovery after SIGKILL, PID reuse during recovery,
partial-apply faults, suspend/resume, or rollback-failure handling. Those require
the separate crash/recovery matrix before provider graduation.

The affinity helper includes two separate child processes in addition to its
worker threads. Child readiness is explicit. Each child waits on a parent-owned
stdin pipe, so parent death closes the writer and causes child exit; normal
cleanup also kills and reaps the owned child handles. The receipt records three
processes and five tasks. Before restoration, a separate create-new copy of the
validated rollback journal is retained as `restore.before-rollback.json`.

## Automatic recovery identity policy

Before automatic startup recovery restores an affinity token, every original
record must include nonzero task/process IDs and process/task start times. This
is validated for the entire batch before any restore syscall. Incomplete legacy
identity, unsupported/ambiguous restore format, or too few original identities
for the token leaves recovery faulted with its original state retained for
manual inspection. Complete records whose start times no longer match are
skipped without changing the replacement process.

Profile restore schema 1 and historical affinity restore schemas 1–3 are routed
by their actual record layout. An affinity `records` array cannot be interpreted
as an empty profile file. Explicit manual restoration retains legacy identity
compatibility for supported formats; automatic recovery has stricter identity
requirements. The real-child regression tests exercise these semantics but do
not replace controller SIGKILL/restart acceptance.

Automatic startup recovery also has an explicit per-token capability gate,
independent of the manual rollback registry. CPU-affinity restore-file tokens
are `VerifiedAutomatic`, conditional on the strict whole-file identity/layout
validation above. Nice, I/O priority, uclamp, cgroup, IRQ affinity, CPU/GPU power,
and VM tokens are `ManualOnly`. Generic arbitrary-path sysfs tokens are
`NotRecoverable` automatically. Their explicit manual paths remain available.
The startup registry contains only the strict affinity handler and no catch-all
manual handler. An unauthorized token faults before executor dispatch, retains
the journal/token/manual instruction, and records failure in audit, history and
daemon state. This permission boundary does not establish full crash durability,
rollback repeatability, provider graduation, or completion of issue #29.

## Low-risk controller transaction

The low-risk runtime executor uses the shared journaled action transaction:
`Applying → apply → durable Applied(token) → verify`. The Applied write uses
the controller journal's synced temporary file, atomic rename and parent sync.
The live manager checks that exact experiment/action/token instead of rewriting
Applied after the executor returns. It subsequently advances the experiment to
measurement as usual. A failed Applied write immediately invokes rollback and
skips verification. Successful automatic rollback cleans the journal; failed
rollback retains Applied/token for recovery. The runner records terminal failure
in either case.

Each real runtime apply reserves a fresh directory under the journal parent's
`experiments/<timestamp>-<controller-pid>/restore.json`; directory entries are
synced before use. This separates experimental restore data from shared manual
state. Explicit audit/journal paths allow owned-process validation without user
state or environment-variable changes.

The privileged medium-risk service retains its historical delayed journal
protocol and remains experimental. The low-risk fix protects verification after
apply returns; preparation before the first mutable syscall and real SIGKILL
acceptance are separate pending gates (#48/#29). Synthetic transaction regressions
and the explicit owned-child manager/startup-restore test do not establish that
full crash matrix.

## Failed apply reporting

The journaled action error carries the actual observed rollback token and whether
the runner completed rollback. Runtime reporting does not infer these facts from
an error-message string. Partial-apply tokens also enter the journal hook before
rollback, and successful partial rollback runs the cleanup hook.

If rollback fails before the manager has registered the experiment, the manager
retains that experiment/token for manual recovery and daemon-state reporting.
The runtime enters Faulted, emits fault decision/history, and atomically saves a
fault snapshot as `daemon_state.json` beside an explicit controller journal (or
at the normal default state path). This happens before returning the original
error, so a caller's early return cannot discard the only state update. Subsequent
mutation is blocked. If one reporting destination fails, the others are still
attempted and the persistence error accompanies the original action error.

The synthetic fault contract covers preflight failure, verification failure with
successful/failed rollback, partial apply with successful/failed rollback, and a
history-write failure. It checks journal, audit, history, saved daemon state and
blocked future dispatch. Real SIGKILL, interrupted rollback, suspend/resume and
the full physical failure matrix remain separate acceptance gates.

## Rollback preparation before affinity mutation

Affinity-only actions support a preparation callback through the real audited
runner (including boxed actions). Preparation captures a single owned plan and
persists its original identities/masks using a unique temporary file, file sync,
atomic rename and parent-directory sync. The controller then durably writes
`Applying(token)` with `rollback_prepared_before_mutation`. Only after that
callback returns may commit execute affinity syscalls. Commit consumes the exact
prepared plan and revalidates original masks and process/task start times before
mutation. Failures before commit do not trigger restore syscalls; the snapshot
is retained. Failures after mutation may have begun return the prepared token to
the runner's existing rollback path.

Startup and explicit emergency restore now handle `Applying` with a token.
Historical tokenless `Applying` still faults. This uses the existing journal
schema/phase rather than adding a new Prepared enum variant. Priority-bearing
profiles retain their experimental/manual protocol and do not inherit this
affinity-only preparation guarantee. The default preparation method for other
providers is compatibility-only and does not qualify automatic recovery.

Earlier affinity evidence proved restore-file presence, not file/directory sync;
the preparation work closes that newly identified durability gap. The explicit
owned-child tests check the real callback before mutation, the journaled boxed
runner plus startup restore, and rejection before commit. Syscall tracing and
canonical SIGKILL/restart evidence are separate gates; the latter still keeps
#48/#29 open.

## Suspend/resume acceptance (opt-in; suspends the machine)

```bash
cargo run -p xtask -- live-acceptance suspend-resume \
  --stutter-bin target/debug/stutter --out NEW_DIR \
  --allow-system-suspend [--rtcwake-seconds 30] [--suspend-command "systemctl suspend"]
```

Without `--allow-system-suspend` the suite writes a `BLOCKED` receipt and never
suspends. It starts a real low-risk CPU-affinity candidate on the owned disposable
tree, proves it is physically applied and `Measuring`, records a pre-suspend receipt
(including the kernel's `suspend_stats`), suspends, and requires after resume: the
runtime's wall/monotonic clock discontinuity detection, a cleared measurement window,
the active candidate reverted, a stabilization cooldown, and independently re-read
original masks. Run from a clean committed tree. If `rtcwake` is not permitted, wake
the machine manually.

## Portable evidence bundles

Raw suite output is local and may contain account, host and path information.
Publishable evidence is exported and verified with:

```bash
cargo run -p xtask -- evidence-export --raw RAW_DIR --out BUNDLE_DIR
cargo run -p xtask -- evidence-verify BUNDLE_DIR      # nonzero exit on any failure
```

Bundle layout: `receipt.json`, `environment.json`, `validation.json`,
`MANIFEST.sha256`, `README.md`, `logs/`, `artifacts/`. The receipt records schema
version, suite, evidence kind (`synthetic`, `fixture_real`, `controlled_real`,
`real_workload`), source commit and dirty state, binary version and SHA-256,
Cargo.lock SHA-256, artifact schemas, capture start/end, PASS/FAIL/BLOCKED/INCONCLUSIVE
status and explicit `not_claimed` limitations. The environment adds kernel, distribution
and CPU; `proc_version` is deliberately excluded because it embeds the kernel
builder's identity.

Export is sanitize → scan → manifest → independent verification, and publishes nothing
on failure. Replacements use documented placeholders (`<EVIDENCE_DIR>`, `<CHECKOUT>`,
`<HOME>`, `<TMP>`, `<USER>`, `<HOST>`, `<EMAIL>`); provenance values (commit, digests,
kernel, distribution, CPU) are never redacted. The verifier rechecks manifest digests,
required files, JSON validity, schemas, suite/kind/capture/status agreement,
provenance, relative references, absence of raw or private artifacts (binary files,
symlinks, key/credential names) and runs the privacy scan again, including digests of
the exporting machine's account and host names. Raw evidence that lacks capture
identity or artifact schema is refused: rerun the suite from the exact committed source
rather than reconstructing metadata. `scripts/artifacts/export_evidence_bundle.sh` is the
legacy, fail-open exporter and is not canonical.

## Evidence maturity coverage

`cargo run -p xtask -- fixture-coverage` reports, per diagnosis kind, objective,
mutation provider and kernel capability (denominators come from `StutterCause::ALL`,
`ObjectiveKind::ALL`, `MUTATION_PROVIDER_KINDS` and the probe registry), how many
`synthetic`, `fixture_real`, `controlled_real` and `real_workload` evidence items cover
it, plus the surfaces that are synthetic-only/uncovered or lack real-workload evidence.
New captures declare `evidence_kind` and a complete `[provenance]` table
(`source_commit`, `capture_id`, `kernel`, `distribution`, `cpu`, `gpu`, `compositor`,
`workload`, `workload_version`, `scenario`, `expected_cause`, `expected_confidence`,
`control_type`, `probe_set`, `drop_counters`, `artifact_schema`, `limitations`,
`privacy_state`); fixtures without `evidence_kind` keep the legacy inference and are
counted. Optional `[coverage]` single-line arrays (`objectives`, `providers`,
`kernel_capabilities`) tag what a fixture covers. Verified live bundles are pinned by
manifest digest in `docs/evidence/index.json`; only PASS entries count.

## Controlled game-runnable acceptance (needs root/CAP_BPF)

```bash
sudo target/debug/xtask live-acceptance game-runnable \
  --stutter-bin target/debug/stutter --out NEW_DIR [--seconds 20]
```

A disposable workload is installed under `NEW_DIR/steamapps/common/...` so the production
task classifier (not a test hook) must classify it as a game. Two scenarios run on the same
workload pinned to one CPU: `baseline` (no contention) and `degraded` (three CPU hogs on the
same CPU). For each: `record`, artifact validation, analysis, HTML report and advisor run,
then the objective signal is computed from the recording by production code
(`runnable_signals_for_target`, the same merged-histogram logic as the live controller).
Hard requirements per scenario: stable target PID and start time, a recognised game task
population, at least the objective's minimum scheduler samples, a complete merged histogram,
positive runnable p99, an available severe-tail count, zero drops and a direct signal. The
degraded p99 must be strictly worse than the baseline's. Effect-size thresholds stay
provisional and are not tuned here. Without privileges the suite reports `BLOCKED`.
