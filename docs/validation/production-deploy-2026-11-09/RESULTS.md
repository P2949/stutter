# W19 production deployment record (2026-11-09)

Stutter source `4f3baa07aface5cdf355095e2de82ece4ebc4c21` (clean CI, workspace Clippy and
`release-check` all PASS, see `../source-ci-2026-11-09-4f3baa07.json`). Release binary SHA-256
`bf79d9c4a4fd3da18a079f1ccddf8c772f9a7c1b12da49e7d698a8c20c20d8a6`, added to the Nix store as
`/nix/store/rvd6k3ww69z9v3r32r6cvwnh1xd68ic1-stutter`.

NixosConf branch `host/stutter-w19-328b3f2b`, commit `250e3cf` (source pin and binary hash moved
together; the earlier commit `4686fb5` pinned `328b3f2b`). `./scripts/validate affected`: PASS
with an exact-commit receipt. The desktop toplevel built; the only closure change was the Stutter
binary and the two unit files. `dry-activate` listed only the two Stutter units; no display
manager, login, kernel or GPU policy change.

| Step | Result |
|---|---|
| First attempt, build of `328b3f2b` | guarded `test` activation succeeded, but `production-smoke` FAILED one of 35 checks: `idle_controller_cpu_is_bounded` 5.63% of one core over 30 s (limit 5%). Evidence: `.llm-runs/production-smoke-20261109-328b3f2b`. Not switched. |
| Diagnosis | eleven cheap sensors read thousands of sysfs/procfs files every tick. Same-flags idle measurement, observe mode: the previous production binary 4.85% of a core, the `328b3f2b` build 5.28%, the fixed build 3.23%. |
| Fix | commit `4f3baa07`: counter and inventory sensors are sampled every 2 to 30 seconds; guards stay at the tick cadence |
| Second attempt, build of `4f3baa07` | guarded `test` activation: running controller (user p2949) and root worker both execute the binary with SHA-256 `bf79d9c4...`, 0 outstanding tokens, display manager untouched. `production-smoke`: **PASS (35 checks)**. Evidence: `.llm-runs/production-smoke-20261109-4f3baa07`. |
| Switch | after the pass only: system profile set and `switch-to-configuration switch`; previous generations retained; services active; 0 outstanding tokens |

Honest limits: the passing idle-CPU check measured **4.97% of one core against the 5% limit**, a thin
margin. The same measurement of the previous production binary under the same host load was
4.85%, and the 2026-11-07 smoke measured 1.87% on a quieter host, so the check is sensitive to host
load rather than showing a regression of this build. The exact-build smoke was run once for the
deployed binary after the failed first attempt, as the plan allows. No reboot, no GPU or LACT
change, and no public GitHub issue, PR, comment or release was created (the NixosConf branch was
pushed without a PR).

## NixosConf `main` promotion (2026-11-09, evening)

`./scripts/validate full` was run on the deployed commit `250e3cf` (branch
`host/stutter-w19-328b3f2b`) and passed: all flake checks evaluated, every named validation
(activation-safety, blank-disk-reconstruction, granular-impermanence, interrupted-recovery,
persistent-identity, recovery-iso, workstation-smoke, desktop and the rest) reported `Validating`
with no failure, exit 0. The local receipt is
`/etc/nixos/.git/local-validation/receipts/250e3cf8be4ef6182585705e81406a493ecc2139/full.json`.
With that receipt in place, `main` (`db578b2`) was fast-forwarded to `250e3cf` (two commits: the
pin to `328b3f2b` and the re-pin to `4f3baa07`), no force, no PR. `main` now equals the commit the
host is running. No reboot, no activation and no service change was involved in the promotion.

## Attempted redeploy of the final source `cc3e0730` (2026-11-09, 16:2x): not switched

Prepared as the plan's final exact-source deploy: the clean-clone release binary
(sha256 `58884a30…b3c4`, in `docs/validation/source-ci-2026-11-09-cc3e0730.json`) was added to the Nix
store, and NixosConf branch `host/stutter-w19-final-cc3e0730` (commit `541841d`, local only) moved the
source pin, `flake.lock` and `binarySha256` together. `./scripts/validate affected` passed with an
exact-commit receipt; the toplevel built; `dry-activate` listed only the two Stutter units; guarded
`switch-to-configuration test` ran both services on the new binary with 0 outstanding tokens.

`production-smoke` then **FAILED 5 of 35 checks** (`exit_while_applied`,
`controller_killed_while_applied`, `workload_discovered_and_mutated_through_the_worker`,
`the_mutation_was_physically_applied`, `the_experiment_resolved`: the controller never resolved the
fixture as a target). Per the plan nothing was switched; the previous generation
(`system-102-link`) was reactivated and verified (binary `bf79d9c4…`, 0 tokens).

