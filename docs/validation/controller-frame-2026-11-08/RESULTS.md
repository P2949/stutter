# Controlled workload attached to a development controller

This is `controlled_real` engineering evidence, not game benefit, G2/G3 qualification or a
production deployment. The actual controller ran in separate bounded transient services, in
`observe` mode, with profiling capabilities only. Production units were left running unchanged.
Exact executable hashes and local raw-artifact hashes are in [summary.json](summary.json).
These development builds have a recorded base/diff; they are not committed-source release receipts.

| Case | Observation | Disposition |
|---|---|---|
| Default diagnosis preset | Controller exited before its first tick: this kernel's `sched_stat_wait.pid` offset is 12; the compiled optional reader expects 8. MangoHud itself produced a log. | Failed startup retained; no measurement pass |
| Existing lightweight preset, one vsync-capped 640x480 vkcube | 62 controller ticks, 47 with an attached source; one unknown attribution then 46 renderer-attributed ticks; zero reported dropped frames, maximum 2325 frames in a window. Observed clock uncertainty 482.04–507.34 ms. | Diagnostic ingestion observed; **every populated window denied frame-primary eligibility** by the unchanged 2 ms limit |
| Second vsync-capped 320x240 vkcube renderer starts inside the same launch tree | 81 ticks, 67 attached; 56 attached ticks after the second launch, all denied direct attribution. First observed ambiguity about 39.7 ms after the second launch. Ambiguous-window taint remained set. | Live attribution withdrawal observed; no primary keep possible |

The workload launched through the existing `game-run` path after the controller started. Genuine
MangoHud CSV files grew during execution; the runtime resolved producer PID/start ticks, file owner,
source epoch, age and loss. The new optional `frame_source` decision-stream record exposes these
values and the primary-quality denial. It represents unknown clock bounds as null, never zero.
Both temporary controllers exited successfully in the lightweight cases; temporary services were
stopped, MainPID was zero, and their groups were empty/inactive. No worker was launched and no
operation or Stutter restore token was acquired. Collected kernel logs contained no GPU fault.

Two regressions were reproduced/fixed with fixtures: a renderer appearing 1 ms after a unique scan
previously retained `ValidatedDirect` for the five-second cache interval; a transient competitor
could also leave ambiguous historical rows available for later attestation. Competition is now
checked per bounded batch, and a seen ambiguity taints the window until a proven boundary drain.
Positive, PID-reuse, focus/target retirement, epoch/rotation, missing/stale source, loss and phase
boundary fixtures remain part of the telemetry/runtime suites.

Limits: the 2 ms alignment requirement was **not attained**, and the MangoHud producer/flush timing
is not an independently synchronized timestamp source. These observations cannot qualify a direct
frame-primary experiment. A source with a verified production-time/flush error bound is needed;
the threshold must not be relaxed to accept polling latency. Default-preset tracepoint compatibility
and further service/phase/crash acceptance remain open. No actuator campaign was run after the
measurement gate failed, and no unchanged historical negative pilot was repeated.

Private raw logs, capture records, unit states, scripts and patches are retained under the three
`.llm-runs/controller-frame-20261108*` directories. Hashes identify exactly which files supported
this report; a later exact committed build still needs its own consolidated acceptance.
