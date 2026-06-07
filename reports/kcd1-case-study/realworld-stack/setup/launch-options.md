# KCD1 real-world stack launch options

This file preserves the launch/scheduler setup used for the exploratory
real-world stack comparison without keeping raw Steam command lines. The
comparison is a configuration-bundle check, not a causal test of any individual
flag.

## Clean stack

Purpose: stripped-down measurement setup used as the controlled reference for the
real-world stack add-on.

Properties:

- default scheduler / `sched_ext` disabled
- Gamescope at 1920x1080
- 100 Hz output
- 100 FPS MangoHud cap
- MangoHud logging enabled
- kept `+exec user.cfg`
- no RADV experimental flags
- no FSR/FSR4
- no gamemode
- no mimalloc
- no forced Wine CPU topology

Launch shape:

```text
Steam -> Gamescope 1920x1080@100Hz -> MangoHud logging with 100 FPS cap -> GE-Proton10-34 -> KingdomCome.exe +exec user.cfg
```

## Personal stack

Purpose: the author's normal player-used gaming configuration bundle. This
changes many variables at once, so it must not be interpreted as a causal test of
`scx_lavd`, Gamescope FSR, RADV/Mesa flags, Wine/Proton flags, allocator choice,
gamemode, or any individual launch flag.

Properties:

- `scx_lavd` enabled with the aggressive gaming flags listed below
- Gamescope internal size 2560x1440 and output size 3840x2160
- 60 Hz output
- Gamescope FSR scaling enabled
- RADV/Mesa experimental/performance flags enabled
- Wine/Proton flags enabled
- mimalloc and gamemode enabled
- MangoHud logging enabled
- kept `+exec user.cfg`

Launch shape:

```text
Steam -> Gamescope 2560x1440 internal / 3840x2160 output at 60Hz with FSR -> RADV/Mesa and Wine/Proton experimental flag bundle -> mimalloc + gamemode -> MangoHud logging -> GE-Proton10-34 -> KingdomCome.exe +exec user.cfg
```

## Personal-stack scheduler command

The personal-stack runs used `scx_lavd` with the following flags:

```bash
doas scx_lavd \
  --cpu-pref-order 1,2,3,4,5,0,7,8,9,10,11,6 \
  --performance \
  --preempt-shift 0 \
  --slice-min-us 100 \
  --slice-max-us 1000 \
  --pinned-slice-us 250 \
  --lb-low-util-pct 0 \
  --lb-local-dsq-util-pct 0 \
  --per-cpu-dsq
```

The postcheck artifacts for the personal-stack runs reported `sched_ext` enabled
with `lavd_1.1.0_x86_64_unknown_linux_gnu`. The clean-stack runs were recorded
with `sched_ext` disabled.

## Interpretation note

This add-on intentionally represents a realistic player-used configuration
bundle. It should be used to show that `stutter` can capture and compare a
real-world setup, not to claim that any single scheduler, driver flag, Wine flag,
allocator, or Gamescope option explains the observed result by itself.
