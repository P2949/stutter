# KCD1 Follow-Up Hypothesis

This file is for the next serious KCD1/FYP run. It is not a retroactive decision rule for the archived `kcd1-affinity-02` run.

## Hypothesis

The tuned CPU-affinity/process-placement profile will reduce `diagnostic_raw_score_total` by at least 10% versus `baseline-online` on the selected KCD1 Rattay route.

## Primary Metric

`diagnostic_raw_score_total`, lower is better.

## Secondary Metrics

- `frame_p99_ms`
- `frame_over_16ms`
- `frame_over_33ms`
- `frame_over_50ms`
- scheduler `over_5ms`
- max scheduler latency

## Decision Rule

The tuned profile is considered validated only if:

1. the primary metric improves,
2. the bootstrap confidence interval excludes zero or the repeated-run evidence is otherwise explicitly justified,
3. no major secondary frame-pacing metric regresses materially,
4. data-quality warnings are absent or explained,
5. the candidate order is counterbalanced.

## Rejection Rule

If the tuned profile is worse on the primary metric across most or all paired runs, the profile is not recommended even if one secondary frame metric improves.

## Candidate Order

Use counterbalanced profile order for the next run, for example:

```text
iteration 1: baseline, tuned
iteration 2: tuned, baseline
iteration 3: baseline, tuned
iteration 4: tuned, baseline
```

Report paired per-iteration deltas, profile-level medians/distributions, and bootstrap confidence intervals.
