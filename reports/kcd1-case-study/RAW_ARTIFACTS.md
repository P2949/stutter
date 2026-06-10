# Raw KCD1 Artifact Archive

The full raw KCD1 case-study archive is intentionally not tracked in Git.

The repository keeps only the polished case-study report, summary material, and
the small curated evidence bundle. The raw run directories, MangoHud CSVs,
profile-plan artifacts, tuning directories, setup logs, and exploratory stack
comparison artifacts are published as a GitHub Release asset instead.

## Release asset

Release tag:

```text
fyp-report-final-v4
````

Raw archive asset:

```text
raw-kcd1-case-study-fyp-report-final-v4.tar.zst
```

Checksum asset:

```text
raw-kcd1-case-study-fyp-report-final-v4.tar.zst.sha256
```

Contents listing asset:

```text
raw-kcd1-case-study-fyp-report-final-v4.contents.txt
```

## Why this is not tracked in Git

The raw case-study archive is useful for audit and reproduction, but it makes the
normal repository checkout unnecessarily large for supervisor review. First
review should use:

* `SUPERVISOR_README.md`
* `reports/fyp-report/FYP_SUPERVISOR_PITCH.md`
* `reports/fyp-report/PRE_FYP_SCOPE_NOTE.md`
* `reports/fyp-report/AI_DISCLOSURE.md`
* `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md`
* `evidence-bundle/`

The raw archive should only be downloaded when audit or reproduction artifacts
are needed.
