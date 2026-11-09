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
