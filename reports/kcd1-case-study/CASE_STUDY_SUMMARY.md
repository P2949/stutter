# KCD1 Case Study Summary

This document summarizes the current Kingdom Come: Deliverance 1 case-study evidence collected with `stutter`. It is intended to be committed in the repo as:

```text
reports/kcd1-case-study/CASE_STUDY_SUMMARY.md
```

The purpose of this case study is not to prove that `stutter` automatically fixes KCD1, or that a specific Linux tuning tweak universally improves the game. The purpose is narrower and more useful: show that the prototype can collect evidence from a real Proton/Wine game workload, generate a scoped tuning hypothesis, and determine whether that hypothesis is validated, unsupported, or requires more evidence through repeated A/B measurements.

## Key takeaways

- This case study demonstrates that `stutter` can separate a plausible-but-unsupported tuning hypothesis from a validated improvement claim in this noisy KCD1/Proton workload.
- `stutter` successfully captured and analyzed a real KCD1/Proton workload with scheduler data, process-tree data, GPU samples, and MangoHud frame timing.
- Five formal baseline runs were valid: each ran for about 180 seconds, stopped because the maximum duration was reached, ingested frame data, and used monotonic frame timestamp alignment.
- The baseline route was noisy but useful: median frametime ranged from about 16.3ms to 22.9ms, while p99 frametime stayed in the mid-40s to low-50s milliseconds.
- The workload is noisy enough that small tuning effects may require roughly 18-30+ runs per condition, depending on the metric; the five-run A/B test is enough to show the workflow and avoid a false positive, but not enough to claim precise small-effect estimates.
- The advisor generated a plausible, reversible CPU-affinity hypothesis, but A/B tuning did not validate the tested profile.
- In the proper A/B tune run, `baseline-online` had a lower primary diagnostic score than the tuned affinity profile in all five paired iterations.
- The non-validation result is valuable: it shows that `stutter` can prevent a plausible Linux tuning tweak from being mistaken for a proven improvement.
- A measurement-quality pilot found that wakeup replacement counters were not ring-buffer reserve failures and were not reduced by `--ebpf-wakeup-map-factor 4`.
- A follow-up profile-explainability implementation now allows `stutter` to report which profile rules match which tasks, classes, `comm`, `process_comm`, match source, and proposed CPU masks before a profile is applied.
- An exploratory real-world stack add-on compared the stripped-down measurement setup against the author's normal gaming configuration bundle with `scx_lavd`; in this small sample, the personal stack did not show a clear frame-pacing advantage.

## Experiment scope

The experiment used a fixed Kingdom Come: Deliverance 1 route under Linux/Proton:

- Game: Kingdom Come: Deliverance 1
- Platform: Steam with GE-Proton10-34
- Session: Wayland/Sway with Gamescope
- Resolution: 1920x1080 through Gamescope
- Refresh / cap: 100 Hz output with 100 FPS MangoHud cap
- Route: fixed Rattay route from a fixed save
- Run length: 180 seconds per measured run
- Hardware: Intel Core i5-10600K, 6 cores / 12 threads; AMD Radeon RX 9070 XT
- CPU topology used by the profile design:
  - core 0: CPUs 0,6
  - core 1: CPUs 1,7
  - core 2: CPUs 2,8
  - core 3: CPUs 3,9
  - core 4: CPUs 4,10
  - core 5: CPUs 5,11

The large personal optimized launch configuration was intentionally not used. The measurement launch was kept stripped down: Gamescope, MangoHud logging, the fixed KCD1 config, and no RADV experimental flags, no FSR/FSR4, no gamemode, no mimalloc, and no forced Wine CPU topology.

The Steam launch still used `+exec user.cfg`. The archived `user.cfg` mainly sets memory, texture streaming, material preload, and pak stream-cache options. These settings are treated as part of the fixed workload configuration, not as the tuning variable.


## Reproducibility checklist

To reproduce this case study as closely as possible, keep the workload and measurement environment fixed:

