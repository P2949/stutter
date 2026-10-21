# KCD1 case-study artifact index

This directory contains the Git-tracked entry points for the Kingdom Come:
Deliverance 1 case study. Start with `KCD1_EXPERIMENT_REPORT.md` for the full
narrative or `CASE_STUDY_SUMMARY.md` for the compact technical summary.

## Storage policy

The full raw KCD1 archive is intentionally kept out of normal Git history. It is
published as a GitHub Release asset under:

```text
case-study-kcd1-2026
```

Raw archive:

```text
stutter-kcd1-case-study-raw.tar.zst
```

Checksum:

```text
stutter-kcd1-case-study-raw.tar.zst.sha256
```

Contents listing:

```text
stutter-kcd1-case-study-raw.contents.txt
```

Contents-listing checksum:

```text
stutter-kcd1-case-study-raw.contents.txt.sha256
```

The normal repository checkout keeps the source tree, case-study report, summary
material, and curated evidence bundle. Raw artifact paths mentioned in the
reports preserve the original experiment layout and should be read relative to
the extracted raw archive.

## Git-tracked entry points

| Artifact | Location | Purpose |
| --- | --- | --- |
| KCD1 report | `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md` | Full case-study narrative, setup, evidence, limitations, and conclusion |
| KCD1 report PDF | `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.pdf` | Rendered copy of the Markdown report |
| KCD1 summary | `reports/kcd1-case-study/CASE_STUDY_SUMMARY.md` | Technical summary of method, results, and caveats |
| Hypothesis note | `reports/kcd1-case-study/HYPOTHESIS.md` | Follow-up hypothesis and experiment design notes |
| Curated evidence | `evidence-bundle/` | Small checksum-verified machine-readable evidence bundle |
| Raw artifact note | `reports/kcd1-case-study/RAW_ARTIFACTS.md` | Release location and archive policy for the full raw data |

## Raw archive families

The following artifact families are preserved in the raw release archive rather
than the normal Git checkout:

| Raw family | Role |
| --- | --- |
| `setup/` | Route notes, machine context, config snapshots, tree checks |
| `runs/` | Formal baseline run directories and derived analyses |
| `mangohud/` | MangoHud CSV frame-time logs |
| `profiles/` | CPU-affinity profile, profile-plan, and dry-run artifacts |
| `tune/` | Paired A/B tune run and recommendation artifacts |
| `results/` | Secondary fix-validation artifacts |
| `drop-counter-pilot/` | Measurement-quality pilot artifacts |
| `realworld-stack/` | Exploratory clean-vs-personal-stack comparison |
| `advisor-baseline-*.json` | Advisor outputs used during hypothesis formation |
| `fix-plan-cpu-affinity-profile.json` | Structured CPU-affinity fix plan |

## Curated evidence copies

The most important machine-readable artifacts are copied into
`evidence-bundle/kcd1/` so the primary non-validation result can be inspected
without downloading the raw archive:

- `tuning_summary.json`
- `tuning_recommendation.json`
- `tuning_recommendation.md`
- `profile-plan-summary.json`
- `build-check.txt`
- `command-output.txt`

## Notes

Some files with a `.json` extension in the raw archive are newline-delimited
event streams rather than single JSON documents. Some optional stream files may
also be empty when that signal was enabled but no samples were recorded. They
are preserved as produced by `stutter record`.

Large `migration_events.json` files remain intentionally excluded from Git.