**Diagnosis.** The identical smoke run against the previously deployed `4f3baa07` build failed the
same five checks, so this is not a regression of the new binary. The controller runs with
`--auto-focus --focus-source heuristic`; its log shows no target for the whole window. A real game
(Dark Souls Remastered under Proton) was running on the host at 154% CPU, which a heuristic focus
source cannot be expected to ignore in favour of a synthetic fixture. The earlier 35/35 pass (06:29)
had no game running. The smoke needs a host without a real game in the foreground.

**State and next step.** Production stays on `4f3baa07`; `/etc/nixos` is back on
`host/stutter-w19-328b3f2b` (matching the running system) and `main` is unchanged at `250e3cf`. The
prepared branch can be deployed with the same guarded sequence once the host has no game running.
The new code is not reachable from the daemon, so staying on `4f3baa07` changes no installed
behaviour. Raw evidence: `.llm-runs/production-smoke-20261109-cc3e0730` and
`.llm-runs/production-smoke-20261109-4f3baa07-recheck`.

## Final-source deployment: deferred, `BlockedResource` (host not quiescent) (2026-11-09, evening)

A background job (`deploy-when-free.sh`) had been queued to run the guarded activation, smoke and switch
once no game had been detected for five minutes. It was **stopped without having started any
activation** (its log shows only the start of the wait), and the script was disabled. Five quiet
minutes is not proof that the desktop is free for a test activation, and the smoke exercises real
mutation and recovery. No unattended deployment will be scheduled.

The prepared NixosConf branch `host/stutter-w19-final-cc3e0730` (local only, never pushed) pins
`cc3e0730`, which is now superseded by later code; it must not be deployed or pushed as is. Production
stays on `4f3baa07`; nothing since is reachable from the daemon or the bounded service, so no installed
behaviour differs. If the plan's final-source deploy is still wanted, it needs the guarded sequence on a
build of the then-final commit, with the host demonstrably free.

### The smoke can no longer mistake a busy host for a product failure

`xtask production-smoke` now refuses to act unless it can show nothing on the host could be selected
instead of its fixture. Before anything is measured or mutated, before each fixture starts, after each
is ready and again at the end, it checks that no process looks like a game or its runtime (the whole
command line is matched: a Steam library path with either slash, Steam's launcher, Proton,
pressure-vessel, gamescope, wine and its servers, a Windows executable, with the smoke's own fixtures
under its output directory excluded) and that the host is under 40% busy. A competitor at any of those
points makes the whole run `BlockedResource`: the result is written with the competing processes and no
product verdict, and the command exits with a message that says it is environmental. Afterwards the
controller's history is checked: every state change must be on a fixture this run started
(`controller_changed_only_the_disposable_fixtures`). Run against this host while a game was running it
refused before measuring anything (`.llm-runs/production-smoke-20261109-blocked-demo`, 23 matching
processes, 0 checks, production untouched). This does not make the heuristic controller pick the fixture
over a real game; it makes the smoke refuse to run when it could.

## Fence lifecycle and audit tightening (follow-up to the 9 November review)

Code-level, unit-tested only; not live-qualified (no isolated transient worker has run it yet).

* The smoke now refuses every mutation case unless the running controller and worker checks all pass.
* A validation session starts as a deny-all fence (`root_pid` 0 in the fence file, a worker state `DenyAll`), becomes
  the exact fixture while one is active, returns to deny-all between fixtures and is lifted only when the run ends,
  after the tokens have settled. Arming and writing are fallible: no armed fence means no mutation case.
