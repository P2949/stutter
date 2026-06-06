# stutter fix validation

Status: InvalidExperiment
Fix kind: cpu_affinity_profile
Cause: GameThreadSchedulerDelay
Best profile: baseline-online

## Metric criteria

- diagnostic_raw_score_total: expected >= 5% lower, CI excludes zero; actual actual improvement 52.5% (delta 23762.000score_points); passed=true
- over_5ms: expected >= 10% lower, CI excludes zero; actual actual improvement 0.0% (delta 0.000samples); passed=false
- frame_p99_ms: expected no >5% regression; actual actual improvement 22.2% (delta 10.915ms); passed=true

## Passed criteria

- diagnostic_raw_score_total: >= 5% lower, CI excludes zero
- frame_p99_ms: no >5% regression

## Failed criteria

- over_5ms: CI crosses zero [-1.000, 2.000]
- over_5ms: metric regressed

## Warnings

- comparability Warning kind=candidate-order-not-counterbalanced: two-profile tuning run used the same candidate order for every iteration; profile effect may be confounded with order effect
- comparability Warning profile=baseline-online kind=drop-counters-nonzero: candidate had non-zero drop counters (max=403467)
- comparability Warning profile=kcd1-game-on-1-5-7-11-gamescope-on-0-6 kind=drop-counters-nonzero: candidate had non-zero drop counters (max=428919)

## Next steps

- Repeat the experiment with comparable workload, frame coverage, and drop counters.
