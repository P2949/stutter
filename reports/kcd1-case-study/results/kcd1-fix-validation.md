# stutter fix validation

Status: InvalidExperiment
Fix kind: cpu_affinity_profile
Cause: GameThreadSchedulerDelay
Best profile: none

## Metric criteria

- diagnostic_raw_score_total: expected >= 5% lower, CI excludes zero; actual actual improvement 100.0% (delta 45295.000score_points); passed=false
- over_5ms: expected >= 10% lower, CI excludes zero; actual actual improvement 0.0% (delta 0.000samples); passed=false
- frame_p99_ms: expected no >5% regression; actual actual improvement 100.0% (delta 49.252ms); passed=false

## Passed criteria

- none

## Failed criteria

- diagnostic_raw_score_total: not enough samples for fix validation
- diagnostic_raw_score_total: required CI is missing
- over_5ms: not enough samples for fix validation
- over_5ms: required CI is missing
- over_5ms: metric regressed
- frame_p99_ms: not enough samples for fix validation

## Warnings

- formal A/B evidence is underpowered; do not count this as proof
- comparability Warning profile=kcd1-game-on-1-5-7-11-gamescope-on-0-6 kind=drop-counters-nonzero: candidate had non-zero drop counters (max=472447)

## Next steps

- Repeat the experiment with comparable workload, frame coverage, and drop counters.
