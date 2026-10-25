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

## Scenarios, corpus and workload drivers (phase D)

**Scenarios** (`docs/validation/scenarios/core-validation.toml`) are predeclared experiments:
a target (`vkmark` light/GPU-heavy or a fixed `fio` read), the interventions applied to it, a
control type, the diagnosis families it exercises and the cause the intervention is known to
produce. The set delivers the manifest floors (66 independent captures; the clean `vkmark`
control is shared by the scheduler, IRQ and GPU families and a `fio` control serves I/O).
Ground truth in a generated `fixture.toml` is the *intervention*; Stutter's diagnosis output
is never used to label, select or filter a capture.

**Capture acceptance.** A capture counts only if target identity (pid + start time) is valid,
the recording validates with zero drops and write errors, every declared intervention was
verified from independent telemetry (`/proc` for the SCHED_FIFO contender, `fio` JSON, the
chosen IRQ's `/proc/interrupts` counter, DRM busy percent), no undeclared intervention was
active, the source/binary fingerprint is unchanged, the capture id is unique, the sanitizer
and `stutter validate` accept it, and a `vkmark` target produced frames. A rejected capture is
appended to `corpus/rejections.ndjson`; the budget is `failed_attempts_per_capture` × needed
captures per scenario and spending it ends the task in `BlockedResource`. Progress is
recomputed from the fixtures on disk, so a killed run resumes where it stopped. Staging
directories (`.staging-*`) are never read as corpus.

**Privilege.** The runner only gains two typed actions: `ExecuteRecording` (the runner derives
every `stutter record` flag from `RecordProbes` and re-checks that the target is the campaign
user's own, unreplaced process) and `ExecuteWorkloadOverhead` (a registered workload whose
command line the runner builds; the campaign supplies only a cycle count). Only the `corpus`
and `frame-score` tasks may record or run the burst, only `overhead-real` may run the workload.

**Tools** come from the pinned Nix shell (`vkmark`, `mangohud`, `fio`, `stress-ng`, `iputils`,
`gh`). A missing tool, display, GPU, or controllable NIC interrupt is `BlockedResource` with the
exact missing capability; nothing is substituted.

**Real-workload overhead.** `overhead-real` rebuilds a Stutter package a frozen number of times
in a campaign-local target directory. The cycle count is calibrated once, stored in campaign
state and bound (with the toolchain, `Cargo.lock` and source commit) into the task fingerprint.

**Frame score.** One recording per manifest degradation level (`vkmark` light with a contender
of `level` ms per 32 ms), each retained and marked complete only after its checks pass, then
evaluated with `--require-frames` semantics. A non-monotonic result is a recorded limitation
for the policy phase, never rewritten.

## Integration, policy and release (phase E)

**Evidence integration.** Once every collection task passed, the `evidence-integration` task
re-verifies each published bundle from disk, derives the index entries from the verified bundles
(never by hand), merges them into `docs/evidence/index.json` (deduplicated by capture id),
copies the sanitized corpus fixtures into `stutter/tests/fixtures/runs`, appends a ledger entry,
runs the offline gates on the branch and lands one pull request: branch `campaign/<id>/<task>`,
push, PR, wait for every required check, merge, return to a clean `main`. An interrupted flow
resumes from the branch's PR; red, pending, skipped, cancelled or neutral required checks never
merge. The hardware-only `live smoke tests` job is the one check GitHub cannot run and is ignored.
`gh auth status` must succeed or the task is `BlockedResource`.

**Calibration and score policy** are deterministic. With every evidence floor met the current
diagnosis thresholds and StutterScore weights are retained; weak behavior at the current
threshold (precision or recall below 0.9, any false High observation or causal confidence) and a
non-monotonic score series are recorded as limitations. An insufficient floor stops the campaign
(`BlockedResource`); nothing is optimized against one corpus. A non-monotonic controlled
series is an `INCONCLUSIVE` bundle at the `evaluation` stage and is kept as an observation.

**Capability policy.** After issue gates #27 #30 #31 #32 #33 #38 are satisfied the manifest's fixed
policy is applied to `stutter/src/capability_tiers.rs` and `docs/CAPABILITIES.md` in a pull
request: `doctor`, `record`, `bench`, `report`, `audit` and `completions_and_man` become
Supported; diagnosis, advisor, tune evaluation and every mutating capability stay Experimental;
`privileged_worker` and `fake_action_provider` stay Internal. The registry change is a pure text
transformation checked against the real registry in tests; `capability-check` and the unit tests
gate the pull request. Mutating capabilities never become Supported in this release.

**Release candidate.** `freeze-rc` syncs `main`, requires it clean, rebuilds the binaries, records
their digests *separately* from the evidence-base digests (collection results stay current),
re-pins the privileged runner to them and makes `continue` re-execute itself from the fresh build.
From then on any repository change invalidates only the exact-RC tasks. `rc-offline` runs
`scripts/release/offline-acceptance.sh` against the candidate and requires the candidate's own
GitHub CI to be green. The four exact-RC live suites publish bundles outside the repository.
`release-package` and `release-verify --require-acceptance` drive the existing implementation.
`install-smoke` unpacks the packaged tarball and runs the packaged binary: version equals the
manifest and the candidate revision, `capabilities` equals the policy, `doctor` runs to a
verdict, `restore`/`audit` respond, and `validate` plus `report` accept the candidate's own
observation recording; the packaged eBPF object is the manifest's ELF object. Recording with
the packaged binary needs privilege that the runner only grants to the pinned build, so that step
is the candidate's `rc-live-observation` suite.

**Tag.** Only after strict verification, every release issue gate, `HEAD == RC`, a clean tree and
green exact-RC CI. The annotated tag `v0.1.0` is created on the candidate and pushed. A tag that
already exists on another commit is `FAILURE_REQUIRES_REVIEW` and is never moved, deleted or
force-pushed; a tag already on the candidate is success.

## Host layouts (NixOS and similar)

The installer chooses one of three modes, and only files root can write decide it:

- **ExternallyManagedPersistent**: the host declares both units and a persistent
  `/var/lib/stutter-validation` (a NixOS module, for instance) and states so in the root-owned
  marker `/etc/stutter-validation/persistent-runner-v1.json`
  (`contract_version = 1`, `state_root`, the two `services`, and the `repository` it
  authorizes). The marker is trusted only if it parses exactly, names this state root, these
  services and this repository, both unit definitions exist, and the marker (after following the
  symlink a declarative host points at an immutable store file) and every directory above it are
  owned by root and not writable by group or others. The installer then installs only the pinned
  runner, authorization and state, leaves the unit definitions to the host, restarts the runner,
  and keeps the reboot budget. No campaign flag or user-supplied file can claim this mode.
- **WritablePersistent**: `/etc/systemd/system` is writable, so the installer writes the units.
- **Transient**: otherwise units go to `/run/systemd/system`, which does not survive a reboot.

Any failed check of the marker falls back to the next mode. In the transient mode the
authorization sets `max_reboots = 0`: the multi-kernel stage reports `BLOCKED_RESOURCE` instead
of rebooting into a machine that could not resume.

`continue --only <task>` runs just that task when it is already ready (dependencies are never bypassed); a task
waiting for a reboot is always resumed before any other pending task, so the machine returning from a
kernel boot continues the kernel stage first.

The resume service orders itself after, and requires, the runner (`After=`/`Requires=`), waits
for the runner's socket (`resume --all --wait-for-runner 120`) before starting the controller,
and uses `KillMode=process` so the detached controller outlives the oneshot unit. Both units are
conditioned on an installed `current/authorization.json`, so an idle machine with no campaign
shows no failed units. In the writable and transient modes the
service keeps `ProtectHome=read-only`; a generated drop-in makes only the repository's
`.llm-runs` and the owner's `~/.cargo` writable. Boot candidates are deduplicated by the kernel
image an entry boots, so dozens of NixOS generations sharing one image are one candidate and a
host with a single kernel stops without any reboot.

The offline gates build into `.llm-runs/gate-target` (`CARGO_TARGET_DIR`), never `target/`: they
use other feature sets, and cargo does not reliably re-link `target/debug/stutter` afterwards, while
the campaign pins the canonical build's digest. The gate cache persists across campaigns.
