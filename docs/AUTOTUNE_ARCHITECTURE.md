# Auto-tune Architecture

`stutter` does not currently provide a broad autonomous optimizer. This document defines the controller contract that any future limited auto-tuner must follow before it is allowed to change system state autonomously.

See [FULL_SYSTEM_WATCHER_ARCHITECTURE.md](FULL_SYSTEM_WATCHER_ARCHITECTURE.md)
for the always-on watcher loop, provider boundary, and mode compatibility
matrix.

## Controller State Machine

```text
Disabled
  ↓
Observing
  ↓ enough valid data
CandidatePlanning
  ↓ candidate passes safety gate
Applying
  ↓ verify ok
Measuring
  ↓ improved enough       ↓ worse/inconclusive
Keeping                 Reverting
  ↓                     ↓
Cooldown  ←─────────────┘
  ↓
Observing
```

## Rules

```text
- Observe-only is the default.
- Apply mode must be explicitly enabled.
- HighRisk actions require explicit config and must never be default.
- The controller may only run one experiment at a time.
- A failed verify immediately triggers rollback.
- Any action without rollback is ineligible for autonomous mode.
- Inconclusive data reverts unless policy says “keep baseline”.
- Data-quality failure blocks action.
```

## Contract

The controller starts in `Disabled` unless the user explicitly enables apply mode. In normal operation, `Observing` collects evidence without changing system state. `CandidatePlanning` may only select one candidate experiment after enough valid data exists. `Applying` is allowed only after the candidate passes the safety gate. `Measuring` compares the changed state against the baseline. `Keeping` is allowed only when the result improved enough under the configured policy. `Reverting` is mandatory for failed verification, worse results, inconclusive results without an explicit keep-baseline policy, and any action whose data quality becomes untrustworthy. `Cooldown` prevents immediate repeated changes and then returns the controller to `Observing`.

## Experiment Comparison

Experiment comparison uses normalized score rates for keep/revert decisions so unequal baseline and candidate windows are comparable. The primary stutter metric is:

```text
score_per_sample = score.total / scored_samples
```

The over-5ms regression guard is also evaluated as a rate per 1,000 scored samples. Raw score totals and raw over-5ms counts are retained in diagnostics only; they are useful for debugging but must not decide whether an experiment improved or regressed.

This contract is intentionally narrower than a general optimizer. Future implementation must preserve the existing separation between observation, recommendation, and system-changing actions.
