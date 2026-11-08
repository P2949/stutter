# Local KPI observation

The optional runtime collector is configured with both absolute paths:

```toml
[autotune]
kpi_contract = "/path/to/contract.json"
kpi_socket_dir = "/path/to/owner-only-directory"
```

The directory must already be owned by the monitored user and inaccessible to other users.
The collector creates `kpi.sock` with mode 0600 and refuses an existing path. The contract is a
bounded regular JSON `KpiContract` file. Its binding must match the currently resolved root's
UID, PID and live start time. Executable identity, boot and collector session are tracked as well.
The wire protocol is newline-delimited `Sample` JSON, with one `ok` or typed denial per line.
The producer and observer must share the active Linux time namespace: `CLOCK_MONOTONIC` is scoped
to that namespace ([Linux time namespace contract](https://man7.org/linux/man-pages/man7/time_namespaces.7.html)).
Missing, different or changed namespace identities withdraw this source; no offset is guessed.
Captures record the namespace and executable identity. Older diagnostic JSON without this added
namespace field still decodes and does not acquire capture provenance.
An example after completing known work is:

```json
{"metric":"completed_units","value":1000,"monotonic_ns":123456789,"work_unit":"fixed-input-digest","population":1000,"kind":"batch_sum"}
```

Use the reporter's `CLOCK_MONOTONIC` timestamp. One collector handles at most 4096 bytes and a
2 ms read-loop budget per controller tick, using nonblocking I/O, a bounded line buffer and an idle
connection limit. Metric count, labels, samples and contract size are bounded. Identity changes
clear the source store. A source-off or unavailable optional collector does not alter the original
runnable-only controller. Decision-stream `kpi_source` contains observation diagnostics, separate
primary/hard/advisory roles, populations, completeness and semantics. It grants no mutation authority.

Only compatible statistics compose into an underlying-event aggregate:

| Sample kind | Compatible direct aggregation |
|---|---|
| `raw_event` (population exactly one) | Mean, sum, max, last and event percentiles |
| `batch_mean` | Population-weighted mean |
| `batch_sum` | Sum |
| `batch_max` | Max |
| `batch_last` | Last underlying event |
| Legacy/omitted `summary` | Diagnostic only |

A p99 of batch p99s is `QuantileOfSummaries`; an incompatible mean/sum/max/last is
`DiagnosticSummary`. Declaring a kind does not attest that an application measured it correctly.
Mixed work IDs, capacity loss, stale/underpopulated windows and nonfinite aggregates cannot supply
a complete direct block. The service does not consume these diagnostics as an autonomous objective.

The embedding API can freeze a `KpiObservationStudyConfig` on an observation-only `AutotuneRuntime`, capture
`ObservedKpiBlock`s, submit counterbalanced independent pairs and analyse planned fixed-N or adjusted
sequential looks. Hard guardrail limits and one completed-work identity are frozen across all pairs.
Target retirement changes the
capture session epoch, invalidating an unfinished study even if the same process returns. Rejected
source rows and partial/lost input withdraw completeness; previously accepted rows cannot hide them.
Captures contain accepted raw samples and can be serialized for evidence, but cannot be deserialized
into trusted capture objects.
Imported JSON and a matching digest do not establish collector provenance. Raw captures must be
retained by the campaign; `verify_captures` reconstructs metric aggregates and every statistical
interval and rejects coherent but fabricated interval claims. The observation-study path never
populates the mutation manager's keep evidence or changes the support registry.

Run the bounded unprivileged demonstration with:

```sh
nix develop --command cargo run -q -p xtask -- kpi-observation run --out /new/private/run
```

It supervises one owned producer, measures eight completed 100000-step integer folds with an
independently checked checksum, and uses four counterbalanced pairs of identical arms. Raw blocks
are persisted before analysis. It checks reconstruction, action-state emptiness, producer exit and
socket cleanup. The source metrics are real; scheduler monitor ticks are supplied by the harness.
It does not change a setting, qualify the installed service or demonstrate a tuning benefit. The
2026-11-08 first run completed with the frozen classifier's `Equivalent` verdict; its interval
excluded a 20 percent gain but did not establish strict equivalence within the 5 percent margin.
Raw evidence is retained privately in `.llm-runs/kpi-observation-20261108`.
