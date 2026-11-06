# cgroup `cpu.weight` pilot: result (preregistered rule not met)

Raw rows: [results.txt](results.txt). Sixteen runs in four balanced blocks, as preregistered. Each block's
control is the mean of its two `100:100` runs. Foreground: `fgload`, 6.1 ms of CPU work per 16.667 ms frame,
pinned with a cold `cargo build -p xtask -j4` to CPUs 0-3 in sibling cgroups of a delegated user scope.

| Block | Control p99 (ms) | Control change | w400 p99 improvement | w1000 p99 improvement | Compile slowdown w400 / w1000 |
|---|---|---|---|---|---|
| 1 | 9.599 / 9.911 | 3.2 % | 8.3 % | 9.0 % | 1.6 % / 1.1 % |
| 2 | 9.732 / 9.903 | 1.7 % | 6.5 % | 9.8 % | 3.5 % / 6.0 % |
| 3 | 10.002 / 9.504 | 5.1 % | 8.0 % | 9.3 % | -1.9 % / -3.8 % |
| 4 | 10.092 / 10.340 | 2.4 % | 9.8 % | 13.0 % | 2.3 % / 2.0 % |

Foreground mean frame time also fell from about 6.9 ms (control) to 6.06 ms at weight 1000. Compile elapsed was
about 91 to 99 s in every condition.

Preregistered rule: at least 20 percent p99 improvement in at least 3 of 4 blocks, a median at least twice the
largest control change (5.1 percent, so 10.2), and a median compile slowdown of at most 15 percent.

- Weight 1000: direction is consistent (4 of 4 blocks better), median improvement 9.5 percent, median compile
  slowdown 1.6 percent. The compile cost passes; the 20 percent floor and the twice-noise bar (10.2) are not met.
- Weight 400: median improvement 8.2 percent, below the floor.

Result: `cpu.weight` is a real but small, nearly free lever on this workload. It does not clear the preregistered
bar, so no operation is built on it. It is capped near 10 percent because the background threads still preempt
at wakeup; the remaining gap to the 6.1 ms the loop takes alone is not a weight effect. A different mechanism
(`cpu.idle` on the background cgroup) is a substantively new hypothesis and is preregistered separately.
