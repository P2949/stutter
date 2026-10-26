# Local verification

> Verification is repository-owned and local-only. Hosted CI and GitHub-runner-dependent
> verification must not be reintroduced.

Everything that decides whether a change, a release candidate or a validation campaign is correct
runs on the development workstation, from this repository, with the Git remote unavailable and no
`gh` on the path. GitHub may remain a Git remote and an optional publication target
(`release-publish`); it plays no part in whether anything passes.

`xtask` is the authority. All commands run inside the pinned Nix shell
(`nix develop --command cargo run -q -p xtask -- <command>`).

| Command | What it is |
| --- | --- |
| `local-ci-policy` | fails if hosted CI configuration or the hosted-runner protocol is present |
| `verify-fast` | cheap admission gate: policy, `cargo fmt --check`, `no-allow-attrs`, objective docs, capability check, `git diff --check` |
| `ci` | the full deterministic non-root CI, in named stages (`--stage NAME` reruns one); never runs as root |
| `ci-live` | the live smokes (privileged eBPF loader, sleep-tree churn, minimal recording) with non-interactive privilege granted to the finished test processes only; a skipped smoke is a failure |
| `release-check` | builds `stutter --release`, packages it with `release-package`, verifies it with `release-verify`; `--expected-tag vX.Y.Z` also checks the tag against the crate version |
| `ci-all` | `ci`, then `ci-live`, then `release-check` |
| `setup-local-ci` | points `core.hooksPath` at `.githooks` and checks the hooks are executable |

## The stages of `ci`

`policy`, `lint` (lint policy, dependency hygiene), `build` (fmt, build, clippy and every workspace
test through `scripts/smoke/build.sh`, plus the `xtask` and runner members), `objective`,
`smoke` (the offline workflow smokes), `campaign` (validation-campaign state machine, runner
security, kernel stage, scenarios, landing, policy and release), `daemon-safety` and `schema`.
`xtask ci --stage campaign` reruns one without editing code; the default is every stage.

## Hooks

`.githooks/pre-commit` runs `verify-fast` and `.githooks/pre-push` runs `ci`. They only call
`xtask`; run `xtask setup-local-ci` once per checkout.

## Live and release validation

The Core Validation and mutation-graduation campaigns (`xtask validation-campaign`,
`xtask mutation-campaign`) and the persistent privileged runner are unchanged: they are the
repository's real-kernel validation. `ci-live` only preserves the old smoke contract and does not
replace them. Exact-RC acceptance runs `ci` in a fresh local checkout and writes a local receipt.

## Why

A verification result that depends on someone else's runner can be lost, delayed or rewritten, and
cannot cover privileged and real-kernel behaviour. The policy scanner (`local-ci-policy`) is the
enforcement: it rejects `.github/workflows`, `.github/actions`, other hosted-CI roots and the
runner protocol (`GITHUB_OUTPUT`, `github.event`, `uses: owner/action@ref` ...) in executable and
configuration files, while allowing documentation like this page to explain them.
