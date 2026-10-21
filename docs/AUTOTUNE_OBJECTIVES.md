# Auto-tune Objectives

Generated from `ObjectiveKind::descriptor()`. Regenerate with
`cargo run -p xtask -- objective-docs --write`; CI uses `--check`.

Signal quality below describes the declared primary contract. Actual observation
quality can be lower or missing. Autonomous eligibility remains subject to the
separate experimental action, policy and safety gates; it does not enable mode
or grant permission. No objective has graduated to Supported.

| Objective | Primary metric | Quality | Required signals | Fallback | Guardrails | Autonomous eligible | Maturity |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `StutterScore` | `normalized_score_per_sample` | derived | scored_samples, score_total | None | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, frame_p99, over_5ms_rate | true | Experimental |
| `GameFramePacing` | `frame_p99_ms` | derived | frame_p99_ms_or_explicit_score_fallback | NormalizedScoreWhenMissingOrUnchanged | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, foreground_over_5ms_rate, foreground_frame_pacing, generic_score | true | Experimental |
| `GameRunnableLatency` | `runnable_p99_ns` | direct | complete_target_scheduler_histogram, stable_target_process_identity, at_least_1000_runnable_samples | None | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, runnable_over_5ms_rate, frame_p99, foreground_over_5ms_rate, foreground_frame_pacing | true | Validating |
| `DesktopInteractivity` | `foreground_over_5ms_per_sample` | derived | foreground_latency_or_explicit_score_fallback | NormalizedScoreWhenMissingOrUnchanged | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, generic_score | true | Experimental |
| `BrowserInteractivity` | `foreground_over_5ms_per_sample` | approximate | foreground_latency_or_explicit_score_fallback | NormalizedScoreWhenMissingOrUnchanged | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, generic_score | true | Experimental |
| `CompileThroughputWithForegroundProtection` | `compile_fixed_work_elapsed_ns` | direct | successful_identical_fixed_work, stable_target_process_identity, ordered_distinct_executions, foreground_latency | None | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, foreground_over_5ms_rate, foreground_frame_pacing, observed_memory_pressure | true | Validating |
| `IoLatency` | `block_io_overlap_count_then_worst_latency_ns` | direct | block_io_overlap_count, block_io_worst_latency_ns | None | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, generic_score | true | Experimental |
| `IrqOverlapReduction` | `irq_overlap_count_then_worst_overlap_ns` | direct | irq_overlap_count, irq_worst_overlap_ns | None | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, generic_score | true | Experimental |
| `ThermalRecovery` | `thermal_degraded_then_throttle_count` | derived | thermal_degraded, thermal_throttle_count | None | target_presence, experiment_data_quality, valid_nonzero_scored_samples, thermal_degradation, thermal_throttle_count, cpu_power_limit, gpu_power_limit, generic_score | true | Experimental |

## Limitations

- `StutterScore`: Ranking proxy within comparable experiments; weights are not empirically calibrated.
- `GameFramePacing`: Frame source quality varies; unchanged or missing primary uses explicit score fallback.
- `GameRunnableLatency`: Merged latency-sensitive target-game histograms; finite buckets and provisional effect thresholds; live acceptance pending.
- `DesktopInteractivity`: Existing derived foreground signal; unchanged or missing primary uses score fallback.
- `BrowserInteractivity`: Uses the existing foreground approximation, without a browser-specific direct primary measurement.
- `CompileThroughputWithForegroundProtection`: Cache/toolchain/environment policy is harness-declared; foreground protection is derived; provisional 10 percent threshold and live acceptance pending.
- `IoLatency`: Overlap evidence is correlation, not proof of causal storage delay; source trust varies.
- `IrqOverlapReduction`: Overlap evidence is correlation, not proof of causal interrupt delay.
- `ThermalRecovery`: Thermal evidence depends on available platform telemetry.

See [runnable latency](RUNNABLE_LATENCY_OBJECTIVE.md),
[fixed-work compile completion](FIXED_WORK_COMPILE.md), and the
[behavioral matrix](OBJECTIVE_CONTRACT_MATRIX.md) for detailed contracts and
validation boundaries. Historical `compile_progress_*` fields deserialize as
activity; legacy Direct labels never authorize a throughput improvement.
