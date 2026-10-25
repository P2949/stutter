# Mutation graduation campaign

`cargo run -p xtask -- mutation-campaign` graduates one autonomous operation at a time. It is
generic: [`docs/validation/mutation-graduation.toml`](validation/mutation-graduation.toml) has
one section per operation, and the stage order is fixed in code. An operation ends exactly one
way: `Supported`, `DeferredEvidence` or `ExcludedFromAutonomy`. A campaign never forces
`Supported`.

State lives under `.llm-runs/mutation-validation/<campaign-id>/`; it is pinned to a clean `main`
commit, the pinned `stutter` and `xtask` binaries and the manifest digest. Any change of those
invalidates the campaign (`Invalidated`), and a successor campaign is started at the new commit.
Attempts are append-only, state writes are atomic, and published evidence goes through the Core
Validation pipeline (export, `evidence-verify`, index entry, atomic rename, receipt).

```bash
cargo run -p xtask -- mutation-campaign init cpu_affinity_profile
cargo run -p xtask -- mutation-campaign status --json
cargo run -p xtask -- mutation-campaign continue
cargo run -p xtask -- mutation-campaign verify --strict
cargo run -p xtask -- mutation-campaign report
cargo run -p xtask -- mutation-campaign cancel
```

`continue` exits 0 when complete, 11 for `BlockedResource`, 12 for `BlockedPrivilege` and 13 when
a source change is required (the Core Validation exit codes).

## Stages

| Stage | What it proves |
| --- | --- |
| `preflight` | The operation is known, its objective is `Validating`, nothing registers it yet. |
| `mechanical_apply_verify_rollback` | The existing live suite: real apply, verification, rollback. |
| `fault_matrix` | The recovery suite: the 11 checkpoint classes plus target exit, PID/TID reuse, external change, manager death and shutdown restoration. |
| `suspend_resume` | The machine really suspends and the action survives. |
| `benefit_calibration` | Matched counterbalanced AB/BA pairs through the real control path; thresholds chosen on the calibration part and frozen. |
| `benefit_holdout` | One evaluation of the frozen thresholds: zero false keeps on neutral and regression pairs, and the required keep rate on beneficial pairs. |
| `kernel_matrix` | Only for operations whose behaviour depends on a kernel interface; skipping it needs a reasoned `skip` entry. |
| `evidence_integrate` | Every bundle re-verifies at the campaign's source commit. |
| `tier_decision` | The disposition, derived from the recorded stage outcomes. |

## Benefit analysis

Each pair records the objective's own baseline and candidate populations
(`RunnableSummary`: p99, samples, over-5 ms count), whether the mechanism was physically
observed, and whether guardrails held. The analysis is pure and recomputable from
`benefit/pairs.jsonl`. Selection prefers, in order: zero false keeps on neutral and regression
pairs; most beneficial keeps; most regressions reverted; smallest change from the product
thresholds. Underpowered results top up from 15 to at most 30 pairs per scenario; a calibration
that still cannot separate the scenarios is a `ValidationFailure`, recorded as
`DeferredEvidence` (the objective stays `Validating`, no fallback to `StutterScore`).

A mechanism observation can only withhold a keep, never create one: the primary workload
objective decides.

## Privilege and reboot resume

A mutation campaign authorizes itself with the privileged validation runner; the
authorization carries a typed `controller` (`core_validation` or `mutation_graduation`). The
runner derives the campaign directory (`.llm-runs/validation` or `.llm-runs/mutation-validation`)
and the reboot-resume command (`validation-campaign continue` or `mutation-campaign continue`,
followed by `--campaign <dir> --resume`) from that kind alone; there is no caller-supplied argv.
The grants name each stage's live suite, the recording and scheduler-burst actions the benefit
stages use, and campaign status/resume/cancel. A mutation campaign is never granted a reboot or a
boot-entry change.
