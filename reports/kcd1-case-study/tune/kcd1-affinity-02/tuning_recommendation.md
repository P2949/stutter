# stutter tuning recommendation

Verdict: NeedsRetest
Best profile: baseline-online
Confidence: Medium

## Summary

Profile 'baseline-online' is currently best, but the result is not strong enough to trust without another run.

## Why

- formal A/B diagnostic_raw_score_total: baseline_median=21533.000 tuned_median=21533.000 improvement=0.000 effect_size=0.00 95% CI [-11461.000, 11461.000] enough_samples=true significant=false power=30 runs/side
- formal A/B over_5ms: baseline_median=0.000 tuned_median=0.000 improvement=0.000 effect_size=0.00 95% CI [-1.000, 1.000] enough_samples=true significant=false power=n/a
- formal A/B frame_p99_ms: baseline_median=38.338 tuned_median=38.338 improvement=0.000 effect_size=0.00 95% CI [-6.350, 6.350] enough_samples=true significant=false power=18 runs/side
- formal A/B frame_over_16ms: baseline_median=6796.000 tuned_median=6796.000 improvement=0.000 effect_size=0.00 95% CI [-1593.000, 1593.000] enough_samples=true significant=false power=24 runs/side
- formal A/B frame_over_33ms: baseline_median=433.000 tuned_median=433.000 improvement=0.000 effect_size=0.00 95% CI [-367.000, 367.000] enough_samples=true significant=false power=30 runs/side
- formal A/B frame_over_50ms: baseline_median=8.000 tuned_median=8.000 improvement=0.000 effect_size=0.00 95% CI [-16.000, 16.000] enough_samples=true significant=false power=30 runs/side
- formal A/B max_latency_ns: baseline_median=4152319.000 tuned_median=4152319.000 improvement=0.000 effect_size=0.00 95% CI [-1089458.000, 1089458.000] enough_samples=true significant=false power=26 runs/side
- Median score delta versus baseline 'baseline-online' is 0 (0.0%, effect_size=0.00, noise_ratio=0.32)
- over_5ms delta versus 'baseline-online' is 0 (effect_size=0.00, noise_ratio=n/a)
- frame p99 delta versus 'baseline-online' is 0us (effect_size=0.00, noise_ratio=0.04)
- best profile 'baseline-online' had 5 valid run(s) and 0 invalid run(s)
- best median score=21533 IQR=6829 worst=32994

## Warnings

- best profile score IQR is non-zero
- comparability Warning profile=baseline-online kind=drop-counters-nonzero: candidate had non-zero drop counters (max=403467)
- comparability Warning profile=kcd1-game-on-1-5-7-11-gamescope-on-0-6 kind=drop-counters-nonzero: candidate had non-zero drop counters (max=428919)
- best profile score IQR is non-zero (6829)
- diagnostic_raw_score_total: estimated 30 runs per side needed to detect 10% improvement at current noise
- diagnostic_raw_score_total: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- diagnostic_raw_score_total: baseline distribution is noisy: noise_ratio=0.32
- diagnostic_raw_score_total: tuned distribution is noisy: noise_ratio=0.32
- over_5ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_p99_ms: estimated 18 runs per side needed to detect 10% improvement at current noise
- frame_p99_ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_16ms: estimated 24 runs per side needed to detect 10% improvement at current noise
- frame_over_16ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_33ms: estimated 30 runs per side needed to detect 10% improvement at current noise
- frame_over_33ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_33ms: baseline distribution is noisy: noise_ratio=0.27
- frame_over_33ms: tuned distribution is noisy: noise_ratio=0.27
- frame_over_50ms: estimated 30 runs per side needed to detect 10% improvement at current noise
- frame_over_50ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_50ms: baseline distribution is noisy: noise_ratio=0.38
- frame_over_50ms: tuned distribution is noisy: noise_ratio=0.38
- max_latency_ns: estimated 26 runs per side needed to detect 10% improvement at current noise
- max_latency_ns: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- close score margin versus 'baseline-online': delta_abs=0 threshold=1076
- formal diagnostic score comparison is underpowered, insignificant, or not positive; recommendation requires retest

## Next steps

- Rerun tune with the same workload and at least 5 runs.
