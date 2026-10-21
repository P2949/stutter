# Auto-tune Objectives

Objective comparison is intentionally explicit: each objective has a primary metric when the relevant signal exists, and normalized stutter score is a guardrail or fallback rather than an implicit universal gate.

| Objective | Primary comparison | Required signals | Guardrails |
| --- | --- | --- | --- |
| `StutterScore` | Normalized score per sample | Scored samples | Frame p99 and over-5ms rate |
| `GameFramePacing` | Frame p99 when frame pacing is available; normalized score fallback | Frame data when available | Foreground over-5ms rate, thermal, CPU/GPU power |
| `GameRunnableLatency` | Merged target-game scheduler histogram p99; no fallback | Direct complete histogram population, stable identified target, at least 1,000 runnable samples | >5ms runnable rate, frame p99, foreground latency, thermal, CPU/GPU power |
| `DesktopInteractivity` | Foreground over-5ms rate when available; normalized score fallback | Foreground latency when available | Thermal and CPU/GPU power |
| `BrowserInteractivity` | Foreground/browser latency approximation when available; normalized score fallback | Foreground latency when available | Thermal and CPU/GPU power |
| `CompileThroughputWithForegroundProtection` | Successful identical fixed-work completion time, lower is better; no score fallback | Ordered matching completion markers and foreground protection | Frame, foreground, thermal, CPU/GPU power, observed memory pressure |
| `IoLatency` | Block I/O overlap count, then worst overlap latency | Block I/O overlap count and worst latency | Generic normalized score cannot regress badly |
| `IrqOverlapReduction` | IRQ overlap count, then worst overlap latency | IRQ overlap count and worst overlap | Generic normalized score cannot regress badly |
| `ThermalRecovery` | Degraded state recovery and throttle-count reduction | Thermal degraded state and throttle count | Generic normalized score cannot regress badly |

## Signal Maturity

`GameRunnableLatency`, `IoLatency`, `IrqOverlapReduction`, and `ThermalRecovery` have direct primary-metric handling. The [runnable-latency contract](RUNNABLE_LATENCY_OBJECTIVE.md) defines target membership, histogram resolution, sufficiency, provisional thresholds, and validation limits. `StutterScore` is fully implemented around normalized score rates. `GameFramePacing`, `DesktopInteractivity`, and `BrowserInteractivity` use direct or derived latency signals when present and fall back to normalized score when those signals are missing. `CompileThroughputWithForegroundProtection` uses validated fixed-work completion time; missing or incomparable completed work remains inconclusive. It never uses activity counts or normalized score to claim throughput improvement. See the [fixed-work contract](FIXED_WORK_COMPILE.md) for producer responsibilities and validation limits. Historical `compile_progress_*` fields deserialize as `compile_activity_*`, and historical direct-quality labels do not restore the old comparator.
