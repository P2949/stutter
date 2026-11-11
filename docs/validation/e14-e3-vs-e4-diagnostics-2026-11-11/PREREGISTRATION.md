# E14 preregistration: why did owned affinity (E3, about 43%) beat owned scope weights (E4, about 7%)? (written before any measurement)

Observation already in hand (from the E3/E4 raw data, no new run): mean elapsed seconds were E3 baseline 9.77, E3 best arms about 5.5; E4 baseline
7.06, E4 best arms about 6.6. E4's own baseline (compile and hogs in separate equal-weight scopes) is already 27.7% faster than E3's
unscoped baseline, so the two studies measured against different baselines. This snapshot checks WHERE the CPU time went; it cannot assign a
cause from an effect alone, and it compares across two separate studies.

Question: in the E3-style context (affinity family) and the E4-style context (scope family) does the compile receive different CPU time per
wall second, migrate or switch more, or see different system CPU pressure, in the baseline and in one candidate each?

Two short studies (6 slots each, 3 counterbalanced blocks, `diagnostics: true`), the E3 workload identity (cold debug `stutter-config`, 2 jobs),
12 owned hogs: `e14a-affinity-diag` (manifest-affinity.json, digest `a241ab252902c5e4b0ea15f7d354af8c6488c9e680fe77247fc7c036307858db`): baseline versus
`hogs-sibling-threads`; `e14b-scope-diag` (manifest-scope.json, digest `c649cc39b84ede803e6b66a92ba8c891ba8317c95df65d781decc532142da62a`): scope baseline (100/100) versus
`hogs-scope-weight-10`. No second 30-slot campaign.

Signals, each labelled by scope and time domain: `perf stat` on the compile's own process tree, USER-SPACE ONLY, events task-clock,
context-switches, cpu-migrations, cycles:u, instructions:u, with percent-counted so multiplexing is visible; derived CPU seconds per wall second
(task-clock / elapsed) and user-space IPC; SYSTEM-wide CPU "some" stall time from /proc/pressure/cpu, snapshotted around the compile
(kernel microseconds). No runnable-delay tracepoint is read (needs tracefs rights this harness lacks). The perf wrapper is identical in every
arm, so its overhead cancels between arms but is not zero. A counter perf cannot count is Unavailable, never zero.

Decision rule for the report (descriptive, not a hypothesis test): compare per-arm means of the derived signals within each study and
state the pattern; the limited conclusion may only say what moved together with the elapsed-time change. A game running or an incomplete
host inspection invalidates a slot. Same seeds, thresholds and verdict rule as E3 for the elapsed-time effects. No production authority.
