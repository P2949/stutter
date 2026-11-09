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

## Authenticated, epoch-bound source (`telemetry::stamped_source`)

Delivery latency was never meant to stand in for source trust or stamp accuracy, so the stream now
sits behind a source that binds the producer and keeps the two uncertainties apart:

- **Binding.** The producer is bound only when the kernel's peer credentials name the expected user
  and process, that process is alive with the start time the controller recorded (a reused pid is
  refused), producer and controller share a `CLOCK_MONOTONIC` time namespace, and its hello names
  protocol version 1, `CLOCK_MONOTONIC`, a supported frame event (`present` only) and a source epoch
  greater than every epoch already used (a replayed hello is refused). A bound, living producer makes
  a second claim `CompetingProducer`. A producer that exits is withdrawn on the next check, every
  bind resets the stream's trust, and a late first frame in an epoch is counted as unexplained loss.
- **Two bounds.** The alignment uncertainty reported to the unchanged 2 ms gate is the measured
  delivery bound **plus** a stamp-error bound that must come from an independent reference
  (`StampAccuracy`: at least 1000 comparisons and a named reference, dropped when a different
  process binds). Without it the uncertainty is unknown and the gate denies however fast the socket
  is. Tests cover both bounds present (eligible), delivery only (denied), a stamp error that takes the
  sum over 2 ms (denied), wrong user and process, reused pid, vanished process, another time
  namespace, bad and replayed hellos, an unsupported event, a competing producer, restart with a new
  epoch (recalibration and re-established accuracy both required), and real socket peer
  credentials.

What is still missing is the part that needs a real renderer: the instrumented producer, an
independent reference for the stamp-versus-presentation error, and connecting the bound source to the
live observation-only controller. Until a measured `StampAccuracy` exists for a real renderer this
source cannot make any real-game stream frame-primary, and the real-game direct-frame path stays
`EvidenceDeferred`.

## Second review round: fail-closed hardening of the source (2026-11-09, evening)

Review findings against `cf02c4bd`, all fixed and tested:

- **Invalid input is loss.** A malformed, truncated or oversize (over 256 bytes) line now counts as a
  rejected message, so a stream that was otherwise complete reports loss and is denied; it used to
  return an error and leave the quality unchanged.
- **No counter overflow.** The maximum sequence number is refused (`SequenceOverflow`) instead of
  panicking in debug or wrapping in release, and every counter (accepted, rejected, gaps, histogram,
  sum) saturates.
- **Decision-time revalidation.** `quality_at` rechecks the renderer's identity first, so a renderer
  that exited, or whose pid was reused, since the last frame is no longer `ValidatedDirect`.
- **Unforgeable accuracy claims.** `StampAccuracy` has private fields and is built only from raw
  (stamp, independent-reference) pairs by `from_raw_pairs`: the bound is recomputed from the pairs
  (largest absolute difference, bias recorded not subtracted), the claim names the digest of its raw
  evidence and one of a closed set of external reference kinds (photodiode, display hardware
  timestamp, high-speed camera), and the source accepts it only for the bound renderer binary (its
  sha256 recomputed from the process's executable), the frame event the producer announced, this
  boot, and within its validity window, which is checked again at every decision.

What this still cannot establish: that the raw pairs were really measured against a physical
reference. The module can make a number impossible to state without raw data and impossible to apply
to the wrong renderer; it cannot witness the measurement. That needs the instrumented renderer and
the reference apparatus, so the real-game direct-frame path stays `EvidenceDeferred`.

## Third review round: executable identity and capture provenance (2026-11-09, evening)

- **`execve` no longer keeps trust.** A pid and start time survive `execve`, which swaps the program
  image. The bound source now stores a cheap identity of the executable (device, inode, size and
  modification time of `/proc/<pid>/exe`) and compares it at every evidence decision and ingest; a
  change retires the source and drops its accuracy claim, so the old program's calibration cannot carry
  over to a new one. A producer whose executable cannot be identified is not bound at all.
- **Reference rows must be real events.** Pairs must be strictly increasing in both the stamp and the
  reference column: a repeated, replayed or reordered row is refused (`NotMonotonic`) and cannot pad a
  measurement.
- **Caller-typed data confers nothing.** The raw-pairs constructor is now module-private. The only
  public way to obtain a claim is `StampAccuracy::from_trusted_capture`, which reads a capture file only
  if it lies in a path where every component is owned by root and not writable by group or others (no
  symlink, checked on the open file, the same trust model as the campaign receipt), names a closed set
  of external reference kinds, and has no unknown fields. The bound is still recomputed from the raw
  pairs. What this establishes is that an unprivileged caller cannot have written the capture; it still
  cannot witness the physical measurement, which needs the real renderer and reference apparatus.

## Production smoke isolation, second round

The smoke's preflight now fails closed: an unenumerable `/proc`, an unreadable entry, a command line
that stays unreadable after bounded retries, a candidate whose identity cannot be read, and a CPU-load
sample that is missing, malformed, non-finite, negative or without elapsed time all block, exactly as a
detected competitor does; only a process that genuinely vanished is skipped. A fixture is exempt only by
exact identity (its pid and start time, or an ancestor's), no longer by a command line that merely lies
under the smoke's output directory. The post-run audit is strict: every state-changing history record
must name a parseable pid that is a fixture of this run, whose comm matches and whose start time, where
recorded, equals the fixture's; a record that cannot be attributed fails, and so does a history that
shrank (rotation) or has unparsable lines. This build's history leaves `process_starttime_ticks` null, so
those records bind by pid and comm only, and the check says so.

### Evaluation: an exact validation-only target fence at the privileged worker

Feasible without expanding normal authority, because it can only **narrow**. The worker already
revalidates that a candidate's named task (tid, comm, start time) is still the live task
(`daemon/privilege/revalidate.rs`) and restricts service targets by uid and cgroup
(`service_targets.rs`); it does not know which campaign authorized a target. A fence would be an
expiring, root-owned file (written by the runner or by `sudo -n`, with the receipt's trust model) naming
one pid and start time, enforced beside `ServiceTargets`: while a valid fence exists, any candidate
whose target is not the fenced task is denied; with no fence, behaviour is unchanged. It is **not
implemented here** because it changes the production privileged worker: it needs its own tests, a
qualification and a deployment, which is the work the final-source deploy is already waiting on. Until it
exists the installed-service *mutation* smoke remains only as safe as the preflight plus the strict
post-run audit, and must not be run while the desktop is in use. The deterministic isolated
transient-controller acceptance suites remain the way to exercise the mutation path without the
installed service.
