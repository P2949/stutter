# Renderer-stamped frame stream: measured delivery bound on a controlled fixture (2026-11-09)

`controlled_fixture` engineering evidence. It is **not** a game, **not** a Vulkan layer, and it
establishes **no attribution**; it does not change the frame-primary decision for any real source.

## Why

MangoHud's CSV has no absolute clock anchor, so its rows can be placed on the controller's clock only
through a polled offset with 454-507 ms of uncertainty against the unchanged 2 ms gate. The review
asked for an opt-in, renderer-bound timestamp whose error is *measured*. This is the measured
contract and method, on a fixture, with the gate applied unchanged.

## Contract (`stutter/src/telemetry/stamped_frames.rs`)

- The producer stamps each frame with `CLOCK_MONOTONIC` at its present call and pushes it to the
  controller; both share the clock (the caller verifies the same time namespace, as the KPI source
  does), so there is no offset to estimate.
- The controller reads the same clock as each frame arrives and records `rx - stamp`. A frame
  stamped after the controller's own reading, out of order, with a repeated or regressing counter,
  or older than 250 ms is rejected and counted as loss; a skipped counter is unexplained loss.
- The alignment uncertainty is the **maximum** delivery latency over the most recent 4096 accepted
  frames (a sliding window; at least 1000 accepted frames this epoch are needed), a one-sided bound. With no calibration it is `NaN`, which the gate refuses, never zero.
  One slow delivery in the calibration is the stream's bound, not an outlier to drop.
  `MAX_ALIGNMENT_UNCERTAINTY_MS` stays 2.0.

## Measurement (`xtask frame-clock-prototype`, 3000 frames at 60 Hz, 12 logical CPUs)

| run | spin threads | frames | mean µs | p99 µs | p99.9 µs | uncertainty ms (max) | 2 ms gate |
|---|---|---|---|---|---|---|---|
| idle | 0 | 3000 | 24 | 51 | 113 | 2.136 | false |
| idle-2 | 0 | 3000 | 21 | 47 | 110 | 0.362 | true |
| idle-3 | 0 | 3000 | 21 | 45 | 52 | 0.114 | true |
| idle-4 | 0 | 3000 | 19 | 44 | 49 | 0.092 | true |
| spin4 | 4 | 3000 | 22 | 47 | 197 | 0.307 | true |
| spin12 | 12 | 3000 | 77 | 1357 | 4015 | 8.009 | false |
| idle-postfix | 0 | 3000 | 26 | 131 | 327 | 2.677 | false |

(`idle*` rows are five unloaded runs, the last one (`idle-postfix`) taken after the calibration-window fix below; the desktop session was logged in, load average 2.7-5.3. Note the p99 and p99.9 columns of that row come from the new 256-bucket histogram and are bucket upper bounds, so they read higher than the exact values of the earlier rows.)

## What this shows, and what it does not

- Delivery latency is tens of microseconds typically (mean about 25 µs, p99 about 50 µs). The
  bound set by the maximum is under 2 ms in four of seven runs, including with four competing
  CPU-bound threads, and is three orders of magnitude tighter than MangoHud's ~500 ms.
- It is **not** reliably under 2 ms on an ordinary non-real-time desktop: the first idle run had a
  single 2.14 ms stall, and with twelve spinning threads on twelve CPUs the maximum was 8.0 ms. The
  gate is not relaxed and not averaged: a calibration containing such a stall is denied, and the
  stream would have to recalibrate.
- The delivery bound is a conservative stand-in for alignment: placing a frame by its stamp has no
  delivery term at all on a shared clock, but the stamp's own accuracy against the true present is
  the instrumentation's, and is not measured here. A real renderer integration (a Vulkan layer or an
  instrumented engine) must measure that against a reference, so the **real-game direct-frame path
  stays `EvidenceDeferred`**. This prototype is not wired into the daemon or the frame-source
  attribution path.

Raw results stay in `.llm-runs/frame-clock-20261109/` with these SHA-256 values:

```
idle     7c6319cd36bd341073f70a7bc3234519f16e3e3a6baa4d4a7280f67a404fc2e1
idle-2   a443778130fe71664eb25fc5a38432e0e909cebcd2246a718ea16aa763994895
idle-3   77373f0c020cebe82d1ee7c39edf851be0e994b9cff40535fae309cf4a24c359
idle-4   b0b597174adb7b66a867d7b2a8fa272a4f47e13020db27ba79aa6e352b1f684f
spin4    72356ed99631b20e6c24e1b37bd9d3aa03c71d9a224bc40cdc75781d526540fe
spin12   de428cfd5404094e859f9a1152c6d80e953a7bb39e9fd0a9168ebd2ba71a4947
```

## Correction after review (calibration window)

The first version of the stream module kept only the first 131,072 delivery latencies and then
stopped updating its calibration, so a stall after that many frames left a stale passing bound. It
now keeps a sliding maximum over the last 4096 accepted frames in constant time and memory: a
stall denies the stream while it stays in the window and then expires, the epoch's worst delivery is
still reported, a new epoch resets all trust, the first frame must carry the epoch's announced first
sequence number (a late start counts as loss), and a silent stream (over 500 ms) is unhealthy. The
regression test feeds more than 131,072 fast frames and then a 3 ms delivery and requires denial; further
tests cover expiry, epochs, silence, and bounded memory and time at 60, 155 and 240 Hz. The fixture
numbers above were produced with frames at most the window size, so the gating maximum is the same
as the earlier version computed for them.

Still not measured, and still the reason the real-game direct-frame path stays `EvidenceDeferred`:
how accurately a renderer's stamp matches its actual presentation event. Delivery latency and
event-stamp accuracy are different quantities, and only the first is measured here.