- Use the same fixed Rattay route and save file described in the method notes.
- Use Steam with GE-Proton10-34.
- Launch KCD1 with the stripped-down measurement configuration: Gamescope and MangoHud logging, no RADV experimental flags, no FSR/FSR4, no gamemode, no mimalloc, and no forced Wine CPU topology.
- Keep `+exec user.cfg` enabled, because the archived `user.cfg` settings are part of the fixed workload configuration.
- Use 1920x1080 through Gamescope, 100 Hz output, and a 100 FPS MangoHud cap.
- Use the same route duration: 180 seconds per measured run.
- Re-detect the live Gamescope/KCD process-tree root before each recording; do not reuse a PID from a previous launch.
- Keep background load stable: no browser, Discord stream, downloads, package builds, or other heavy activity during measurement.
- Treat hardware differences as a limitation. The recorded result is specific to this Intel i5-10600K / AMD RX 9070 XT / Wayland-Sway / Gamescope / Proton configuration.


## How to read the diagnostic score

The primary comparison metric in the tune output is `diagnostic_raw_score_total`. This is an internal `stutter` weighted penalty score, not an FPS metric. Lower is better. It is used by the tune/recommend pipeline to compare profiles based on the scheduler-aware diagnostic evidence collected during each measured window.

In the frame-aware comparison path used for this case study, the score combines scheduler-latency threshold counts for relevant game/runtime classes with frame-time tail counts. The scheduler component is weighted as `over_5ms * 100 + over_2ms * 20 + over_1ms`; the frame component adds `frame_over_50ms * 100 + frame_over_33ms * 20 + frame_over_16ms`. Larger values therefore mean more or worse scheduler/frame-pacing outliers during the measured window.

Because it is an internal raw score, it should not be presented as a universal performance unit. It is useful inside one controlled experiment for comparing baseline and tuned candidates under the same scenario, route label, and measurement settings.

## Baseline data quality

Five formal baseline runs were collected. All five passed the basic validity checks:

- stop reason: `max_duration_reached`
- duration: about 180 seconds
- MangoHud frame data ingested
- frame timestamp alignment: `monotonic_observed`
- data quality: `Medium`

In this case study, `Medium` data quality means the measurements are usable and contain the required scheduler/frame evidence, but the tool observed expected real-world limitations such as noisy run-to-run frame pacing, estimated percentiles for some latency distributions, and wakeup replacement pressure. It does not mean the runs are invalid; it means the report should treat the evidence as valid but not lab-perfect.

| Run | Duration | Frames | Median frametime | P95 | P99 | Max | Outlier count | Data quality |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| baseline-01 | 180.761s | 8833 | 17.725ms | 41.919ms | 51.008ms | 562.266ms | 1347 | Medium |
| baseline-02 | 180.809s | 7744 | 21.276ms | 40.741ms | 49.712ms | 272.166ms | 1354 | Medium |
| baseline-03 | 180.666s | 9261 | 16.309ms | 41.402ms | 49.085ms | 563.179ms | 1303 | Medium |
| baseline-04 | 180.835s | 7421 | 22.811ms | 39.812ms | 46.778ms | 84.384ms | 1229 | Medium |
| baseline-05 | 180.746s | 7607 | 22.884ms | 38.402ms | 44.788ms | 87.387ms | 1098 | Medium |

The baseline set is valid but noisy. Median frametime varied from about 16.3ms to 22.9ms across repeated runs of the same route. P99 frametime was more stable, around 44.8ms to 51.0ms. This supports the report narrative that real open-world game benchmarking has substantial run-to-run variation, and that repeated A/B measurements are necessary before trusting a tuning claim.

## Advisor output

The advisor consistently produced a scoped tuning hypothesis from the baseline runs. All five baseline advisor outputs returned `TryProfileTuning` with `Medium` confidence and suggested a reversible local CPU-affinity profile experiment.

The evidence varied by run, but the recurring explanation was scheduler delay affecting game/proton threads such as `wineserver` or `Main`, with SCX disabled. This is a useful case-study result: the tool produced a concrete hypothesis, but did not claim the hypothesis was already proven.

## Tested CPU-affinity hypothesis

The tested profile was:

```toml
[[profile]]
name = "baseline-online"

[[profile.rules]]
affinity = "online"
match_class = ["Game", "GameHelper", "WineServer", "GameScope", "Compositor", "Launcher", "SteamRuntime", "Helper", "Unknown"]

[[profile]]
name = "kcd1-game-on-1-5-7-11-gamescope-on-0-6"

[[profile.rules]]
affinity = "1-5,7-11"
match_comm = ["Main"]

[[profile.rules]]
affinity = "1-5,7-11"
match_class = ["Game", "GameHelper", "WineServer"]

[[profile.rules]]
affinity = "0,6"
match_class = ["GameScope", "Compositor", "Launcher", "SteamRuntime"]
```

