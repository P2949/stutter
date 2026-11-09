# Real-workload telemetry and external owners

This page describes the opt-in pieces that let Stutter observe a real game's frames and recognise
other tuning tools on the machine. None of it is on by default, none of it widens what the autotuner
may change, and the Supported autonomous boundary in [CAPABILITIES.md](CAPABILITIES.md) is
unchanged. The honest status of every tool is in
[INTEGRATION_IMPLEMENTATION_STATUS.md](INTEGRATION_IMPLEMENTATION_STATUS.md).

## Launching a game with frame capture

```sh
stutter game-run --mangohud -- %command%
```

`game-run` starts the exact command with its working directory, environment, stdio, signals and exit
status preserved, so it works as a Steam launch option. With `--mangohud` it also points MangoHud at a
fresh owner-only directory under `$XDG_RUNTIME_DIR/stutter/game-run` and asks for a per-frame log. It
never forces itself or MangoHud on an existing launch, holds no privilege, and keeps the 16 newest
capture directories. A launch id it records is correlation data only.

## Letting the controller use the frames

```toml
[autotune]
frame_capture_root = "/run/user/1000/stutter/game-run"
```

Left unset, nothing changes. When set, the controller looks for a launch that is verified (owner-only
directory, this boot, this user, the recorded process still the same one) and whose process tree
contains the focused game, then tails that log. Frames become a *direct* frame signal only while each
tick re-verifies the producer, focus, freshness and the absence of any loss. A wrong game, a replaced
process, a stale or lossy stream, or a truncated log can never produce a frame-primary keep, and frames
produced before a measurement phase ended are discarded at the boundary. Direct frames never authorise
an operation; the existing frame-pacing calibration is unchanged.

## What `stutter doctor` shows

* `external_owners`: which tuning domains another tool holds (GameMode, LACT, sched_ext, Ananicy,
  System76 Scheduler, TuneD, auto-cpufreq, power-profiles-daemon, irqbalance, bpftune, numad), with the
  interface Stutter could read it through and why delegation is refused. A process name is only a hint,
  and anything unreadable counts as owned.
* `exact_owner_states`: what GameMode (client count), `scx_loader` (current scheduler) and
  power-profiles-daemon (active profile) report about themselves over D-Bus. This reads and never writes.
* `topology` and `boot_environment`: cache and CCD domains, verified capacity classes, NUMA distances,
  GPU locality, kernel, preemption model and isolation parameters, each with a fingerprint. A stored
  experiment is not reused when the topology or boot context changes.

## Sensors

The runtime samples pressure (PSI), CPU power state, NUMA locality, DAMON and resctrl inventory and the
user slice's memory-event counters on every evaluation, and costs well under a millisecond. Sustained
pressure raises a *logged hypothesis*, which may start the PMU sensor under a one-at-a-time budget. A
reading is context for diagnosis and never triggers a write.

## Offline tools (xtask)

* `xtask real-workload validate | run | resume | evaluate | inventory`: a frozen, counterbalanced,
  supervised comparison with a preregistered threshold. A run on this repository's controlled vkcube
  workload is under [validation/real-workload](validation/real-workload/controlled-vkcube/README.md);
  it is an engineering check, not real-game evidence.
* `xtask benchmarks verify | import-hyperfine`: verified external artifacts as evidence, never as authority.
* `xtask offline-optimization check`: PGO, AutoFDO and BOLT plans are validated, never built or applied.
* `xtask closeout-report`: the per-tool dispositions, refusing to overstate the ledger.

## Observing an application KPI

An application can report its own KPI (a request p99, a batch completion time) over a local socket.
`stutter kpi-serve` binds an owner-only socket and prints, after each connection, one JSON line per
contract metric summarizing the last window:

```sh
stutter kpi-serve --dir "$XDG_RUNTIME_DIR/stutter/kpi" --contract kpi-contract.json --window 10
```

