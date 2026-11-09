# Validation cgroup containment

W1 remains **PARTIAL**. The xtask supervisor now contains fork/setsid/environment-cleared
descendants in a fresh cgroup. The privileged runner has not yet adopted this primitive; its
delegation/recovery integration, bounded logs and whole-runner deadline remain required. No
product service was deployed or restarted and no Supported operation was added.

Before launching a command, the caller must have an owned cgroup v2 delegation. The guard creates
a child under its current cgroup, preserving ancestor limits, and attaches the new process through
an opened `cgroup.procs` descriptor before exec. Missing delegation or kill support refuses launch.
TERM uses pidfds opened before kernel membership checks; a bounded grace precedes `cgroup.kill`.
The final population check and cleanup must succeed for the stage to pass.

An independent minimal reaper owns the original kill descriptor and waits for EOF on a private
CLOEXEC socket. Owner SIGKILL or exec closes that socket. The reaper then kills the original
container, including detached children that removed the old environment marker. Its shell has
root-protected canonical ancestors and an empty environment. This handles inherited descendants;
it is not a sandbox against deliberate cgroup migration by a process with delegation authority.
Manager-created services and mutation workloads require their separately verified lifecycles.

Private owner-only birth records bind boot, owner PID/start ticks, parent device/inode and the
created cgroup device/inode. Intent is durably published before creation and completion before
workload launch. Atomic publication prevents partial reads and initial publication refuses
replacement. Crash-resume cleanup validates live kernel identity, refuses a changed inode/owner,
and never kills a populated creation intent without completed birth proof. These records govern
validation containers; the product worker does not accept them as operation ownership receipts.

The first inode-in-name prototype failed before launch because cgroup v2 forbids rename
([kernel source](https://github.com/torvalds/linux/blob/master/kernel/cgroup/cgroup.c)). The birth
journal is the tested replacement. Earlier preflight, fixture and shared-Cargo-target failures are
retained under `.llm-runs/`; zero-test executions and the shared-target CI attempt are excluded.
Clean committed-source CI at `6638a5b3` used its exclusive checkout/target and passed independently
of this newer slice ([receipt](source-ci-2026-11-09.json)).

Development validation in the pinned Nix environment:

* Full xtask suite: 516 passed plus its integration test.
* Validation-runner suite before the final temporary-file case: 91 passed, four live tests ignored.
* Focused containment/journal tests explicitly include the live tests: owner SIGKILL, markerless
  new session, unaffected sentinel, inode replacement, atomic publication, regular-filesystem
  refusal, symlinks/hardlinks, oversized records and temporary-intent cleanup.
* Integrated xtask/runner Clippy runs with warnings denied. Final focused results and hashes are
  recorded alongside the retained logs when this slice is committed.

Reproduction:

```sh
nix develop --command cargo test -p stutter-validation-runner containment:: -- --include-ignored --nocapture
nix develop --command cargo test -p xtask process::supervisor::tests
nix develop --command cargo test -p xtask
nix develop --command cargo clippy -p xtask -p stutter-validation-runner --all-targets -- -D warnings
```

Each concurrent checkout must use its own Cargo target directory. Cargo's build lock does not
make concurrent test execution safe when another checkout can replace the same executable.

The console-echo follow-up reproduces a full stdout pipe in a disposable test process. Echo now
uses an independently nonblocking pipe/device description or per-call nonblocking socket send;
it preserves regular-file offsets and never changes the caller's stdout flags. Lost echo bytes
are counted separately and make the validation stage fail as incomplete evidence. The original
five-second regression timeout and the passing follow-up are retained in `.llm-runs/tee-*`.
The unsafe-documentation guard, all 17 process tests, full library (3,265 passed, ten ignored),
and xtask Clippy passed after correcting the two immediately preceding SAFETY comments. These
checks do not constitute consolidated CI or deployment qualification for this follow-up.
