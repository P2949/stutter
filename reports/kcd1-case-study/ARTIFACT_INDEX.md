# KCD1 case-study artifact index

This directory contains the technical archive for the Kingdom Come: Deliverance 1
case study. The polished lecturer-facing report is `KCD1_EXPERIMENT_REPORT.md`;
the primary technical archive summary is `CASE_STUDY_SUMMARY.md`.

## Primary narrative

- `KCD1_EXPERIMENT_REPORT.md` — polished lecturer-facing case-study report.
- `CASE_STUDY_SUMMARY.md` — summary of the method, results, limitations, and conclusions.
- `setup/kcd1-method-notes.md` — controlled route and setup notes.
- `setup/system-info.txt` — captured machine/software context.
- `setup/kcd1-config/` — archived KCD1 `system.cfg` and `user.cfg`.
- `setup/build-check.txt` — validation command output for the repo state used by the case study.
- `setup/cli-help.txt` — relevant CLI help output.

## Setup and process selection

- `setup/live-process-candidates.txt` — live process search output used while selecting the KCD1 process tree.
- `setup/tree-candidates/` — candidate `inspect-tree` outputs from early process-tree selection.
- `setup/baseline-*-tree-check.txt` — final tree checks for the formal baseline runs.
- `setup/launch-smoke-test.txt` — launch/capture smoke-test notes.
- `setup/kcd1-config-summary.txt` — summary of archived KCD1 config files.

## Formal baseline evidence

- `runs/baseline-01` through `runs/baseline-05` — five formal baseline run directories.
- `runs/baseline-*-analysis.json` — analysis JSON for each baseline.
- `runs/baseline-*-postcheck.txt` — basic validity checks for each baseline.
- `advisor-baseline-*.json` — advisor outputs used to form the CPU-affinity hypothesis.
- `mangohud/baseline-*.csv` — archived MangoHud CSVs for the formal baseline runs.

## Affinity A/B evidence

- `profiles/kcd1-affinity-ab.toml` — final A/B profile file.
- `tune/kcd1-affinity-02/` — paired A/B tune run; not counterbalanced because baseline ran before tuned in every iteration.
- `tune/kcd1-affinity-02/tuning_summary.json` — primary source for profile-vs-profile candidate statistics.
- `tune/kcd1-affinity-02/tuning_recommendation.md` and `.html` — generated recommendation outputs.
- `results/kcd1-fix-validation.md` and `.html` — secondary fix-validation output; status is `InvalidExperiment`, so it is not the primary source for the tuned-profile conclusion.
- `results/kcd1-fix-validation.json` — machine-readable fix-validation JSON.
- `results/kcd1-fix-validation-command-output.txt` — stdout paths emitted when the fix-validation artifact was generated.

## Profile explainability

- `profiles/kcd1-affinity-profile-plan.txt` — human-readable profile-plan output.
- `profiles/kcd1-affinity-profile-plan.json` — full JSON profile-plan artifact.
- `profiles/kcd1-affinity-profile-plan-summary.json` — compact rule/task matching summary.
- `profiles/kcd1-affinity-dry-run.txt` — earlier dry-run output retained for comparison.

## Measurement-quality pilot

- `drop-counter-pilot/mapfactor-4-analysis.json` — analysis for the wakeup-map-factor pilot.
- `drop-counter-pilot/mapfactor-4-comparison.txt` — comparison showing map factor 4 did not reduce wakeup replacements.
- `drop-counter-pilot/mapfactor-4-record.log` — recording command output.

## Exploratory real-world stack comparison

- `realworld-stack/README.md` — scope of the exploratory bundle comparison.
- `realworld-stack/realworld-stack-summary.csv` — compact clean vs personal-stack metrics.
- `realworld-stack/setup/launch-options.md` — exact launch options and `scx_lavd` command.
- `realworld-stack/ARTIFACT_NOTES.md` — notes about missing raw MangoHud CSVs for two clean runs.
- `realworld-stack/runs/clean-*` — clean-stack run artifacts.
- `realworld-stack/runs/personal-stack-*` — personal-stack run artifacts.
- `realworld-stack/mangohud/` — raw MangoHud CSVs that were still available when finalized.

## Notes

Some files with `.json` extension are newline-delimited event streams rather than
single JSON documents. Some optional stream files may also be empty when that
signal was enabled but no samples were recorded. They are preserved as produced
by `stutter record`.

Large `migration_events.json` files are intentionally excluded from Git.
