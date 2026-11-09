# Worker validation-fence qualification against a real, isolated, transient root worker (2026-11-09)

**Scope: isolated real mechanism (G2-level for the fence only). It is not an installed-binary proof and not a
deployment: production stayed on `4f3baa07`, its units were never restarted or touched, and the user's game was running.**

## What ran
* Binary: the release build of committed source `71d90506` from the exact-source receipt
  ([source-ci-2026-11-09-71d90506.json](../source-ci-2026-11-09-71d90506.json)), sha256 prefix `60e80145ab1144fc`
  (checked equal to the receipt's `release_binary_sha256` before use).
* A separate root worker, not the production unit: `sudo -n env HOME=/var/lib/stutter-qualify/home
  STUTTER_SERVICE_CONFIG=/var/lib/stutter-qualify/config.toml ... privileged-worker --socket
  /run/stutter-qualify/worker.sock --controller-user p2949`, at nice 19. Its own root-owned state directory, config
  and socket; the production worker's state directory (`/var/lib/stutter/worker-p2949`) and socket were not used.
* Two disposable `sleep` processes of the same account: the fixture the fence names and a decoy it does not.
* Nothing was mutated: only `fence_status` requests and dry runs were sent. No token was issued.
* Afterwards: the worker was stopped with its own `shutdown` request and `/var/lib/stutter-qualify` and
  `/run/stutter-qualify` were removed. `stutter-privileged-worker@p2949` and `stutter-autotune-bounded@p2949`
  stayed `active` throughout.

## Results (real socket protocol)
| fence file | worker's `fence_status` |
|---|---|
| none | `absent`, no digest |
| deny-all (`root_pid` 0) | `deny_all`, digest `eaae638a...` = `sha256sum` of the file |
| pid 4242 / ticks 99 | `active`, `root_pid` 4242, `start_ticks` 99, account 1000, this boot |
| expired | `refusing`: `validation_fence_expired_remove_it` |
| wrong boot id | `refusing`: `validation_fence_wrong_boot` |
| mode 0664 (not trustworthy) | `refusing`: `validation_fence_untrusted`, no digest claimed |

Decoy and fixture (`live_fence_tests.rs::a_real_worker_refuses_a_decoy_the_fence_does_not_name`, run with
`--ignored` and the three `STUTTER_QUALIFY_*` variables):
* Fence naming the fixture (pid and start ticks read from `/proc`): `fence_status` reported `active` for exactly it.
* Dry run for the **decoy**: refused, `privileged_worker_request_failed: validation_fence_target_not_fenced`.
* Dry run for the **fixture**: passed the fence and the target checks; refused only afterwards by an ordinary check
  (`profile 'live-fence' has no rules`), which is the test's empty profile, not the fence.

## Second run: the full two-phase exchange (same binary, fresh transient worker)
Harness: `live_fence_tests.rs::a_real_worker_applies_restores_and_refuses_at_precommit_under_the_fence` (ignored;
`STUTTER_QUALIFY_SOCKET` and `STUTTER_QUALIFY_HOME` select the transient worker). It spawns two disposable `sleep`
processes of the same account, fences one (written as root with `sudo -n tee`, as the smoke does) and drives real
`ApplyPrepared` / commit / `Rollback` / `SettleIssuedTokens` requests. Binary: the release build of `2f9abb20`
(sha256 prefix `fcad15d9f81e82ac`, equal to its receipt's `release_binary_sha256`; the executable code is that of
`71d90506`). Results, all passing:
* **Decoy at prepare:** refused with `validation_fence_target_not_fenced`; the commit hook never ran; the decoy's
  `Cpus_allowed_list` was unchanged.
* **Fixture:** prepared and committed; its allowed CPUs were read back from `/proc` as changed to `0`; the rollback
  restored the original list exactly; the decoy was never touched.
* **Precommit, fence moved:** the fence was rewritten to name the decoy between the prepare and the commit; the
  worker refused the commit, nothing was written to the fixture, and the settle answer reported 0 outstanding.
* **Precommit, fence expired:** the fence was rewritten as already expired between prepare and commit; the commit was
  refused, nothing was written, 0 outstanding.
* Afterwards the worker was stopped with its own `shutdown` request, its directories were removed, no test process
  remained, and the production worker and controller units stayed `active` and untouched throughout.

## What this still does not establish
* Worker restart with an outstanding token under a fence, and a runner SIGKILL mid-session, were not run live (the
  restart and owner-death recovery paths have their own earlier live evidence without the fence).
* The installed production worker (`4f3baa07`) does not know `fence_status`, so the smoke correctly refuses to mutate
  against it. Qualifying the installed build needs the final-source deploy on a free desktop.