The contract is a JSON `KpiContract`: one primary metric, at least one hard guardrail, and the one
process (uid, pid and live start time) allowed to report. Each sample is a line of JSON; the reply
line is `ok` or `denied <reason>`. The peer's identity comes from the kernel, not from the sender;
a reused pid, a stale or future timestamp, a replay or an unknown metric is denied. A quantile of
batch summaries is labelled `quantile_of_summaries` and is never a request or frame percentile.

The standalone `kpi-serve` command is observation only: it runs no operation, and a summary it
prints authorizes nothing. The daemon can also attach the same exact-process collector when the
optional `kpi_contract` and `kpi_socket_dir` settings are present; it feeds runtime diagnostics and
the frozen observation-only study, never a keep decision on its own.

### Sensor cadence

Sensors are not all sampled every tick. The sensor hub gives each a minimum interval so the idle
cost stays inside its budget: `numa_stat`, `damon_state` and `resctrl_inventory` every 30 s;
`resctrl_monitor`, `network`, `numa_faults` and `block_io` every 5 s; `cpu_idle` every 10 s;
`cpu_power` every 2 s. The safety guards (`psi`, `oom_guard`, `sched_ext`) stay on every tick.
The table is in `stutter/src/autotune/runtime/sensor_hub.rs`.

### Renderer-stamped frames (prototype)

The direct-frame requirement stays EvidenceDeferred for real games. A measured alternative to the
MangoHud polling offset exists as an opt-in prototype: `telemetry::stamped_frames` validates a
producer-stamped `CLOCK_MONOTONIC` stream and turns its measured delivery latency into the alignment
uncertainty the unchanged 2 ms gate checks, and `xtask frame-clock-prototype` measures it on a
controlled fixture. On the 2026-11-09 fixture runs the bound was under 2 ms in four of six runs and
not under heavy CPU oversubscription; it is not wired into the daemon and establishes no
attribution. See `docs/validation/frame-clock-prototype-2026-11-09/RESULTS.md`.

### Experimental audio and browser contracts

Instead of writing a contract file, `--template audio-xrun --pid PID` or `--template
browser-response --pid PID` binds a versioned experimental contract (`experimental-1`) to one live
process of yours. Audio is judged on the application's own audio-callback deadline misses with
server-reported xruns as a hard guardrail; browser and desktop interactivity on the p95 of
input-to-response time with long main-thread tasks as a hard guardrail. The thresholds are declared,
not calibrated, nothing is derived from CPU use, and the contract authorizes no operation.

## Frame-primary clock quality: EvidenceDeferred (2026-11-09)

Direct frame-primary measurement requires the game-to-monitor clock mapping to be bounded within
2 ms. The only live source today is the MangoHud CSV log. Its `elapsed` column counts from
MangoHud's own logging start; the file carries no absolute anchor, and lines are buffered and
flushed by MangoHud's writer, so the only available bound is the observer's poll gap plus the
unknown flush delay. The development-controller run measured 482-507 ms against the 2 ms gate.

This is recorded as **EvidenceDeferred**, not as a pass and not as a threshold to relax:

- `StreamQuality::frame_primary_eligible` keeps its 2 ms alignment gate; unknown alignment is NaN
  and denies. Frame data stays diagnostic (attribution, loss and age are still reported).
- No substitute is accepted: file mtime, CSV read time, a guessed start offset, or a global KMS or
  compositor event without process correlation.
- What would unblock it, any one of: a renderer-side timestamp with a documented flush bound in the
  game's own monotonic clock (for example a Stutter-owned Vulkan layer or a MangoHud build that
  logs an absolute `CLOCK_MONOTONIC` anchor at start and flushes per frame), or a per-process
  presentation event stream (DRM fence or KMS events attributed to the producing process) whose
  error is measured against ground truth on a fixture.
- Once a candidate exists, retest source loss, rotation, a competing renderer, focus, suspend and
  phase boundaries before any direct-frame experiment.