The hypothesis was that isolating Gamescope/runtime work on CPU pair `0,6` while placing the game/Wine process on CPUs `1-5,7-11` might reduce scheduler delay and frame-time tail latency.

The dry run confirmed that the profile was applyable and would change task affinity:

```text
checked_tasks=109
pending_affinity=109
total_pending_tasks=109
```

## Profile matching caveats

Two profile-matching details are important for reproducing and interpreting this case study:

> **Important:** Profile rules are first-match-wins. A broad rule such as `match_comm = ["Main"]` can match all threads in the KCD process through `process_comm = "Main"`, including worker threads such as `RenderThread`, `ClothingRaycast`, `Streaming Async`, and `dxvk-submit`. This is useful for coarse process placement, but it can also create unintended CPU placement if more specific rules are placed after the broad rule.

1. Profile rules are first-match-wins. Once a task matches an earlier rule, later rules do not get a chance to move it elsewhere.
2. `match_comm` currently checks both a task's own `comm` and its parent process `process_comm`.

This matters because KCD's process appears as `process_comm = "Main"`, while important worker threads appear with task names such as `RenderThread`, `ClothingRaycast`, `Streaming Async`, `AudioThread`, and `dxvk-submit`. A broad rule such as `match_comm = ["Main"]` can therefore match those worker threads through their process comm, even when their own task comm is different.

The original dry-run output showed that tasks would move, but it did not explain which rule matched which semantic thread. The follow-up profile-plan output fills that gap with rule-level summaries showing matched `comm`, `process_comm`, class, match source, original mask, proposed mask, and per-rule counts.

## Profile explainability follow-up

After the initial KCD1 A/B experiment, `stutter` gained profile explainability output through:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6
stutter apply-profile \
  --tree-pid <PID> \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6 \
  --dry-run \
  --explain
