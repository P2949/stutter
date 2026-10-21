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
three-task disposable helper. It runs without eBPF and does not require root
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
