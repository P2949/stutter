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
