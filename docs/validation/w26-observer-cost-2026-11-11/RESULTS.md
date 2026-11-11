# W26: observer cost of the installed services (2026-11-11)

Installed build: Stutter `9cfca635` (binary `6922c7c6…`), generation `2m2lpklm…`. Five consecutive 60 s intervals, the two
service processes together (`stutter-autotune-bounded@p2949`, `stutter-privileged-worker@p2949`): CPU from `utime+stime`
ticks, RSS from `VmRSS`, open FDs, and write volume from `write_bytes` in `/proc/<pid>/io` (via sudo for the root-owned
worker). No game running (checked before every interval); load average 0.01–0.61. Raw data: `result.tsv`; method: `measure.sh`.

| interval | CPU % of one core | RSS (kB) | FDs | writes (B/s) |
|---|---|---|---|---|
| 1 | 2.600 | 63804 | 55 | 1979 |
| 2 | 2.250 | 63804 | 55 | 4232 |
| 3 | 2.683 | 63804 | 55 | 1911 |
| 4 | 2.883 | 63804 | 55 | 1911 |
| 5 | 2.567 | 63804 | 55 | 4164 |

CPU mean 2.60 % (range 2.25–2.88 %), about half of the 5 % ceiling; RSS and FD count are constant (no growth, no leak over
5 min); write volume is 2–4 kB/s. This is a stable budget on a quiet host. It is higher than the 1.99 % measured on the older
`4f3baa07` build under a busier desktop (`../idle-cpu-2026-11-09.md`); the two builds differ and the conditions differ, so no
per-sensor attribution is claimed, and no cadence or threshold was changed. If a later reading approaches the ceiling, the next
step is per-sensor attribution through the sensor hub.

Not covered here: trace drops (no eBPF tracing is enabled in this service mode), and correlation of perf/eBPF counters with a
direct KPI, which needs the installed `perf` workflow and is a separate slice.
