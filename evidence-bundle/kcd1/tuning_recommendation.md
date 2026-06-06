# stutter tuning recommendation

Verdict: NeedsRetest
Best profile: baseline-online
Confidence: Low

## Summary

Baseline profile 'baseline-online' remained best, but the result is not strong enough to trust without another run.

## Why

- formal A/B diagnostic_raw_score_total: comparison_median=38806.000 selected_median=21533.000 selected_improvement=17273.000 effect_size=0.86 95% CI [5644.000, 76928.000] enough_samples=true significant=true power=30 runs/side
- formal A/B over_5ms: comparison_median=0.000 selected_median=0.000 selected_improvement=0.000 effect_size=0.00 95% CI [-1.000, 3.000] enough_samples=true significant=false power=n/a
- formal A/B frame_p99_ms: comparison_median=37.798 selected_median=38.338 selected_improvement=-0.539 effect_size=-0.10 95% CI [-6.690, 10.419] enough_samples=true significant=false power=30 runs/side
- formal A/B frame_over_16ms: comparison_median=6546.000 selected_median=6796.000 selected_improvement=-250.000 effect_size=-0.24 95% CI [-2698.000, 1419.000] enough_samples=true significant=false power=30 runs/side
- formal A/B frame_over_33ms: comparison_median=445.000 selected_median=433.000 selected_improvement=12.000 effect_size=0.01 95% CI [-389.000, 2783.000] enough_samples=true significant=false power=30 runs/side
- formal A/B frame_over_50ms: comparison_median=6.000 selected_median=8.000 selected_improvement=-2.000 effect_size=-0.18 95% CI [-18.000, 28.000] enough_samples=true significant=false power=30 runs/side
- formal A/B max_latency_ns: comparison_median=4866891.000 selected_median=4152319.000 selected_improvement=714572.000 effect_size=1.36 95% CI [-407583.000, 1705388.000] enough_samples=true significant=false power=19 runs/side
- Median score delta versus best-non-baseline 'kcd1-game-on-1-5-7-11-gamescope-on-0-6' is -17273 (-44.5%, effect_size=0.86, noise_ratio=0.32)
- over_5ms delta versus 'kcd1-game-on-1-5-7-11-gamescope-on-0-6' is 0 (effect_size=0.00, noise_ratio=n/a)
- frame p99 delta versus 'kcd1-game-on-1-5-7-11-gamescope-on-0-6' is 539us (effect_size=0.10, noise_ratio=0.04)
- best profile 'baseline-online' had 5 valid run(s) and 0 invalid run(s)
- best median score=21533 IQR=6829 worst=32994

## Warnings

- best profile score IQR is non-zero
- candidate order was not counterbalanced; ranking confidence lowered
- comparability Warning kind=candidate-order-not-counterbalanced: two-profile tuning run used the same candidate order for every iteration; profile effect may be confounded with order effect
- comparability Warning profile=baseline-online kind=drop-counters-nonzero: candidate had non-zero drop counters (max=403467)
- comparability Warning profile=kcd1-game-on-1-5-7-11-gamescope-on-0-6 kind=drop-counters-nonzero: candidate had non-zero drop counters (max=428919)
- best profile score IQR is non-zero (6829)
- best profile is the baseline profile 'baseline-online'; comparing against the best valid non-baseline candidate
- diagnostic_raw_score_total: comparison distribution is noisy: noise_ratio=0.26
- diagnostic_raw_score_total: selected distribution is noisy: noise_ratio=0.32
- over_5ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_p99_ms: estimated 30 runs per side needed to detect 10% improvement at current noise
- frame_p99_ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_16ms: estimated 30 runs per side needed to detect 10% improvement at current noise
- frame_over_16ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_33ms: estimated 30 runs per side needed to detect 10% improvement at current noise
- frame_over_33ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_33ms: comparison distribution is noisy: noise_ratio=0.57
- frame_over_33ms: selected distribution is noisy: noise_ratio=0.27
- frame_over_50ms: estimated 30 runs per side needed to detect 10% improvement at current noise
- frame_over_50ms: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant
- frame_over_50ms: comparison distribution is noisy: noise_ratio=0.83
- frame_over_50ms: selected distribution is noisy: noise_ratio=0.38
- max_latency_ns: estimated 19 runs per side needed to detect 10% improvement at current noise
- max_latency_ns: bootstrap 95% CI crosses zero; A/B improvement is not statistically significant

## Next steps

- Rerun tune with the same workload and at least 5 runs.
