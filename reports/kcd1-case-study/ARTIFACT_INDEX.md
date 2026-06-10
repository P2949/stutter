# KCD1 case-study artifact index

This directory contains the lightweight, Git-tracked entry points for the
Kingdom Come: Deliverance 1 case study. The polished lecturer-facing report is
`KCD1_EXPERIMENT_REPORT.md`; the primary technical summary is
`CASE_STUDY_SUMMARY.md`. For supervisor review, prefer the curated evidence
bundle linked from the top-level `SUPERVISOR_README.md`.

## Storage policy for supervisor handoff

The full raw KCD1 archive is no longer tracked in Git. It is published as the
GitHub Release asset:

```text
raw-kcd1-case-study-fyp-report-final-v4.tar.zst
```

on release tag:

```text
fyp-report-final-v4
```

The normal repository checkout keeps only the source tree, polished reports,
summary files, and curated evidence bundle. Raw artifact paths mentioned in the
reports preserve the original experiment layout and should be read relative to
the extracted raw archive.

## Git-tracked first-review artifacts

| Artifact | Location | Purpose |
| --- | --- | --- |
| Supervisor entry point | `SUPERVISOR_README.md` | Five-minute review path and FYP scope boundary |
| FYP pitch | `reports/fyp-report/FYP_SUPERVISOR_PITCH.md` | Short supervisor-facing proposal |
| Scope note | `reports/fyp-report/PRE_FYP_SCOPE_NOTE.md` | Existing prototype vs proposed assessed work |
| AI disclosure | `reports/fyp-report/AI_DISCLOSURE.md` | Development-tool transparency note |
| KCD1 report | `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md` | Polished case-study narrative |
| KCD1 summary | `reports/kcd1-case-study/CASE_STUDY_SUMMARY.md` | Technical summary of method, results, and caveats |
| Curated evidence | `evidence-bundle/` | Small checksum-verified evidence bundle |
| Raw artifact note | `reports/kcd1-case-study/RAW_ARTIFACTS.md` | Release-asset location for full raw archive |

## Raw archive families

The following artifact families are preserved in the raw release archive, not in
the normal Git checkout:

| Raw family | Role | Needed for first read? |
| --- | --- | --- |
| `setup/` | Route notes, machine context, config snapshots, tree checks | no |
| `runs/` | Formal baseline run directories and derived analyses | no |
| `mangohud/` | MangoHud CSV frame-time logs | no |
| `profiles/` | CPU-affinity profile, profile-plan, and dry-run artifacts | no |
| `tune/` | Paired A/B tune run and recommendation artifacts | no |
| `results/` | Secondary fix-validation artifacts | no |
| `drop-counter-pilot/` | Measurement-quality pilot artifacts | no |
| `realworld-stack/` | Exploratory clean-vs-personal-stack comparison | no |
| `advisor-baseline-*.json` | Advisor outputs used during hypothesis formation | no |
| `fix-plan-cpu-affinity-profile.json` | Structured CPU-affinity fix plan | no |

## Curated evidence copies

The most important machine-readable artifacts are copied into
`evidence-bundle/kcd1/` so reviewers can inspect the non-validation result
without downloading the raw archive:

- `tuning_summary.json`
- `tuning_recommendation.json`
- `tuning_recommendation.md`
- `profile-plan-summary.json`
- `build-check.txt`
- `command-output.txt`

## Notes

Some files with `.json` extension in the raw archive are newline-delimited event
streams rather than single JSON documents. Some optional stream files may also
be empty when that signal was enabled but no samples were recorded. They are
preserved as produced by `stutter record`.

Large `migration_events.json` files remain intentionally excluded from Git.
