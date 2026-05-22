# Auto-tune Objectives

Objective comparison is intentionally explicit: each objective has a primary metric when the relevant signal exists, and normalized stutter score is a guardrail or fallback rather than an implicit universal gate.

| Objective | Primary comparison | Required signals | Guardrails |
| --- | --- | --- | --- |
| `StutterScore` | Normalized score per sample | Scored samples | Frame p99 and over-5ms rate |
| `GameFramePacing` | Frame p99 when frame pacing is available; normalized score fallback | Frame data when available | Foreground over-5ms rate, thermal, CPU/GPU power |
| `GameRunnableLatency` | Normalized runnable latency score through the stutter score model | Scored task latency | Frame p99, foreground latency, thermal, CPU/GPU power |
| `DesktopInteractivity` | Foreground over-5ms rate when available; normalized score fallback | Foreground latency when available | Thermal and CPU/GPU power |
| `BrowserInteractivity` | Foreground/browser latency approximation when available; normalized score fallback | Foreground latency when available | Thermal and CPU/GPU power |
| `CompileThroughputWithForegroundProtection` | Normalized score fallback until a direct `compile_progress_intervals` signal exists | Foreground latency for protection | Frame and foreground regression |
| `IoLatency` | Block I/O overlap count, then worst overlap latency | Block I/O overlap count and worst latency | Generic normalized score cannot regress badly |
| `IrqOverlapReduction` | IRQ overlap count, then worst overlap latency | IRQ overlap count and worst overlap | Generic normalized score cannot regress badly |
| `ThermalRecovery` | Degraded state recovery and throttle-count reduction | Thermal degraded state and throttle count | Generic normalized score cannot regress badly |

## Signal Maturity

`IoLatency`, `IrqOverlapReduction`, and `ThermalRecovery` have direct primary-metric handling. `StutterScore` is fully implemented around normalized score rates. `GameFramePacing`, `DesktopInteractivity`, and `BrowserInteractivity` use direct or derived latency signals when present and fall back to normalized score when those signals are missing. `GameRunnableLatency` and `CompileThroughputWithForegroundProtection` remain fallback-based because the current runtime does not expose a distinct runnable-latency or compile-throughput progress signal.
