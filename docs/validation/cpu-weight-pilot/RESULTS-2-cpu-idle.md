# cgroup `cpu.idle` pilot: result (preregistered 20 percent floor not met)

Raw rows: [results-2-cpu-idle.txt](results-2-cpu-idle.txt). Same harness as the first pilot; the control is the mean
of a block's two `cpu.idle` 0 runs and the treatment the mean of its two `cpu.idle` 1 runs on the background cgroup.

| Block | Control p99 (ms) | Control change | Treated p99 | p99 improvement | Mean-frame improvement | Compile slowdown |
|---|---|---|---|---|---|---|
| 1 | 10.245 / 9.885 | -3.6 % | 9.021 | 10.4 % | 11.1 % | 7.5 % |
| 2 | 9.300 / 9.723 | 4.4 % | 8.869 | 6.8 % | 10.8 % | 11.1 % |
| 3 | 9.594 / 9.671 | 0.8 % | 8.808 | 8.6 % | 12.9 % | 3.1 % |
| 4 | 9.717 / 9.999 | 2.9 % | 8.911 | 9.6 % | 12.0 % | 4.7 % |

Median p99 improvement 9.1 percent (direction consistent, 4 of 4), median mean-frame improvement 11.5 percent,
median compile slowdown 6.1 percent. The compile cost is within the 15 percent limit and the median clears twice the
largest control change (8.8), but the preregistered 20 percent floor is not met: `DeferredEvidence` for this metric.

Harness note: resetting the background `cpu.weight` right after an idle run printed `Invalid argument` (the kernel rejects
weight writes on an idle cgroup); the next `cpu.idle` write restored the default weight, and the affected controls match
the others (for example 9.671 ms), so the data stands.

Both mechanisms land at the same ceiling (about 9 to 10 percent p99, 11 to 13 percent mean frame time). Frame time
includes the loop's own 6.1 ms of CPU work, so it understates a scheduling effect. The controller's foreground
objective, `GameRunnableLatency`, keeps a change only at a 12.5 percent wake-to-run latency improvement, so the
relevant question is the wake-latency effect, which a third pilot measures directly.