```

This reports rule-level matched task counts, pending affinity changes, top task `comm`, top `process_comm`, matched classes, match source (`task.comm`, `process_comm`, class, or catch-all), and broad `process_comm` captures.

Because the case-study TOML contains both `baseline-online` and the tuned affinity profile, the profile-plan artifact is generated with `--profile-name` so the audited profile is the tuned profile rather than the first profile in the file.

For this case study, that matters because the KCD process appears as `process_comm = "Main"`, while important worker threads use task names such as `RenderThread`, `ClothingRaycast`, `Streaming Async`, `AudioThread`, and `dxvk-submit`. The explainability output can now show whether those worker threads were matched by the broad `match_comm = ["Main"]` rule, rather than leaving that interpretation implicit.

## A/B tuning result

The second tuning experiment, `kcd1-affinity-02`, used a proper A/B profile set with both `baseline-online` and the tuned CPU-affinity profile. Each profile had five valid measured iterations.

The tuning summary selected `baseline-online` as the best profile with `Medium` ranking confidence. The tested tuned profile was not validated and should not be recommended on the current evidence.

| Profile | Valid runs | Median diagnostic score | Mean diagnostic score | Median frame P99 | Mean over_5ms |
|---|---:|---:|---:|---:|---:|
| `baseline-online` | 5 | 21,533 | 23,431.4 | 38.337ms | 0.2 |
| `kcd1-game-on-1-5-7-11-gamescope-on-0-6` | 5 | 38,806 | 49,746.0 | 37.798ms | 0.6 |

This table is derived from `reports/kcd1-case-study/tune/kcd1-affinity-02/tuning_summary.json` candidate statistics. The generated `tuning_recommendation.json` selected `baseline-online` as the best profile; therefore some formal comparison fields in that artifact compare the selected best profile against itself and report zero deltas. The profile-vs-profile conclusion here is based on the candidate scores in the tuning summary.

Lower diagnostic score is better. The tuned profile was worse on the primary diagnostic metric in every paired iteration:

| Iteration | baseline-online score | tuned score | Delta | Lower score |
|---:|---:|---:|---:|---|
| 1 | 19,579 | 32,052 | +63.7% | baseline-online |
| 2 | 21,533 | 34,566 | +60.5% | baseline-online |
| 3 | 16,643 | 44,845 | +169.5% | baseline-online |
| 4 | 26,408 | 38,806 | +46.9% | baseline-online |
| 5 | 32,994 | 98,461 | +198.4% | baseline-online |

A compact visual summary of the median diagnostic scores makes the profile non-validation visible at a glance:

```text
baseline-online median: 21,533  [=========           ]
tuned-profile median:   38,806  [================    ]  +80.2% worse
```

Frame P99 was mixed, but the primary scheduler-aware diagnostic score did not support the tuned profile. The correct conclusion is:

> The tested CPU-affinity profile was not validated and should not be recommended on the current evidence for this route and system. The tool prevented a plausible tuning hypothesis from being mistaken for a proven improvement.

This is a good result for the FYP narrative. The project is about evidence-based validation, not forcing a positive tuning result.

`reports/kcd1-case-study/results/kcd1-fix-validation.md` should be treated as a secondary validation artifact rather than the primary source for the tuned-profile conclusion. Its status is `InvalidExperiment`, and it compares the selected best tune candidate, `baseline-online`, against the earlier formal baseline set. The tuned-profile conclusion in this summary comes from the profile candidate statistics in `tuning_summary.json`.

## Why the affinity profile likely failed

The profile-plan artifact shows that the tuned profile did not simply miss the important KCD1 worker threads. `RenderThread`, `ClothingRaycast`, `Streaming Async`, `dxvk-submit`, and `dxvk-cs` were matched through `process_comm = "Main"` and would have been moved by rule 0.

The more likely explanation is that the profile reduced the game's available CPU set from all 12 logical CPUs to 10 logical CPUs while KCD1/DXVK/Wine had many active worker, render, streaming, physics, and audio threads. Reserving CPU pair `0,6` for Gamescope was plausible, but on this 6-core/12-thread machine the reduced scheduling capacity appears to have outweighed any presentation-thread isolation benefit.

This is an interpretation, not a proven causal mechanism. It is still useful because it is grounded in the profile-plan and A/B evidence: the same worker/render threads appeared in the spike evidence, but the tuned profile produced worse diagnostic scores under the tested route and configuration.

## Statistical interpretation

`tuning_recommendation.json` returned `NeedsRetest`, with `baseline-online` as the current best profile. The A/B uncertainty output warned that several metrics were noisy and that bootstrap confidence intervals crossed zero.

Estimated sample requirements included approximately:

- 30 runs per side for `diagnostic_raw_score_total`
- 18 runs per side for `frame_p99_ms`
- 24 runs per side for `frame_over_16ms`
- 30 runs per side for several tail/outlier metrics

This supports a key report point: KCD1 is a noisy real-world workload, and small tuning effects require many repeated runs to validate.

## Drop-counter / measurement-quality pilot

A separate measurement-quality check investigated the recurring non-zero drop-counter warning. The important finding is that the issue was not ring-buffer loss:

| Run | wakeup replacements/s | ringbuf reserve failures |
|---|---:|---:|
| baseline-01 | 1525/s | 0 |
| baseline-02 | 1436/s | 0 |
| baseline-03 | 1476/s | 0 |
| baseline-04 | 1557/s | 0 |
| baseline-05 | 1477/s | 0 |
| mapfactor-4 pilot | 1568/s | 0 |

The `--ebpf-wakeup-map-factor 4` pilot did not reduce `wakeup_data_replaced_entries`; it produced a similar replacement rate to the baselines. Therefore, the counter is probably not simple map-capacity exhaustion. It is more likely caused by repeated wakeups for the same target task before `sched_switch` consumes the previous wakeup timestamp.

Report wording:

> The case study exposed a measurement-quality nuance in the profiler. KCD1/Proton produced high wakeup replacement pressure, but ring-buffer reserve failures remained zero. Increasing wakeup-map capacity did not reduce the replacement rate, suggesting the counter may represent repeated wakeup churn rather than ordinary event loss. Future work should distinguish harmful measurement loss from benign replacement of superseded wakeup timestamps.

## Exploratory add-on: personal gaming stack

As a realism check, the case study also recorded a small exploratory comparison between the stripped-down clean measurement setup and the author's normal gaming configuration bundle.

This add-on compared:

- `clean`: stripped-down measurement launch configuration, default scheduler, 1920x1080 through Gamescope, 100 Hz output, and 100 FPS MangoHud cap.
- `personal-stack`: the author's usual launch-option bundle plus `scx_lavd` using the recorded aggressive gaming flags.

This is not a causal test of `scx_lavd`, Gamescope FSR, RADV/Mesa options, Wine/Proton options, allocator choice, gamemode, or any individual launch flag. The personal stack changes many variables at once, so the result is best treated as a realistic configuration-bundle comparison.

All six exploratory runs passed the basic validity gates: full-duration recording, `max_duration_reached`, non-zero frame count, `monotonic_observed` frame timestamp alignment, `Medium` data quality, and zero ring-buffer reserve failures.

| Condition | Runs | Median frametime | P95 | P99 | Max | Median outlier % | Scheduler |
|---|---:|---:|---:|---:|---:|---:|---|
| `clean` | 3 | 19.3419ms | 26.2893ms | 29.8236ms | 65.7529ms | 0.428% | default |
| `personal-stack` | 3 | 20.1032ms | 28.3838ms | 32.4916ms | 91.134ms | 0.826% | `scx_lavd` |

In this small sample, the personal stack did not show a clear frame-pacing advantage over the clean stack. Its condition-level median was worse on median frametime, P95, P99, max frametime, and outlier percentage. The strongest personal-stack outlier was `personal-stack-02`, with P99 `37.8372ms`, max frametime `181.596ms`, and outlier rate `4.814%`.

The correct interpretation is restrained:

> The personal gaming stack was captured successfully, but this exploratory sample did not show a clear frame-pacing improvement over the stripped-down measurement setup. Because the personal stack changes many variables at once, this result should not be used to attribute causality to `scx_lavd` or any individual launch flag. It is useful as evidence that `stutter` can capture and compare realistic player-used configuration bundles without turning the result into an unsupported tuning claim.

Artifact note: the raw MangoHud CSV used by `clean-01` and `clean-02` was no longer available when the archive was finalized. Their frame timing data had already been ingested into the committed `stutter` analysis JSON files, so the runs remain usable for this exploratory comparison. This limitation is documented in `reports/kcd1-case-study/realworld-stack/ARTIFACT_NOTES.md`.

## Additional limitations and future work

- The formal KCD1 runs used explicit `--tree-pid` targeting rather than foreground-window auto-selection. This was appropriate for the Gamescope/Proton process tree, but it means the case study should not be presented as a demonstration of foreground focus attribution. Foreground-window evidence was secondary; the validity of the recording comes from explicit KCD/Gamescope tree targeting.
- IRQ, KMS flip, and DRM fence correlation were not available in the formal baseline artifacts. Future KCD1 runs could use `stutter inspect-irqs` to identify relevant GPU/device IRQs, then record with `--irq-latency --irq <IRQ>` to test whether device interrupt overlap contributes to the observed tail latency.
- The profile-plan artifact was added after the first A/B experiment exposed the need for better explainability. Future case studies should run `profile-plan` before A/B data collection and include it as part of hypothesis formation.

## Current conclusion

This case study is already useful for the report:

1. `stutter` successfully recorded a real KCD1/Proton workload.
2. Five valid baseline runs were collected with scheduler and frame data.
3. The route showed real frame-pacing problems and meaningful run-to-run variance.
4. The advisor produced a reversible CPU-affinity tuning hypothesis.
5. A controlled A/B tuning run tested the hypothesis.
6. The tested profile was not validated and should not be recommended on the current evidence; `baseline-online` remained best.
7. A drop-counter pilot clarified that wakeup replacements were not ring-buffer drops and were not fixed by increasing wakeup-map capacity.
8. A follow-up profile-explainability feature now reports rule-level task matches, classes, `comm`, `process_comm`, and proposed masks so future tuning hypotheses are easier to audit before collecting A/B data.
9. An exploratory real-world stack add-on shows that `stutter` can capture and compare a realistic player-used configuration bundle, while still avoiding causal claims when many variables change at once.

The report should frame this as an evidence-based validation case study, not as a successful performance tweak. The most defensible conclusion is:

> For this machine, route, Proton version, and configuration, the tested CPU-affinity profile was not validated and should not be recommended on the current evidence. The value of `stutter` is that it made this conclusion visible through repeated measurements, rather than relying on anecdotal impressions.

In short, this case study validates the workflow rather than the tuning tweak. `stutter` collected evidence from a complex KCD1/Proton process tree, generated a scoped CPU-affinity hypothesis, tested it with repeated A/B measurements, and declined to recommend the hypothesis when the data did not support it. That ability to avoid false-positive tuning recommendations is a critical step toward reliable tuning advice. The profile explainability follow-up makes this kind of tuning result easier to audit and reproduce by showing which rules matched which tasks before any profile is applied.
