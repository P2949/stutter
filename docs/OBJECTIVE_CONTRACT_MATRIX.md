# Objective behavioral contracts

The reusable matrix lives in
`stutter/src/autotune/objective/tests/contract_matrix.rs`. Its objective-specific
fixture uses an exhaustive match: a new objective requires a fixture decision.
Run it with `cargo test -p stutter --lib contract_matrix`; the complete objective
suite is `cargo test -p stutter --lib autotune::objective`.

Every objective runs through primary improvement, regression, unchanged, missing
signal, target disappearance, low experiment quality, zero scored samples,
thermal degradation, increased throttling, CPU/GPU power-limit regression, and
generic-score conflict. Assertions verify verdicts, not merely absence of panic.
The common safety cases start with an improving primary metric and vary one
failure at a time. Low-quality experiment data and an exited target reject the
candidate; structurally invalid zero-sample windows are Invalid.

| Objective | Primary exercised | Missing-primary behavior | Additional contracts |
| --- | --- | --- | --- |
| StutterScore | Score/sample | Score lives in WindowScore; zero samples invalid | Equal rates under 10x sample imbalance; better rate despite larger raw total |
| GameFramePacing | Frame p99 | Explicit score fallback | Foreground regression; worsening p99 despite better score |
| GameRunnableLatency | Merged target scheduler p99 | Inconclusive, no fallback | Insufficient samples, missing/low-quality histogram, severe-tail rate, target/process identity, frame guards, score conflict |
| DesktopInteractivity | Existing foreground >5ms/sample signal | Explicit score fallback | Foreground regression/improvement |
| BrowserInteractivity | Same existing foreground approximation | Explicit score fallback | Foreground regression/improvement; no claim of new browser-specific measurement |
| CompileThroughputWithForegroundProtection | Fixed-work completion elapsed time | Inconclusive, no fallback | Activity-only refusal, source/workload/process identity, failures, overlap, foreground, memory pressure, score conflict |
| IoLatency | Overlap count and worst latency | Inconclusive | Worst latency changes at constant count; score regression guard |
| IrqOverlapReduction | Overlap count and worst latency | Inconclusive | Worst latency changes at constant count; score regression guard |
| ThermalRecovery | Throttle count and degraded state | Inconclusive | Degraded-to-healthy, healthy-to-degraded, throttle increase/decrease; score regression guard |

Dedicated runnable and compile modules extend the shared matrix. Existing
individual tests cover frame/foreground guards and thermal state transitions.
The shared score-conflict test records the current policy: runnable latency and
fixed-work compile completion may improve despite a worse generic score; other
objectives retain their generic score guard or score primary. Registry and
generated documentation work must represent those differences explicitly.

These are synthetic behavioral contracts. They do not supply live acceptance,
empirical thresholds/confidence, kernel compatibility, or performance evidence.
The existing foreground signal remains derived; browser-specific target
semantics and direct foreground measurement must not be inferred from these tests.
