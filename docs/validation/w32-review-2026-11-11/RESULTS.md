# W32: registry-led evidence and capability review (2026-11-11)

Generated or checked from the registries, not written by hand:

* `xtask closeout-report --strict`: passes; it refuses a ledger that overstates (unknown disposition, physical-proof
  disposition without evidence, `HardwareUnavailable` or `NotPlannedForVerification` without a reason, `code_complete` with a
  missing required subcapability). Rendered report: `../closeout-report-2026-11-09.md`.
* `xtask capability-check`: "no capability is Supported ahead of its evidence". `xtask objective-docs --check` passes (objective
  contract docs match the code). Both run in every `xtask ci`.
* `stutter capabilities` and `docs/CAPABILITIES.md` list the same tiers; the only Supported mutating surface is the packaged
  bounded systemd service with the two `AUTONOMOUS_CONTRACTS` entries. No entry has `AutonomySupported` in the ledger (0), and
  no new Supported tuple was added by any 2026-11-11 work.
* `stutter doctor` (unprivileged) reports missing tracepoint formats and a reduced-coverage warning: tracefs is root-only for this
  account, the registry probes load from `PROBE_REGISTRY`, and the degraded-evidence messages name the exact cause, as designed.

Ledger state: 34 of 34 entries `code_complete`, `FixtureVerified` 34, `RealObserved` 21, `MechanismValidated` 4,
`NotPlannedForVerification` 11, `EvidenceDeferred` 31 (each with an unblock or reason). Safety invariants at close: 0 of 513
ledger entries outstanding, both units active, no fence file, no orphan load/renderer/scheduler process, zero reboots, the
previous NixOS generations preserved, no public GitHub activity.

One focused evidence or diagnostic improvement per phase this round: W21 real renderer stream; W23 compile cohort (CV 1.62 %);
W24 benefit/harm classification; W25 held-out ranking; W26 observer-cost budget; W27 cgroup delegation facts; W28 live
coexistence; W29/W31 blockers stated; W30 real `wrk2`/`hyperfine`/hwloc captures.

## Items that stay open, and why they cannot be closed here

* **Real-game direct-frame benefit study (W22 reference + W24 game part): `NotPlannedForVerification`.** The accepted
  independent references are a photodiode, a display hardware timestamp or a high-speed camera; none exists on this machine and a
  display timestamp for a composited window would need the display taken from the user's session. Without that reference the frame
  objective stays measurement-only and direct-frame keep is denied (proven live in W21). No unattended game benchmark exists
  either. This is hardware this machine lacks, which the user designated as not planned for verification.
* **Different-hardware / platform items** (multi-node NUMA, RDT, OpenRC, vendor VTune/uProf, CoreCtrl, alternate kernels that
  need reboots, a kdamond-free host, the packaged `scx_loader`): `NotPlannedForVerification` or `BootUnavailable` with the
  reason recorded in the ledger.

## Probe: is any independent frame-timing reference reachable on this host? (2026-11-11)

* No camera: no `/dev/video*`. No light or photodiode sensor: no `/sys/bus/iio/devices`. USB devices are input, audio, hub and LED
  controllers only (no capture device).
* The only DRM device is `card1` (one connected output, `DP-3`) and it is the display of the user's active Wayland session
  (`Type=wayland`, `Active=yes`). A display hardware timestamp (page-flip/vblank) for a frame needs the application to own the
  scan-out (`vkcube --wsi display`, DRM master), which means taking the display from that session; and a composited window's
  present has no one-to-one page flip to pair with. Doing that to the user's only display is excluded by the standing constraint not
  to disturb an occupied session.

So every accepted reference kind (photodiode, display hardware timestamp, high-speed camera) is unavailable here by hardware or
by the standing session constraint, which makes the W22 independent reference, and with it the real-game direct-frame benefit
study, an irreducible blocker on this machine, not merely unfinished work.
