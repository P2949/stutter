# Validation campaign

`xtask validation-campaign` is the execution mechanism for Core Validation. The campaign, not
the agent, owns dependencies, provenance, pass/fail state, retries, corpus sufficiency, kernel
progress, capability policy, release readiness and source invalidation. The agent loop is:

```bash
cargo run -p xtask -- validation-campaign status --json   # source of truth
cargo run -p xtask -- validation-campaign continue         # perform everything runnable
```

## Commands

| command | purpose |
| --- | --- |
| `init --manifest docs/validation/core-validation.toml` | create a campaign from a clean `main` (refuses a dirty tree, another branch, a missing/mismatched `stutter` binary, an invalid manifest or another active campaign) |
| `bootstrap` | detect the privileged validation runner (installation arrives in the runner phase) |
| `status [--json]` | stable machine-readable state; never exposes raw command lines |
| `next` | read-only prediction of the next runnable task |
| `continue [--max-tasks N] [--resume]` | run until `COMPLETE`, `WAITING_FOR_REBOOT`, `BLOCKED_RESOURCE`, `BLOCKED_PRIVILEGE` or `FAILURE_REQUIRES_SOURCE_CHANGE` (exit codes 0/10/11/12/13) |
| `verify --strict` | rebuild truth from retained attempts, receipts and bundles instead of cached statuses |
| `report` | write `report.json` and `report.md` |
| `cancel` | stop future automation; collected evidence is never deleted |

## State

State lives in `.llm-runs/validation/<campaign-id>/` (ignored by Git): `campaign.json`,
`manifest.snapshot.toml`, `attempts/<task>/<n>/` (immutable `attempt.json`, raw output,
`failure.json`), `bundles/<task>/<capture-id>/` (verified portable bundles), `receipts/`,
`indexes/` (including `combined.json`: the committed historical index plus the campaign's
verified entries) and `release/`. Every write is temp file, `sync_all`, rename, parent
directory sync. Attempts are append-only: a retry adds a record and never erases a failure.

## Task graph and issue gates

The DAG is encoded in `validation_campaign/graph.rs`:

```text
offline-ready -> live-observation, game-runnable, overhead-controlled, overhead-real,
                 score-runnable, kernel-matrix, corpus, frame-score
              -> evidence-integration -> diagnosis-calibration, score-review
              -> capability-policy -> release-docs -> freeze-rc
              -> rc-offline, rc-live-observation, rc-game-runnable, rc-affinity, rc-recovery
              -> release-package -> release-verify, install-smoke -> tag
```

Issue gates: #27 observation + game-runnable; #30 kernel matrix; #38 controlled and real
overhead; #31 corpus; #32 calibration; #33 runnable + frame score + review; #35 capability
policy; #36 the exact-RC release path through `tag`. `status --json` emits `repository_actions`
(`close_issue`, `push_tag`) that the agent performs with its own authenticated `gh`; the
campaign holds no GitHub credentials, and an issue is `ready` only when every gate task truly
passed.

## Integrity and invalidation

Before each source-bound task the campaign verifies: `HEAD` equals the evidence base (or the
frozen release candidate for exact-RC tasks), the tree is clean, the manifest, `Cargo.lock`,
`stutter` and `xtask` digests are unchanged, and the binaries report the source commit. A
failure sets the campaign to `Invalidated`, marks only the affected binding's tasks (and their
dependents) `Invalidated`, and keeps their evidence as history. A `Passed` result counts only
while its fingerprint matches the current one. A moved `main` after collection (evidence
integration, policy PRs) is expected and does not invalidate completed work; a defect found
during collection is fixed through a PR and a successor campaign is initialized at the new
`main`.

## Failure classes

`Retryable` (bounded attempts, then an honest `BlockedResource` with retained attempts),
`BlockedResource` and `BlockedPrivilege` (never replaced by a fabricated substitute),
`SoftwareDefect` (stop collecting, fix by PR) and `ValidationFailure` (record the limitation
and apply the conservative product policy; never rewritten to pass).

## Atomic evidence

A live suite's result is not task evidence until its portable bundle verifies: raw attempt,
export into a dot-prefixed partial directory, `evidence-verify`, derived index entry, fsync,
atomic rename into `bundles/`, receipt. Partial directories never count as published; a
duplicate capture id or a bundle for another commit is rejected.

Manual shell recipes in `LIVE_ACCEPTANCE.md` remain a fallback for debugging only.

## Privileged runner (phase B)

Live suites need BPF and scheduler privilege; the campaign must not obtain it by running the
whole mutable `xtask` as root or by handing the agent `sudo sh`. Two trust domains result:

| domain | components |
| --- | --- |
| normal campaign domain | `xtask validation-campaign`, the agent, repository code, workloads, Git/GitHub |
| privileged validation domain | `stutter-validation-runner` (root service), root-owned authorization, capability delegation, boot control |

`stutter-validation-runner` is a workspace crate built by the ordinary gate. It is **not** part
of the product package (a test asserts the tarball manifest cannot contain it) and its units
`packaging/systemd/stutter-validation-{runner,resume}.service` are validation-only.

