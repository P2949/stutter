"""
# Plan 5 — Build one internal event pipeline

Current problem: events are not routed uniformly. Some go to sinks, some to recorder, some to external bus, some to autotune.

Steps:

1. Define `DaemonEvent` or promote `MonitorEvent` to the universal internal event type.
2. Create one internal event bus:

   * producer side: eBPF, focus, foreground, frame, hwmon, SCX, timers, agent commands.
   * consumer side: recorder, autotune, health monitor, status cache, sinks, remote stream.
3. Remove direct sink dispatch from event producers.
4. Remove manual “record here, emit there” patterns.
5. Add event delivery classes:

   * reliable,
   * conflated,
   * droppable,
   * audit-critical.
6. Add backpressure policy:

   * drop noisy samples,
   * never drop rollback/audit/fault events,
   * count and expose dropped events.
7. Add tests for event fanout:

   * recorder receives expected events,
   * autotune receives expected events,
   * status cache updates,
   * droppable events are dropped under load.

Done when:

* Every daemon-visible state change flows through one event pipeline.

# Plan 6 — Finish decomposing `MonitorSession`

Current problem: `MonitorSession::new` and `run` still own too much setup and control flow.

Steps:

1. Split initialization into builders:

   * `TargetRuntime::build`
   * `ProbeRuntime::build`
   * `RecordingRuntime::build`
   * `OutputRuntime::build`
   * `FocusRuntime::build`
   * `ForegroundRuntime::build`
   * `TelemetryRuntime::build`
2. Split event-loop handlers:

   * `handle_bpf_event`
   * `handle_summary_tick`
   * `handle_focus_tick`
   * `handle_foreground_tick`
   * `handle_watch_tick`
   * `handle_scx_tick`
   * `handle_hwmon_tick`
   * `handle_shutdown`
3. Make each handler independently testable with fake state.
4. Remove inline construction of exporters/recorders/alerts from session core.
5. Make session return structured exit reasons, not strings.
6. Add tests for startup failure localization:

   * eBPF load failure,
   * recorder path failure,
   * hwmon unavailable,
   * invalid target,
   * TUI init failure.

Done when:

* Session startup and event handling are boring, small, and testable.

# Plan 7 — Finish splitting `recorder.rs`

Current problem: recording is feature-rich but structurally too centralized.

Steps:

1. Keep `recorder/mod.rs` as public re-export layer.
2. Move event schemas to `recorder/event_types.rs`.
3. Move file/stream writers to `recorder/writers.rs`.
4. Move `LiveRecorder` and live counters to `recorder/live.rs`.
5. Move session metadata/finalization to `recorder/session.rs`.
6. Move persisted JSON schemas to `recorder/schema.rs`.
7. Keep `spike_buffer.rs`.
8. Add schema snapshot tests to prevent accidental output format breakage.
9. Add retention and disk-budget controls:

   * max run count,
   * max total bytes,
   * max age,
   * emergency stop when disk low.

Done when:

* Recording is safe for months of daemon use without unbounded disk/memory growth.

# Plan 8 — Finish reducing `events.rs` to dispatch-only

Current problem: `events.rs` still owns decode helpers, artifact pushing, cause tags, and dispatch.

Steps:

1. Move `read_event_unaligned` into `events/decode.rs`.
2. Move cause-tag priority policy into `events/cause.rs` or `events/interpret.rs`.
3. Move `push_artifact_event` into `artifacts.rs` or `recorder/live.rs`.
4. Keep only event dispatch and record construction in `events.rs`.
5. Move tests beside the functions they test.
6. Make event interpretation pure:

   * input raw event + task state,
   * output `MonitorEvent`s,
   * no sink dispatch.
7. Add fuzz/property tests for raw event decoding boundaries.

Done when:

* `events.rs` is not a dumping ground and decoding/interpretation/recording are separate.

---

# Plan 9 — Turn output sinks into a real registry

Current problem: sink dispatch is partly cleaned up but still enum-based and not fully dynamic.

Steps:

1. Decide:

   * true trait-object registry,
   * or explicit enum registry.
2. For daemon extensibility, choose trait-object registry.
3. Build sinks once at runtime startup.
4. Include sinks conditionally:

   * recorder if recording enabled,
   * Prometheus if metrics enabled,
   * OTel if endpoint enabled,
   * alert if alert channel enabled,
   * TUI only for interactive mode.
5. Remove no-op `TuiSink` unless it becomes functional.
6. Make sink failures policy-driven:

   * recorder failure may fault daemon,
   * Prometheus failure may degrade metrics only,
   * alert failure should not stop tuning.
7. Add sink health reporting.
8. Add tests for partial sink failure.

Done when:

* Output failure cannot silently poison daemon behavior.

# Plan 10 — Unify autotune controller paths

Current problem: the controller is default-enabled, but fallback/stub paths still exist.

Steps:

1. Delete `ObservePolicyStub`.
2. Remove `#[cfg(not(feature = "autotune-controller"))]` behavior branches.
3. Make controller runtime unconditional.
4. Keep feature flags only for optional integrations, not core behavior.
5. Route observe, suggest, and apply-low-risk through the same runtime.
6. Rework tests around the real controller.
7. Add compatibility aliases if old commands exist.

Done when:

* There is one autotune brain.

# Plan 11 — Generalize rollback and crash recovery across all action types

Current problem: rollback tokens exist for many action types, but startup recovery currently gives strong support mainly to CPU-affinity restore files. The startup recovery code explicitly bails for unsupported rollback token kinds in crash recovery. 

Steps:

1. Define `RollbackExecutor` for every `RollbackToken` variant:

   * CPU affinity,
   * nice,
   * ionice,
   * IRQ affinity,
   * uclamp,
   * cgroup,
   * CPU power,
   * VM knob,
   * GPU power,
   * sysfs.
2. Make startup recovery use the same rollback registry as normal runtime rollback.
3. Require every applied action to persist:

   * action ID,
   * rollback token,
   * affected identity,
   * restore command,
   * verify method,
   * safety class.
4. Add “rollback dry run” for every token.
5. Add “manual restore command” for every token.
6. Add tests for each token:

   * rollback success,
   * rollback failure,
   * missing target,
   * identity mismatch,
   * partial restore.
7. For unsupported rollback types, block apply before action starts.

Done when:

* Any action that can be applied can be restored after process crash or reboot.

# Plan 12 — Harden the action execution framework

Current strength: the action model already has `preflight`, `dry_run`, `apply`, `verify`, and `rollback`, with `SafetyClass`, `ActionPhase`, `ActionError`, and `RollbackToken`. That is the right foundation. 

Steps:

1. Make all tuning actions run through one `ActionRunner`.
2. Enforce phase order:

   * preflight,
   * dry run,
   * journal applying,
   * apply,
   * journal applied,
   * verify,
   * emit event.
3. Require identity validation before apply:

   * PID/TID still exists,
   * comm matches,
   * starttime matches,
   * cgroup/process tree still matches.
4. Require post-apply verification.
5. If verification fails:

   * rollback immediately,
   * verify rollback,
   * enter cooldown/fault depending on result.
6. Add per-action timeout.
7. Add per-action affected scope limit.
8. Add per-action risk budget.
9. Add audit events for every phase.
10. Add fake action tests for all failure modes.

Done when:

* No code path can apply tuning outside the action runner.

# Plan 13 — Build a daemon policy engine, not just controller decisions

Current problem: controller decisions decide candidate transitions, but daemon-level policy should decide whether action is allowed at all.

Steps:

1. Add `DaemonPolicyEngine`.
2. Inputs:

   * mode,
   * user config,
   * current target,
   * action candidate,
   * data quality,
   * thermal/power state,
   * battery/AC state,
   * active rollback state,
   * cooldown state,
   * user allow/deny lists.
3. Outputs:

   * allow,
   * reject,
   * delay,
   * require observe-only,
   * require manual confirmation.
4. Every rejection must include a machine-readable reason.
5. Add policy tests:

   * no action with low confidence,
   * no action with missing rollback,
   * no system-wide action in low-risk mode,
   * no action during thermal emergency,
   * no action during target instability,
   * no action while previous rollback pending.
6. Expose `stutter daemon policy explain`.

Done when:

* The daemon can always explain why it did or did not tune.

# Plan 14 — Strengthen workload identity and target stability

Current problem: autonomous tuning only works if the daemon knows what workload it is optimizing and when that workload changed.

Steps:

1. Define `WorkloadIdentity`:

   * root PID,
   * process starttime,
   * executable path,
   * command line hash,
   * cgroup path,
   * foreground app/window identity,
   * Steam/game identifier if detectable.
2. Track `WorkloadSession`:

   * started,
   * active,
   * idle,
   * changed,
   * ended.
3. Reset experiments when workload identity changes.
4. Roll back active experiments when:

   * focus switches,
   * root PID exits,
   * foreground app changes,
   * process identity mismatch occurs,
   * system sleeps/resumes.
5. Cache known-good profiles per workload identity.
6. Never apply a previously kept candidate to a different workload without revalidation.
7. Add tests with fake process trees:

   * PID reuse,
   * process restart,
   * foreground switch,
   * helper process exit,
   * game launcher → game process transition.

Done when:

* The daemon never applies stale tuning to the wrong process.

# Plan 15 — Improve candidate generation from “profiles” to “safe search space”

Current problem: low-risk CPU-affinity candidates exist, but a daemon needs a well-defined candidate universe with constraints.

Steps:

1. Create `CandidateProvider` trait.
2. Providers:

   * CPU affinity topology provider,
   * nice/priority provider,
   * ionice provider,
   * uclamp provider,
   * IRQ affinity provider,
   * power/thermal provider,
   * game-specific profile provider.
3. Every candidate must include:

   * action kind,
   * safety class,
   * affected scope,
   * rollback requirement,
   * expected benefit,
   * known risks,
   * prerequisites,
   * conflicts.
4. Add conflict detection:

   * two candidates cannot fight over same CPU masks,
   * power-save and performance boost cannot conflict,
   * IRQ pinning must not collide with isolated cores unless policy allows it.
5. Add candidate dry-run scoring.
6. Add candidate cooldown memory.
7. Add candidate blacklist after repeated failure.
8. Add candidate allowlist for daemon default mode.

Done when:

* The daemon searches a controlled safe space, not ad hoc actions.

# Plan 16 — Make experiment design statistically sane

Current problem: “measure before/after” can be fooled by scene changes, loading screens, thermals, shader compilation, network events, and random game variance.

Steps:

1. Define experiment phases:

   * warm-up,
   * baseline,
   * apply,
   * settle,
   * candidate measurement,
   * compare,
   * keep/revert.
2. Add minimum sample thresholds:

   * interval count,
   * scored samples,
   * frame count,
   * target presence duration.
3. Detect invalid windows:

   * loading screen,
   * target disappeared,
   * low activity,
   * high drop counters,
   * focus changed,
   * thermal throttling,
   * compositor/GPU reset,
   * background compile/update spike.
4. Compare multiple metrics:

   * scheduler latency,
   * frame p99/p99.9,
   * frame max,
   * stutter count,
   * input-sensitive runnable latency,
   * CPU migration count,
   * block I/O spikes,
   * IRQ interference.
5. Require improvement margin above noise floor.
6. If uncertain, revert or continue observing, not keep.
7. Add “A/B retry” for candidates that look promising but uncertain.
8. Persist experiment evidence in history.

Done when:

* The daemon keeps a candidate only when the evidence is strong enough.

# Plan 17 — Add confidence model and “do nothing” bias

Current problem: autonomous tuning must be conservative. The default answer should be “do nothing” unless there is strong evidence.

Steps:

1. Add `DecisionConfidence`.
2. Inputs:

   * focus confidence,
   * data quality,
   * sample size,
   * workload stability,
   * candidate history,
   * rollback reliability,
   * effect size,
   * system health.
3. Require minimum confidence by mode:

   * observe: no threshold,
   * suggest: medium,
   * apply-low-risk: high,
   * apply-medium-risk: very high,
   * high-risk: manual only.
4. Add “uncertain” state distinct from failure.
5. Add explicit no-op reasons:

   * insufficient data,
   * workload unstable,
   * candidate recently failed,
   * cooldown active,
   * rollback unavailable,
   * target not present.
6. Surface confidence in status API and CLI.
7. Test that low confidence never applies.

Done when:

* The daemon is boring and conservative by design.

# Plan 18 — Add thermal, power, and system-health guardrails

Current problem: performance tuning can trade against thermals, power, and system stability.

Steps:

1. Add `SystemHealthMonitor`.
2. Inputs:

   * CPU temperature,
   * GPU temperature,
   * fan/thermal zones,
   * CPU freq/throttle state,
   * GPU clocks if available,
   * AC/battery state,
   * memory pressure/PSI,
   * load average,
   * disk free space,
   * eBPF drop counters.
3. Define health states:

   * healthy,
   * degraded,
   * overheated,
   * low disk,
   * low memory,
   * instrumentation broken,
   * suspended/resumed.
4. Policy:

   * never apply during degraded health unless action explicitly fixes degradation,
   * rollback on overheating if action may contribute,
   * pause experiments after resume,
   * fault on repeated instrumentation failure.
5. Record health alongside experiments.
6. Expose health in `/health` and status commands.

Done when:

* The daemon stops tuning when the machine is not in a trustworthy state.

# Plan 19 — Create persistent daemon state store

Current problem: history/journal/status exist, but daemon state should be formalized.

Steps:

1. Create `DaemonStateStore`.
2. Store:

   * current daemon version,
   * config hash,
   * active workload,
   * active experiment,
   * active rollback token,
   * kept candidates,
   * rejected candidates,
   * cooldowns,
   * last health state,
   * last clean shutdown marker.
3. Use atomic writes.
4. Use schema versioning.
5. Add recovery path for corrupt state:

   * keep backups,
   * enter safe observe-only mode,
   * never apply if state uncertain.
6. Add `stutter daemon doctor`.
7. Add `stutter daemon reset-state --dry-run`.

Done when:

* Daemon restart does not forget important safety context.

# Plan 20 — Upgrade controller journal into transaction log

Current strength: controller journal exists and startup recovery can recover applied CPU-affinity actions. 

Steps:

1. Expand journal phases:

   * clean,
   * planned,
   * preflighted,
   * applying,
   * applied,
   * verifying,
   * measuring,
   * keeping,
   * reverting,
   * reverted,
   * faulted.
2. Record full action metadata:

   * candidate,
   * workload identity,
   * target identity,
   * rollback token,
   * verify result,
   * timing.
3. Make every state transition atomic.
4. On startup:

   * read transaction log,
   * decide recovery action,
   * execute rollback if required,
   * write recovery event.
5. Add migration for old journal format.
6. Add fault-injection tests:

   * crash before apply,
   * crash after apply before journal,
   * crash after journal before verify,
   * crash during rollback,
   * corrupt journal.

Done when:

* Crash recovery is transactional, not best-effort.

# Plan 21 — Build status and explainability as first-class features

Current problem: a daemon must make users trust it.

Steps:

1. Define `DaemonStatus`.
2. Include:

   * mode,
   * active workload,
   * active target,
   * current phase,
   * active candidate,
   * last decision,
   * last no-op reason,
   * current score,
   * baseline score,
   * data quality,
   * health state,
   * rollback availability,
   * cooldown remaining,
   * last fault,
   * manual restore command.
3. Add:

   * `stutter daemon status`,
   * `stutter daemon status --json`,
   * `/daemon/status`,
   * `/daemon/explain`.
4. Keep last N decisions in memory.
5. Add human-readable reason strings.
6. Add machine-readable reason codes.
7. Add “what changed on my system?” command.
8. Add “why did you not optimize?” command.

Done when:

* A user never has to guess what the daemon is doing.

# Plan 22 — Productize the local agent into the daemon control plane

Current strength: the agent already exposes health/version/capabilities, record start/stop/status, autotune start/stop/status/restore/history/config, runs, and artifact access. It also has bind/auth safety logic. 

Steps:

1. Rename or layer:

   * `agent` = HTTP/API control plane,
   * `daemon` = local service runtime.
2. Keep agent optional but enabled for local control.
3. Add daemon-native endpoints:

   * `/daemon/status`,
   * `/daemon/policy`,
   * `/daemon/health`,
   * `/daemon/restore`,
   * `/daemon/pause`,
   * `/daemon/resume`,
   * `/daemon/explain`.
4. Add API auth levels:

   * read-only,
   * control observe/suggest,
   * apply low-risk,
   * admin restore/reset.
5. Keep apply endpoints loopback-only by default.
6. Add request audit IDs.
7. Add rate limits.
8. Add CSRF/localhost browser threat mitigation if any browser UI is planned.
9. Add API compatibility/versioning.

Done when:

* The daemon has a stable, secure control plane.

# Plan 23 — Package as a real service

Current state: install docs say the project is currently technical-local-use packaging, not distro-ready. There are systemd units for agent and autotune observe/low-risk services, including an opt-in low-risk service that runs restore on service stop. 

Steps:

1. Add official service modes:

   * user observe service,
   * system observe service,
   * system low-risk service,
   * local agent service.
2. Add OpenRC service files, given your target ecosystem.
3. Add systemd hardening:

   * capability bounding,
   * private tmp,
   * protect system where possible,
   * state directory,
   * logs directory,
   * restart policy,
   * watchdog.
4. Define privilege model:

   * root daemon,
   * unprivileged UI/client,
   * minimal capabilities if possible.
5. Add install/uninstall behavior for:

   * binary,
   * config,
   * service units,
   * state directory,
   * logs.
6. Add `stutter service install --dry-run`.
7. Add `stutter service doctor`.
8. Add packaging for:

   * Gentoo ebuild,
   * Arch PKGBUILD,
   * generic tarball.
9. Add upgrade/migration scripts.

Done when:

* A user can install, enable, inspect, disable, and uninstall cleanly.

# Plan 24 — Add permissions and privilege separation

Current problem: eBPF and tuning actions often need privileges. A long-running privileged daemon must be minimized.

Steps:

1. Split into:

   * privileged worker,
   * unprivileged control/UI process,
   * optional local API.
2. Keep privileged worker small:

   * eBPF attach,
   * action apply/rollback,
   * protected state writes.
3. Move policy, UI, and reporting to unprivileged side where possible.
4. Use Unix socket for local control.
5. Require explicit auth for remote HTTP if enabled.
6. Add command allowlist between unprivileged and privileged parts.
7. Add audit for every privileged operation.
8. Add seccomp/profile possibilities later.
9. Add tests for privilege-denied behavior.

Done when:

* Most daemon logic does not need to run as root.

# Plan 25 — Add daemon-safe retention and resource budgets

Current problem: always-on means memory/disk/log growth must be controlled.

Steps:

1. Add memory budget:

   * max buffered events,
   * max active targets,
   * max retained intervals,
   * max diagnosis entries.
2. Add disk budget:

   * max run directory size,
   * max history size,
   * max audit size,
   * max artifacts size.
3. Add log rotation or truncation.
4. Add retention policy:

   * keep last N runs,
   * keep last N days,
   * keep faulted runs longer,
   * delete low-value observe runs first.
5. Add degraded mode when disk low.
6. Add metrics for current storage usage.
7. Add tests with fake full disk / write failures.

Done when:

* Leaving it running for months cannot fill the disk.

# Plan 26 — Add health watchdog and self-healing

Current problem: daemons fail in boring ways: lost events, stuck tasks, wedged probes, bad state, stale targets.

Steps:

1. Add periodic health tick.
2. Check:

   * ringbuf still alive,
   * event rate plausible,
   * drop counters below threshold,
   * target refresh working,
   * recorder writable,
   * action runner idle or progressing,
   * rollback journal clean or intentionally active.
3. Define self-healing actions:

   * restart monitor subsystem,
   * clear stale target,
   * pause autotune,
   * rollback active experiment,
   * enter observe-only,
   * fault hard.
4. Add watchdog timeout for experiments.
5. Add watchdog timeout for action phases.
6. Add watchdog timeout for shutdown rollback.
7. Expose watchdog status.

Done when:

* The daemon notices when it is blind, stuck, or unsafe.

# Plan 27 — Add suspend/resume and session-boundary handling

Current problem: laptops/desktops sleep, games restart, GPUs reset, display servers restart.

Steps:

1. Detect suspend/resume.
2. On suspend:

   * pause experiments,
   * optionally rollback active low-risk changes,
   * flush journal.
3. On resume:

   * refresh topology,
   * refresh target identity,
   * refresh eBPF maps,
   * clear measurement windows,
   * wait stabilization period.
4. Detect:

   * GPU reset,
   * compositor restart,
   * target PID restart,
   * cgroup moved,
   * CPU topology online/offline changes.
5. Add tests with simulated lifecycle events.

Done when:

* Sleep/resume does not produce stale tuning.

# Plan 28 — Build workload profile memory carefully

Current goal: if a candidate helps a game, the daemon should remember but not blindly trust it.

Steps:

1. Store kept candidates per workload identity.
2. Store:

   * hardware fingerprint,
   * kernel version,
   * Mesa/driver version if relevant,
   * scheduler/scx state,
   * CPU topology hash,
   * workload identity hash.
3. Invalidate profile memory when environment changes.
4. Revalidate kept candidate periodically.
5. Decay old confidence over time.
6. Keep separate profiles for:

   * plugged in vs battery,
   * different monitor refresh/FPS caps,
   * different graphics settings if detectable,
   * different scheduler.
7. Add command:

   * `stutter daemon profiles list`
   * `stutter daemon profiles forget`
   * `stutter daemon profiles explain`.

Done when:

* The daemon learns without becoming dangerously overconfident.

# Plan 29 — Add action-family progression

Do not jump straight from CPU affinity to every knob. Expand in layers.

Steps:

1. Phase A: CPU affinity only.

   * current low-risk target.
2. Phase B: nice/ionice for target tree.

   * reversible,
   * process-scoped,
   * easy rollback.
3. Phase C: uclamp.

   * process-scoped,
   * verify kernel support,
   * rollback required.
4. Phase D: IRQ affinity.

   * device-scoped,
   * harder to attribute,
   * require stronger evidence.
5. Phase E: cgroup/cpuset.

   * more invasive,
   * require explicit opt-in.
6. Phase F: CPU power/VM/GPU/sysfs knobs.

   * system-wide,
   * not default,
   * require high confidence/manual policy.
7. For each phase:

   * implement action,
   * implement rollback,
   * implement verify,
   * implement dry-run,
   * add crash recovery,
   * add policy gate,
   * add test fixture,
   * add docs.

Done when:

* Each new optimization family is safe before the next one starts.

# Plan 30 — Create a simulation harness

Current problem: testing on the real machine is slow and dangerous.

Steps:

1. Build fake event stream generator.
2. Simulate:

   * stable game,
   * unstable game,
   * focus switch,
   * loading screen,
   * thermal throttling,
   * target disappearance,
   * candidate improves,
   * candidate worsens,
   * candidate neutral,
   * noisy metrics.
3. Feed fake `MonitorEvent`s into `AutotuneRuntime`.
4. Assert decisions:

   * no-op,
   * suggest,
   * start experiment,
   * keep,
   * revert,
   * cooldown,
   * fault.
5. Add golden histories.
6. Add randomized property tests:

   * never apply without rollback,
   * never keep without measurement,
   * rollback after target disappearance,
   * cooldown after failure.
7. Add regression corpus from real sessions.

Done when:

* Controller logic can be tested without eBPF or a live game.

# Plan 31 — Add fault-injection tests

Current problem: daemon reliability requires testing the ugly paths.

Steps:

1. Inject failures into:

   * eBPF load,
   * ringbuf read,
   * recorder write,
   * action preflight,
   * action apply,
   * action verify,
   * rollback,
   * journal write,
   * history write,
   * audit write,
   * API request handling.
2. Add fake actions for every phase.
3. Add fake rollback executor.
4. Test:

   * apply failure does not leave journal applied,
   * verify failure triggers rollback,
   * rollback failure enters faulted,
   * crash recovery handles applied journal,
   * corrupt journal blocks apply,
   * missing restore file blocks apply.
5. Run these in CI.

Done when:

* Failure behavior is as tested as happy-path behavior.

# Plan 32 — Add long-running soak tests

Current problem: set-and-leave requires time-based confidence.

Steps:

1. Create `stutter-soak` test mode.
2. Run fake daemon for:

   * 1 hour,
   * 8 hours,
   * 24 hours,
   * eventually 7 days.
3. Track:

   * memory,
   * file descriptors,
   * disk usage,
   * event queue growth,
   * task count growth,
   * history size,
   * CPU overhead,
   * wakeups/sec.
4. Add real-machine soak profile:

   * observe-only,
   * apply-low-risk with fake action,
   * apply-low-risk with CPU affinity on test process.
5. Fail if:

   * memory grows unbounded,
   * disk grows beyond policy,
   * event drops exceed threshold,
   * health degraded without explanation.

Done when:

* The daemon has evidence it can run for days.

# Plan 33 — Define overhead budgets

Current problem: a performance daemon must not cost more than it saves.

Steps:

1. Define budgets:

   * CPU overhead,
   * memory overhead,
   * wakeups/sec,
   * disk writes/min,
   * eBPF drops,
   * latency impact.
2. Add self-metrics.
3. Add benchmark command:

   * `stutter daemon bench-overhead`.
4. Add CI/perf regression tests where possible.
5. Add adaptive sampling:

   * high frequency only during active experiments,
   * lower frequency when idle,
   * disable expensive probes when not needed.
6. Add probe activation profiles:

   * observe-light,
   * diagnose,
   * experiment,
   * deep-debug.
7. Stop tuning if overhead exceeds budget.

Done when:

* The daemon is measurably cheap.


# Plan 34 — Improve data quality model

Current problem: controller decisions depend heavily on whether the data is trustworthy.

Steps:

1. Expand `OnlineDataQuality`.
2. Include:

   * sample count,
   * target presence,
   * focus confidence,
   * event drops,
   * frame count,
   * workload stability,
   * measurement duration,
   * health state,
   * probe availability.
3. Make data quality block actions centrally.
4. Add reason codes:

   * `insufficient_samples`,
   * `target_missing`,
   * `focus_low_confidence`,
   * `drop_counters_high`,
   * `workload_changed`,
   * `thermal_degraded`.
5. Persist quality in history.
6. Display quality in status.
7. Add tests proving bad quality blocks apply.

Done when:

* Bad measurements cannot cause tuning.

---

# Plan 35 — Add a policy DSL or config schema for users

Current problem: advanced users will want control, but unsafe free-form config is dangerous.

Steps:

1. Define stable TOML schema:

   * mode,
   * action families,
   * targets,
   * deny lists,
   * safety class,
   * thermal limits,
   * retention,
   * confidence thresholds.
2. Add schema validation.
3. Add `stutter config check`.
4. Add `stutter config explain`.
5. Add examples:

   * gaming desktop,
   * laptop battery-safe,
   * workstation compile-heavy,
   * observe-only.
6. Add per-workload overrides.
7. Add migration for schema versions.
8. Reject unknown high-risk fields unless `experimental = true`.

Done when:

* Users can safely configure the daemon without editing code.

---

# Plan 36 — Add “pause and emergency restore” as sacred paths

Current problem: if a daemon changes system state, restore must be obvious and reliable.

Steps:

1. Implement:

   * `stutter daemon pause`,
   * `stutter daemon resume`,
   * `stutter daemon restore`,
   * `stutter daemon emergency-restore`.
2. Make restore work even if daemon is not running.
3. Make restore discover all known rollback records.
4. Add dry-run restore.
5. Add restore summary:

   * restored,
   * skipped dead,
   * identity mismatch,
   * errors.
6. Add prominent manual restore command in every status response.
7. Add service stop hook to restore or leave according to policy.
8. Add panic handler / shutdown handler where possible.

Done when:

* The user always has a big red button.

---

# Plan 37 — Add daemon UI/UX layer after internals stabilize

Current problem: raw JSONL and CLI status are useful for technical users, but not enough for trust.

Steps:

1. Improve terminal status:

   * current mode,
   * active workload,
   * current score,
   * last action,
   * rollback status,
   * reason for no-op.
2. Add `watch` mode:

   * `stutter daemon watch`.
3. Add compact notifications:

   * only on action apply,
   * rollback,
   * fault,
   * restore needed.
4. Add quiet mode by default.
5. Add “explain last 10 decisions.”
6. Optional later: local web dashboard.

Done when:

* The daemon is understandable without reading logs.

---

# Plan 38 — Add security hardening for the agent

Current strength: agent already handles loopback checks, bearer auth, remote apply restrictions, and audit events. 

Steps:

1. Bind to Unix socket by default instead of HTTP TCP where possible.
2. Keep TCP loopback optional.
3. Require auth for any state-changing endpoint.
4. Separate read token from apply token.
5. Add rate limiting.
6. Add request size limits.
7. Add structured audit per request.
8. Add CORS disabled by default.
9. Add optional mTLS only if remote use becomes real.
10. Add security tests:

* unauthenticated apply rejected,
* non-loopback apply rejected,
* invalid bearer rejected,
* read-only token cannot apply.

Done when:

* A local web page or LAN client cannot casually tune the machine.

---

# Plan 39 — Add compatibility and environment detection

Current problem: eBPF, tracepoints, sysfs knobs, and process controls vary by kernel/distro.

Steps:

1. Create `CapabilityProbe`.
2. Detect:

   * kernel version,
   * BTF availability,
   * tracepoint availability,
   * perf permissions,
   * cgroup v2,
   * sched_ext/scx state,
   * uclamp support,
   * ionice support,
   * IRQ affinity permission,
   * GPU sysfs support.
3. Generate `DaemonCapabilities`.
4. Policy must disable unsupported actions.
5. Status must show unavailable features.
6. Add `stutter daemon doctor`.
7. Add tests with fake capability sets.

Done when:

* The daemon adapts to the machine instead of failing late.

---

# Plan 40 — Add “safe defaults” presets

Current problem: set-and-leave users need conservative defaults.

Steps:

1. Add daemon presets:

   * `observe-only`
   * `gaming-low-risk`
   * `gaming-laptop-safe`
   * `workstation-low-risk`
   * `debug-aggressive`
2. Default to observe-only.
3. Make apply-low-risk require explicit enable.
4. Preset controls:

   * action families,
   * probe set,
   * confidence threshold,
   * thermal guardrail,
   * retention,
   * notification level.
5. Document each preset.
6. Add tests proving presets map to expected policy.

Done when:

* Users can choose intent without understanding every knob.

---

# Plan 41 — Stabilize public/internal module boundaries

Current problem: modules exist, but architecture still leaks across boundaries.

Steps:

1. Define crate layers:

   * ABI,
   * probe loading,
   * event decoding,
   * observation,
   * policy,
   * action execution,
   * daemon runtime,
   * control plane,
   * reporting.
2. Add `pub(crate)` discipline.
3. Remove accidental public exports.
4. Add module-level docs for each major subsystem.
5. Add forbidden dependency checks manually or via architecture tests:

   * actions must not depend on CLI,
   * daemon runtime must not depend on CLI parse structs,
   * event decoding must not depend on recorder,
   * policy must not mutate state.
6. Add architecture tests with simple grep/rustdoc checks if needed.

Done when:

* The codebase prevents future coupling from creeping back in.

---

# Plan 42 — Create CI gates for daemon safety

Current problem: normal unit tests are not enough.

Steps:

1. Add CI jobs:

   * unit tests,
   * integration tests,
   * fake daemon simulation,
   * fault injection,
   * clippy,
   * formatting,
   * schema snapshot tests.
2. Add optional privileged CI/manual test suite:

   * eBPF smoke,
   * fake process tuning,
   * restore verification.
3. Add test categories:

   * safe no-root,
   * root required,
   * destructive/manual.
4. Add coverage for:

   * rollback,
   * journal recovery,
   * policy rejection,
   * target identity.
5. Add regression artifacts from real sessions.

Done when:

* A risky daemon change cannot merge without safety coverage.

---

# Plan 43 — Add release readiness gates

Steps:

1. Define release channels:

   * experimental,
   * observe-stable,
   * low-risk-stable.
2. Require for observe-stable:

   * no apply actions,
   * stable service,
   * retention controls,
   * health/status.
3. Require for low-risk-stable:

   * action runner mandatory,
   * universal rollback for enabled action families,
   * crash recovery,
   * soak tests,
   * service packaging,
   * docs.
4. Require for medium-risk:

   * per-action opt-in,
   * stronger tests,
   * manual confirmation or explicit config.
5. Add changelog categories:

   * safety,
   * tuning behavior,
   * rollback,
   * config migration.
6. Add `stutter --version --features`.

Done when:

* Users know what level of trust each release deserves.

---

# Plan 44 — Final “100% daemon” acceptance test

This is the final boss. Define one scenario suite that must pass before calling it done.

Steps:

1. Install service.
2. Start daemon in observe mode.
3. Detect active foreground workload.
4. Record baseline.
5. Enter apply-low-risk mode.
6. Apply only reversible candidate.
7. Verify action.
8. Measure improvement.
9. Keep only if improved.
10. Revert if worse.
11. Roll back on focus switch.
12. Roll back on process exit.
13. Survive daemon crash.
14. Restore on startup.
15. Survive system suspend/resume.
16. Enforce disk/memory limits.
17. Expose correct status throughout.
18. Reject unsafe remote apply.
19. Stop service cleanly.
20. Restore on service stop if policy requires.
21. Leave no stale system state behind.
22. Produce a complete audit/history trail.

Done when:

* You can run this suite repeatedly on a real machine and the daemon never leaves the system worse.

---

## The shortest useful execution order

If you want maximum progress per patch sequence:

1. Finish `MonitorConfig` runtime migration.
2. Split `recorder.rs`.
3. Clean `events.rs`.
4. Decompose `MonitorSession`.
5. Build one internal event bus.
6. Make the autotune controller unconditional.
7. Centralize action execution through `ActionRunner`.
8. Generalize rollback for every enabled action.
9. Build daemon runtime state machine.
10. Add daemon state store.
11. Add policy engine.
12. Add workload identity.
13. Add confidence/data-quality gates.
14. Add health/thermal/disk guardrails.
15. Add service lifecycle and OpenRC/systemd hardening.
16. Add simulation tests.
17. Add fault-injection tests.
18. Add soak tests.
19. Add status/explain UX.
20. Expand action families one at a time.

## My blunt strategic advice

Do **not** add more probes first. Do **not** add more risky tuning knobs first.

The next real advancement is to make the daemon **hard to fool, hard to wedge, and hard to leave in a bad state**.

The current code already has enough raw power to start hurting the system if the policy is wrong. The project becomes “daemon-grade” when the boring safety infrastructure becomes stronger than the optimizer.
"""