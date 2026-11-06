# Frame campaign, design v1: unproven (calibration failed, holdout never run)

Pre-registered design: interference as a nice-value ladder (19, 10, 5, 0) for one busy loop sharing the
vkcube CPU ([frame-graduation-v1.toml](frame-graduation-v1.toml)). Twelve calibration blocks ran
([calibration-runs.ndjson](calibration-runs.ndjson)); no population (300 to 2400 frames) and no
threshold (10 to 30 percent) satisfied the selection rule (`unproven.json`): the largest neutral change was
33 to 64 percent of the frame p99, and levels nice 5 and nice 10 both saturate near 1.1 ms (the scheduler
quantum), so adjacent rungs were not separable. The holdout blocks were never run. Nothing was relaxed.

Design v2 (the current `docs/validation/frame-graduation.toml`) was chosen after seeing this calibration
data: interference is the number of full-priority busy loops (0, 1 and 3), which separates the frame p99
by 3x to 10x. It uses fresh runs and a fresh, never-seen holdout, and it can only support a claim about
large frame-p99 effects.
