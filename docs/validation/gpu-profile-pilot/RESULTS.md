# LACT GPU-profile pilot: result (aborted for safety; no useful effect observed)

Outcome: `DeferredEvidence`. The pilot was aborted in block 4 by a GPU hang, and it will not be resumed.

## What ran

Workload frozen in Addendum A: `vkmark -b effect2d:duration=40 -s 3840x2160 -p immediate` under MangoHud
(GPU 99.8 to 100 percent busy, shader clock about 3.26 GHz, board power at the 238 W cap). Profiles were
switched only with `lact cli profile set`; every run's profile was read back through LACT and the
firmware mode (`2_POWER_SAVING` for Daily, `1_3D_FULL_SCREEN` for Gaming) from `pp_power_profile_mode`.
Power cap, voltage offset and fan settings were never changed, and `Daily` was restored and verified
after the abort. Raw rows: [results-analysis.txt](results-analysis.txt), [run-log.txt](run-log.txt).

## Three complete blocks (all that exist)

| Block | Daily mean frame (ms), Daily/Daily control | Gaming mean frame (ms) | Gaming p99 (ms) | Daily p99 (ms), 3 runs |
|---|---|---|---|---|
| 1 | 0.1800, 0.1777, 0.1778 | 0.1781 | 0.2749 | 0.2916, 0.2732, 0.2763 |
| 2 | 0.1780, 0.1778, 0.1779 | 0.1784 | 0.2783 | 0.2781, 0.2762, 0.2663 |
| 3 | 0.1779, 0.1779, 0.1767 | 0.1773 | 0.2254 | 0.2743, 0.2773, 0.2587 |

- Mean frame time: Gaming differed from the adjacent Daily runs by -0.3 percent to +0.2 percent in every
  block, which is the same size as the Daily/Daily control noise. There is no improvement.
- Frame p99: Gaming was inside the Daily spread in blocks 1 and 2 and lower in block 3 (about 18 percent),
  one favourable block among three; Daily/Daily p99 itself varied by up to 9 percent in a block.
- Shader clock was 3239 to 3251 MHz under Gaming against 3254 to 3266 MHz under Daily: slightly lower,
  not higher. Power and temperature were the same within noise.

Against the preregistered rule (Gaming better in at least 5 of 6 blocks, median improvement at least 5
percent and at least twice the control noise, for both p99 and mean frame time), the completed blocks give
no mean-frame-time improvement at all. The rule is not met and nothing suggests it would have been.

## The safety event (a finding in its own right)

At 09:09:01, during `b4-p0-Daily` (a `Daily` run, so not caused by the Gaming profile), amdgpu logged
`ring gfx_0.0.0 timeout` for the vkmark process, `Ring gfx_0.0.0 reset failed`, then a full MODE1 GPU reset
("VRAM is lost due to GPU reset!"), and the desktop compositor segfaulted afterwards
([excerpt](kernel-gpu-hang-excerpt.txt)). The GPU was at 48 C, so it was not a thermal fault. The harness aborted on
the missing frame log as preregistered and restored `Daily`.

The likely cause is the stability margin of the host's GPU configuration (a -150 mV offset at about 3.26 GHz
and the 238 W cap) under a sustained saturating load. That configuration belongs to the NixosConf GPU
policy and was not changed. Consequently: no further sustained GPU-saturating workload is run on this
host, and any later GPU experiment must use a bounded, non-saturating workload or a different clock policy.

## Decision

- `GpuPowerProfile` stays `DeferredEvidence`: no useful repeatable Daily/Gaming effect was observed on a
  genuinely GPU-bound workload, and the experiment cannot safely be extended here.
- No cooperative LACT backend is built (P2 is skipped); the work continues with P3 and P4.
