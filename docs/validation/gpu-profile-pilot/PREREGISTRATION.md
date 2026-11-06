# LACT GPU-profile feasibility pilot: preregistration

Written and committed before any Daily-versus-Gaming comparison data exists.

## Question

Does switching the existing LACT profile from `Daily` (`power_profile_mode_index: 2`, `POWER_SAVING`)
to `Gaming` (`power_profile_mode_index: 1`, `3D_FULL_SCREEN`) measurably and repeatably improve a
GPU-bound workload? The two profiles share the 238 W power cap, the -150 mV offset, the clock
configuration and the fan curve, so the firmware workload profile is the only intended difference.
This is only a feasibility pilot: it cannot graduate any autonomy, and it does not touch the 50 percent
`GameFramePacing` threshold, whose calibration is scoped to a CPU-bound cohort.

## Ownership and safety (hard stops)

- The profile is changed only with `lact cli profile set <name>`. The GPU sysfs interface is only read.
- Preconditions, checked before the first change: `lactd` active; `gamemodelist` empty (no GameMode
  client); the current profile is `Daily`; GPU edge temperature below 65 C; no other `lact` process.
- Before the first change: the original profile and the full affected readback (profile name,
  `pp_power_profile_mode`, `power_dpm_force_performance_level`, power cap, voltage offset) are recorded;
  an exit trap restores the original profile; and an independent restore timer
  (`systemd-run --user --on-active=1500 lact cli profile set Daily`) is armed so a dead harness still
  restores it. The timer is cancelled on normal completion.
- Abort immediately, restore and record the abort if: a profile other than the one just set is read
  back, a GameMode client appears, the GPU edge temperature reaches 80 C, or any workload exits early.
- Restoration is verified through `lact cli profile get` and the `pp_power_profile_mode` readback.

## Workload (chosen before comparison data, under Daily only)

`vkcube` under MangoHud, unpaced (`--present_mode 0`), logged with the same `MANGOHUD_CONFIG` as the
frame campaign, with no CPU pinning or interference. Step 0 measures candidate resolutions
(1920x1080 and 3840x2160) under `Daily` only and picks the larger per-frame GPU workload that keeps
`gpu_busy_percent` at or above 90 on average; that choice is recorded and then frozen. GPU utilisation
is sampled every second from sysfs for every run.

## Design

Six blocks, each four runs of 12 s of logged frames after a 5 s warm-up, with the profile settled for
8 s after each switch and verified by readback. Block order alternates between `D D G D` and `D G D D`,
so each block yields one Daily/Daily control pair and one Daily/Gaming pair in each direction.
Per run: frame p99, p95, p99.9, mean frame time, 1 percent low (the mean of the slowest 1 percent
of frames) from valid frames; mean GPU busy, shader clock, board power and edge temperature; the
profile readback, run order and an execution id.

## Decision rule (fixed now)

Let a Daily/Gaming pair's change be (Gaming minus Daily) divided by Daily for frame p99 and for mean
frame time (negative is better), and let the control noise be the largest absolute Daily/Daily change in
the same metric.

The pilot finds a **useful repeatable effect** only if, for both frame p99 and mean frame time:
1. Gaming is better in at least 5 of the 6 blocks,
2. the median improvement is at least 5 percent, and
3. the median improvement is at least twice the control noise,

and the Gaming runs show no more than 3 C higher edge temperature and no power-limit regression.

A merely detectable gain below the 5 percent floor is recorded as no useful effect.

## Outcomes

- **No useful repeatable effect:** `DeferredEvidence`, raw data kept, original profile restored, and no
  cooperative backend is built.
- **A useful repeatable effect:** continue to a cooperative-operation design (stable PCI identity, one
  ownership authority across LACT, GameMode and Stutter, prepared-before-change recovery, an
  independent GPU-workload calibration and untouched holdout). The pilot alone graduates nothing.