* Worker acknowledgment (authorized by the user's 9 November instruction): a closed, read-only `fence_status` request
  makes the running bounded worker report the fence it reads from its own root-owned state directory (state, exact
  pid and start ticks, account, boot, digest of the bytes read). The smoke asks it after writing the deny-all fence and
  again after fencing each fixture, and refuses unless the report is exactly what was written; a worker that predates
  the request answers with an error, so it can never reach a mutation case. Unit-tested only: the installed worker
  (4f3baa07) does not have it, and no real root worker has answered it yet, so live qualification (decoy refused at
  prepare and precommit) is still outstanding.
* `closeout-report --strict` now also requires the newest committed report to match the ledger pin and the
  current manifest.

## Final-source deployment of `d70f1b52`: prepared, not activated (`BlockedResource`: occupied interactive session) (2026-11-10)

Everything that does not touch the running system was done and verified; the activation stages were not run.

| Stage | Result |
|---|---|
| NixosConf branch | `host/stutter-final-d70f1b52` at `19e35fb` (local to `/etc/nixos`, from `origin/main` `250e3cf`): source pin, `flake.lock` and `binarySha256` moved together to Stutter `d70f1b52` and its release binary `aa8035f3…ebd943c` |
| Binary identity | the clean-clone release build of `d70f1b52`, added to the store with `nix-store --add-fixed` (`/nix/store/jpnksxjy…-stutter`), hash re-read equal to `release_binary_sha256` in the receipt |
| `./scripts/validate affected` | PASS, exact-commit receipt `/etc/nixos/.git/local-validation/receipts/19e35fb2…/affected.json` |
| Toplevel | built: `/nix/store/bx0cz6fbw1x6rjn8n2qag429wp5rbcag-nixos-system-desktop-26.05.20261025.1f607aa` |
| `dry-activate` | would stop and start only `stutter-autotune-bounded@p2949.service` and `stutter-privileged-worker@p2949.service` |
| `switch-to-configuration test`, `production-smoke`, `switch` | **not run** |
| Running system | unchanged: `/run/current-system` and the system profile are still `/nix/store/z5c4c0vs…-nixos-system-desktop-…` (previous generation, Stutter `4f3baa07`, binary `bf79d9c4…`); both units active; 0 outstanding tokens; no reboot |

**Why activation was not run.** No game process was running when this was checked (no Steam, Proton or gamescope
process; load average 2.4), but the machine's interactive session is active (Hyprland, a browser and an editor in use).
The activation restarts both Stutter units and the exact-build smoke then runs a busy fixture and mutation cases for
several minutes. An idle-game observation is not proof that the occupied session may be interrupted, and the smoke's own
heuristic focus source needs a genuinely quiet host. Per the plan this stage is therefore `BlockedResource` and nothing
was scheduled: no watcher, no timer.

**To resume** (when the session is demonstrably free, from `/etc/nixos` on that branch; the previous generation is
`/nix/store/z5c4c0vs05bkg7bmg6w5jsqpd08ab26b-nixos-system-desktop-26.05.20261025.1f607aa`):
`sudo bx0cz6fb…/bin/switch-to-configuration test`; then, from the Stutter checkout,
`xtask production-smoke --binary-sha256 aa8035f343a60cbd9b5b3ba9d96ef53692153f7fa5a545296ec4eea70ebd943c --out <new dir> --user p2949`;
on a full pass only, `nix-env -p /nix/var/nix/profiles/system --set bx0cz6fb…` and `switch-to-configuration switch`
(then `./scripts/validate full` if main promotion requires it); on any failure reactivate the previous generation.
The smoke now asks the running worker to acknowledge the exact fence, so it also qualifies the installed worker.

## Activation attempts of `d70f1b52` on 2026-11-10 (the session was free): rolled back, one real defect found

The user confirmed the desktop was not in use, no game process was running, and the guarded script was run. Each
attempt activated the new generation with `switch-to-configuration test`, ran one exact-build `production-smoke`, and
reactivated the previous generation on anything but a full PASS. The system profile was never changed. After every
attempt `/run/current-system` was the previous generation (`z5c4c0vs…`), both units were active and 0 tokens were
outstanding.

| # | Result | Cause |
|---|---|---|
| 1 | smoke BLOCKED before measuring or mutating anything, rolled back | the host read 53% busy (limit 40%) in the seconds after activation (the activation itself starts `minecraft-server-survival.service`); an environment block, not a product result |
| 2 | 30 of 32 checks ran, then `Error: No such file or directory` while writing evidence, rolled back | harness invocation: the evidence capture runs `rustc`, which is only on `PATH` inside the dev shell. The script now runs the smoke through `nix develop` |
| 3 | full smoke, **FAIL** (6 of 32 checks), rolled back | a real defect, below |

**Run 3.** Everything about the installed build passed: pinned binary hash for controller and worker, immutability,
account, capabilities, no-new-privileges, the worker's acknowledgment of the exact deny-all fence, idle CPU and memory
bounds, nothing outstanding before, and no state change other than on the three fixtures this run started. The mutation
cases failed: the controller never applied anything to the fixture. The history shows 34 faulted decisions, all
`privileged apply failed for autotune candidate 'game-isolate-render': … validation_fence_target_not_fenced`.

**Root cause.** The fixture's focus (the game root) was the fenced process, but the render threads the candidate names
belong to the fixture's own `wineserver` helper, a child process. `check_task` accepted only the exact fenced tgid,
so the fence refused the fixture's own descendants. The earlier isolated worker qualification used a target without
helper processes, so it did not exercise this. The fence failed in the safe direction: nothing was mutated, nothing
outside the fixture was touched, and the controller recorded the refusal and kept running.

**Fix** (`9cfca635`): a task is admitted when its process is the fenced one, or when the fenced process (matched by pid
and start time) is one of its ancestors. A stranger, a stranger's child and a reused parent pid are still refused, and
the fence still only narrows. Test: `a_process_the_fenced_one_started_is_inside_the_fence_and_a_stranger_or_reused_parent_is_not`.
The binary therefore changes, so this build (`aa8035f3…`) is superseded and its deployment is not retried as is.
