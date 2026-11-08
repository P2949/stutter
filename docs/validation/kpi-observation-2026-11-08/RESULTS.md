# Source-bound KPI observation and independent-block analysis

**Full PLAN.md status: IN_PROGRESS.** The integration ledger remains 23 code-complete and 11 partial.
This milestone grants no new operation/objective tuple and does not deploy to production.

Source `1d5a3ef0458f08d7e50ff5c00c2dcc6a505d52d2` passed all eight local CI stages: policy,
lint, build, objective, smoke, campaign, daemon safety and schema. The full library run passed
3247 tests with nine ignored; all 516 xtask tests and its integration test passed.
[Receipt](source-ci-receipt.json) pins the command, source, stages and private log hashes. Earlier
architecture and CLI-snapshot failures were repaired before this source passed.

A separate bounded transient service ran the exact committed-source executable with SHA-256
`4f601e93ccf74318950f62bea1a68425d33b2503ab80a4d3c9da28fde71507c2`, in observation-only mode,
with real BPF monitor events under the existing lightweight preset. Its kernel-authenticated Python
producer completed eight synthetic CPU workloads (deterministic 100000-step integer folds), each with an independently checked
checksum, and reported actual duration plus completed-work count. All 16 samples were accepted,
none denied; 21 controller ticks all emitted `observed`. Both required metrics were present with
compatible direct aggregation. The final target-exit ticks retired the source. Both temporary
services exited successfully, MainPID was zero, and the KPI socket was removed.
[Service summary](service-summary.json) includes source, binary and raw-artifact hashes.

Separately, `xtask kpi-observation run --out …` ran eight real fixed-work executions through the
runtime collector and its observation-only experiment-manager interface. Four counterbalanced pairs
of identical arms were preregistered, and raw blocks were persisted before admission/analysis.
Every aggregate and interval was reconstructed successfully from those captures. The frozen
classifier returned `Equivalent`: estimated improvement 4.19%, 95% interval -3.71% to +12.09%,
below the predeclared 20% benefit floor. This does **not** establish strict equivalence within 5%,
nor benefit from a tuning action: no setting differed between arms. This harness supplied monitor
ticks itself; the separate service check above supplied the actual live-monitor acceptance.
That first harness run used a development executable pinned by its own SHA in the private summary;
it is not an exact-`1d5a3ef0` production qualification receipt.
The CPU workloads are synthetic fixtures with actual measured timing, not real-game or application
benefit evidence.

The new checks also reject internally coherent but fabricated intervals, legacy batch-p99-as-mean
claims, raw-block replay, pre-design capture, incomplete/ambiguous source data, stale identities,
work/context drift and hard-guardrail regression. Losing/reacquiring a target changes the source
epoch, so an unfinished study cannot silently transfer. Observation results do not enter the
existing manager's keep evidence, candidate memory, support registry or privileged-worker APIs.

The production controller/worker remained at PIDs 1985/1889 with their original 02:43:56 UTC start
times. No restart, activation, reboot, GPU/profile write or privileged operation was performed by
these campaigns. No campaign worker/token was acquired. NixosConf's deployment branch remains
`cd5b8705`, pinning Stutter `e37984c1` and its existing executable hash. The final co-pin, guarded
activation and exact-binary production smoke remain W19 work after the implementation gates.

The separate frame-source investigation still lacks the unchanged 2 ms production-clock proof;
see [frame results](../controller-frame-2026-11-08/RESULTS.md). Network/HTTP outcomes, remaining typed
operation families, physical qualification and final deployment are not established here.
Private artifacts are retained in `.llm-runs/source-ci-1d5a3ef0`,
`.llm-runs/kpi-service-20261108` and `.llm-runs/kpi-observation-20261108`.
