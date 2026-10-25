# Diagnosis corpus v2 and recalibration (post-v0.1)

This records the post-v0.1 diagnosis evidence study. The v0.1 corpus and its calibration report
are historical and unchanged (they produced recall 0.0 in every family). The study's retained
reports are under [docs/evidence/diagnosis-v2/](evidence/diagnosis-v2/); the raw recordings stay
with the operator and are pinned by `calibration/fixtures-manifest.json` (SHA-256 of each
fixture's `fixture.toml` and its capture id). Diagnosis and the advisor remain **Experimental**.

## Design

* [`docs/validation/scenarios/diagnosis-v2.toml`](validation/scenarios/diagnosis-v2.toml) defines
  matched pairs per family: the same target and configuration with and without a predeclared
  intervention, captured in counterbalanced AB/BA order. Ground truth is the intervention and its
  independent verification, never a diagnosis result. Generated fixtures are
  `expected_behavior = "informational"` so false negatives can be measured.
* Placement is real: the scheduler target runs under an explicit CPU mask, the scheduler contender
  runs on the same CPU, and the IRQ target runs on the CPU that services the frozen NIC interrupt
  (kernel effective affinity cross-checked with a per-CPU counter increase).
* A family is eligible for calibration only after an independent target-impact test on a *direct*
  metric of the target (never a diagnosis): bootstrap CI excluding zero in the worsening direction
  and a non-trivial effect size, from 15 matched pairs, extended in steps to 30 if underpowered.
  Otherwise the family is closed as invalid for calibration and no threshold is touched.
* Every capture first waits for storage to settle (the previous capture's writeback leaked into
  matched pairs before this was added: fixed-work controls alternated 85 s / 135 s).

## Target impact (retained: `impact/`)

| Family | Direct metric | Control median | Positive median | Effect size | Result |
| --- | --- | --- | --- | --- | --- |
| Scheduler | target runnable p99 | 0.5 ms | 5 ms | -17.1 | Pass (15 pairs) |
| I/O | fixed-work elapsed time | 19.9 s | 132.3 s | -30.8 | Pass (15 pairs) |
| GPU | target frame p99 | 0.164 ms | 0.328 ms | -5.0 | Pass (15 pairs) |
| IRQ | target frame p99 (unpaced target) | 0.258 ms | 0.257 ms | 0.005 | **Invalid** at the 30-pair cap |

The IRQ family was tried with three designs. A single 4 kpps sender on a vsynced scene left frame
time unchanged (7.79 vs 7.82 ms); ~1M interrupts per capture on that scene left it unchanged too
(idle time absorbs them); an unpaced, CPU-bound target showed a small but real difference that the
plan's unpaired test does not call significant (23 of 30 matched pairs worsened, median paired
difference 0.026 ms; this paired observation is exploratory and was not used for any decision).
The family is therefore not calibrated. The host was also in use during collection (a game kept
the GPU at 25-35% and a core busy), which raises the noise floor; counterbalancing is the
mitigation, and the GPU intervention needed two stressors to stay above its 90% busy bar.

## Calibration protocol

Captures are split by `sha256(capture_id)` within each scenario (2:1, so 15 pairs leave the
calibration part with the evidence floor of 10 positives and 10 controls); the split is persisted
(`calibration/split.json`) and never reshuffled. Thresholds are chosen on the calibration part only
(no new false-high claims, then precision, then recall, then the smallest change) and replace the
current default only if the untouched holdout also improves recall without lower precision, more
false positives, more false-high claims or worse mixed behavior.

A first run on a 50/50 split (`baseline-*-50-50.json`) found recall 0.0 at every threshold in all
three families, which is a diagnosis-logic gap, not a threshold problem.

## The logic gap and its correction

*Scheduler.* The recorded target (`vkmark`) is classified as a generic `Service`, and the scheduler
candidate only considers game-class tasks, so verified, large scheduler delays produced no
diagnosis. The smallest correction: tasks of an explicitly named process (`--tree-pid`/`--pid`)
whose class is only a generic fallback (`Service`, `Helper`, `Render`, `Unknown`) are treated as the
target; compositor, audio, browser, kernel, build and other specialised classes are never
reclassified. The change was designed on the calibration part only (a calibration-only mode never
loads the holdout): recall 0.0 -> 0.9 at the unchanged 2 ms threshold.

*I/O and GPU.* Their positives produce **no stutter clusters at all** (the fio target has no frames
or scheduler spikes; GPU contention left frame pacing smooth), so no threshold of the sweep can
recover them: Stutter diagnoses stutter events, and these interventions degrade throughput or
latency without hitching. This is a design mismatch, recorded as a limitation; no new heuristic was
added.

## Final holdout evaluation (once, after the code was frozen)

| Family | Current threshold | Calibration preferred | Decision | Holdout at the current threshold |
| --- | --- | --- | --- | --- |
| Scheduler | 2 ms | 4 ms (precision 1.0, recall 0.3) | **retain 2 ms** (holdout recall does not improve) | recall 1.0, precision 1.0, 0 false positives (5 positives, 5 controls) |
| I/O | 1 ms | current | retain; logic gap | recall 0 (no clusters) |
| GPU | 95 % | current | retain; logic gap | recall 0 (no clusters) |
| IRQ | 0.25 ms | - | not calibrated | - |

No threshold changed. Mixed scheduler+I/O captures produced no diagnosis (the fio target has no
clusters). Diagnosis remains Experimental; graduation is a separate future decision.
