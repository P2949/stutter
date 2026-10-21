# Raw KCD1 Artifact Archive

The full raw KCD1 case-study archive is intentionally not tracked in Git.

The repository keeps the case-study report, summary material, and curated
evidence bundle. Raw run directories, MangoHud CSVs, profile-plan artifacts,
tuning directories, setup logs, and exploratory stack-comparison artifacts are
published separately as a GitHub Release asset.

## Release

Release tag:

```text
case-study-kcd1-2026
```

Raw archive asset:

```text
stutter-kcd1-case-study-raw.tar.zst
```

Checksum asset:

```text
stutter-kcd1-case-study-raw.tar.zst.sha256
```

Contents listing asset:

```text
stutter-kcd1-case-study-raw.contents.txt
```

Contents-listing checksum asset:

```text
stutter-kcd1-case-study-raw.contents.txt.sha256
```

## Why the raw archive is not tracked in Git

The raw case-study archive is useful for audit and reproduction, but large run
directories, trace data, and frame logs would make normal source clones
unnecessarily large.

Start with:

- `reports/kcd1-case-study/KCD1_EXPERIMENT_REPORT.md`
- `reports/kcd1-case-study/CASE_STUDY_SUMMARY.md`
- `reports/kcd1-case-study/ARTIFACT_INDEX.md`
- `evidence-bundle/`

Download the raw archive when full experiment artifacts are needed for deeper
audit, re-analysis, or reproduction.

Verify the archive before use:

```bash
sha256sum -c stutter-kcd1-case-study-raw.tar.zst.sha256
zstd -t stutter-kcd1-case-study-raw.tar.zst
```

The contents listing can be verified independently with:

```bash
sha256sum -c stutter-kcd1-case-study-raw.contents.txt.sha256
```
