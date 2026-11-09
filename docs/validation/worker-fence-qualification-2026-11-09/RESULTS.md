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

## What this does not establish
* The decoy was refused on the dry-run path, which runs the worker's prepare-entry target check
  (`ServiceTargets::candidate`, the same function `ApplyPrepared` calls at prepare). The precommit re-check
  (`prepared`) and a full prepare, commit, restore exchange against a live worker were not exercised live; they are
  covered by the fake-`/proc` unit tests only.
* Worker restart, fence expiry mid-transaction, rollback before and after expiry, and test-runner SIGKILL were not
  run live.
* The installed production worker (`4f3baa07`) does not know `fence_status` at all, so the smoke correctly refuses to
  mutate against it. Qualifying the installed build still needs the final-source deploy on a free desktop.
