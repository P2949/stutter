# IRQ affinity feasibility pilot (igc NIC queues)

Question: does steering the NIC receive-queue interrupts away from the CPU a latency-sensitive task runs on
improve that task's wake-up latency under sustained network traffic? This replaces the earlier
"no traffic peer on this host" blocker: the host has a live `igc` ethernet interface (`enp6s0`, four
TxRx queues with writable affinity), and a public HTTP download is a controllable traffic source.

Method: a probe pinned to CPU 4 (500 us sleeps, oversleep measured, 30 s) while eight parallel HTTP
downloads ran. In `default` each queue IRQ 146-149 was pinned to CPUs 3-6 (queue 147 on CPU 4); in
`steered` they were pinned to CPUs 8-11. Four interleaved blocks with alternating order, one no-traffic
reference, affinities restored on exit. Raw rows: [irq-affinity-pilot-results.txt](irq-affinity-pilot-results.txt).
An earlier attempt of this pilot was discarded as invalid (the download endpoint refused the request, so
no traffic flowed; then the unchanged affinity file did not migrate the IRQs back); only the corrected run is reported.

| mode | NIC interrupts on CPU 4 per trial | mean p99 (us) | mean p99.9 (us) |
|---|---|---|---|
| default | 76k to 157k | 21 | 89 |
| steered | 0 | 22 | 94 |

Result: the mechanism is observable and physically verified (interrupts leave CPU 4 entirely), but wake
latency does not improve; the tail (max about 3 ms) is the same in both modes and is dominated by other
desktop activity. At about 95 Mbit/s of received traffic the interrupt load is a small fraction of one CPU.
Decision: `irq_known_device` stays `DeferredEvidence`, now with measured evidence rather than a missing
peer. A heavier, queue-saturating traffic source or a latency-critical protected workload sharing the
NIC CPUs would be a new preregistered hypothesis.