**Protocol.** Length-prefixed JSON over `/run/stutter-validation/runner.sock` (owned by the
campaign user, mode 0600), schema-versioned, frames capped at 64 KiB, unknown fields refused.
Requests are a closed enum: `Status`, `ExecuteLiveSuite`, `ExecuteSchedulerBurst`,
`PrepareKernelCycle`, `SetNextBoot`, `Reboot`, `ResumeCampaign`, `CancelCampaign`. There is no
shell, exec, command or argument field, and no caller-chosen path: executables and output
directories derive from the authorization. Every response carries a structured error and a
privileged receipt id (rejections are receipted too).

**Authorization.** `/var/lib/stutter-validation/<campaign>/authorization.json` is root-owned
(mode 0600) and pins the campaign id, owner uid/gid, canonical repository, manifest, runner,
`xtask` and `stutter` digests, the `PATH` the suites run with, per-task grants
(`task`, `action`, and for live suites exactly one suite), the reboot budget and the authorized
boot entries. A request is accepted only if the kernel-reported peer uid (`SO_PEERCRED`)
equals the owner, the campaign id and manifest digest match, and a grant names that exact
(task, action, suite). The pinned binaries are re-hashed before every launch.

**Capability separation.** A profiling suite runs as the campaign user with exactly
`CAP_BPF`, `CAP_PERFMON` and `CAP_SYS_RESOURCE` (plus `CAP_SYS_NICE` for suites that create
real-time contention), set explicitly: bounding set trimmed, ids dropped with `KEEPCAPS`,
permitted/effective/inheritable/ambient set exactly, `no_new_privs`. Workloads (external
commands, synthetic game processes, fio, vkmark) are spawned with *no* capabilities and
`no_new_privs` regardless of what the parent holds. A controlled contender
(`ExecuteSchedulerBurst`) is pinned and given SCHED_FIFO 50 while privileged, then loses all
privilege before `exec`.

**Bootstrap.** `validation-campaign bootstrap` builds the runner, writes a staged
authorization and runs `runner install` through `sudo -n` (or directly as root). The installer
re-validates everything (staged file owned by the invoking user, owner == invoker, repository,
runner digest, units present) and installs only fixed files to fixed locations. Without root
or passwordless sudo the result is `BLOCKED_PRIVILEGE_BOOTSTRAP`; authentication is never
bypassed. After a successful bootstrap no further interaction is required. `--dry-run` prints
the authorization without changing anything.

**Resume.** The oneshot resume unit restarts the controller after a reboot: only the pinned
`xtask validation-campaign continue --campaign ... --resume`, started after dropping root and
all capabilities; cancelled, idle, corrupt or digest-mismatched campaigns are never resumed.

**State and failure.** Runner state (reboot budget, cancel flag, resume flag) is written
durably *before* a reboot; corrupt state stops every action and never reboots.

Tests that need real root (dropping to another uid) detect it at run time and report that they
were skipped; the rest of the security suite (authorization, protocol, state, planning,
capability hygiene of children, install, resume) runs unprivileged in CI.

## Multi-kernel campaign (phase C)

`kernel-matrix` needs at least `[kernels].required_distinct` (3) *distinct observed* kernel
releases. The stage (`validation_campaign/kernels.rs`) is a pure planner plus a task runner:

1. observe the running kernel with `kernel-compat` (a kernel the loader cannot use is a FAIL
   result whose compatibility is `ObservedIncompatible`; that is a valid, kept observation);
2. while fewer than the required distinct `uname -r` values are observed, choose the next
   authorized, unused boot entry, select it **one-shot**, persist the intent
   (`kernel.pending`) and the waiting state, and only then request the reboot;
3. after the reboot the resume service restarts the controller; it verifies it landed on the
   authorized entry, acknowledges the resume, and observes again;
4. when enough kernels are observed, one last one-shot reboot returns to the original entry.

A duplicate `uname -r` never counts, and a kernel is never inferred from an entry title: the
running kernel's own release is the only authority. Entries that report the running version
are skipped as candidates.

**Stops (`BLOCKED_RESOURCE`, no further reboot):** unknown or unsupported bootloader; landing
on an entry other than the authorized one; the reboot budget (`min(candidates + slack, max)`,
fixed into the root-owned authorization at install time) spent; every candidate tried with too
few kernels; a boot-selection or reboot command failing; corrupt runner state. The campaign
never rewrites the user's NixOS/Gentoo/other boot configuration, downloads kernels, kexecs an
unverified kernel, or substitutes a fixture for a missing kernel.

**Boot adapters** (in the runner, root-only): `systemd-boot` (`bootctl list --json`,
`set-oneshot`) and GRUB (read-only `grub.cfg` parse with nested submenu ids, `grub-reboot`,
which needs `GRUB_DEFAULT=saved`). Discovery runs at install time as root and the candidate
list is frozen into the authorization, overwriting anything the user-staged file carried.
`cancel` drops a pending one-shot selection and never touches the default entry.

**Crash safety.** The reboot request runs only after the waiting state is durable, and the
runner writes its own budget/resume flag before rebooting. A task found `Running` after a
restart is recorded as an interrupted attempt and run again. CI covers the whole machine with
a simulated bootloader and reboots (duplicate kernels, a boot that lands elsewhere, budget
exhaustion, too few entries, refused reboots, restarts); real reboots are evidence, not tests.
