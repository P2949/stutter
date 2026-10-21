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
