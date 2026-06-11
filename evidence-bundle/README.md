# Stutter KCD1 Evidence Bundle

This bundle curates the small set of artifacts needed to inspect the KCD1
CPU-affinity non-validation result without loading the full raw trace archive.

Machine-local home paths and usernames in presentation artifacts are sanitized
with placeholders such as `<home>`, `<user>`, and `<repo>`.

The bundle supports a narrow claim: the archived KCD1 CPU-affinity profile was
not validated by the preliminary evidence, and the workflow could decline an
unsupported recommendation. It does not claim a general KCD1 performance result,
does not generalize to other machines or games, and preserves the caveat that the
archived paired A/B run was not counterbalanced.

Start with `kcd1/README_FIRST.md` if opening the KCD1 artifacts directly.
See `EVIDENCE_LIMITATIONS.md` for the short claim-boundary note.

## Layout

```text
evidence-bundle/
  README.md
  EVIDENCE_LIMITATIONS.md
  MANIFEST.sha256
  kcd1/
    README_FIRST.md
    tuning_summary.json
    tuning_recommendation.json
    tuning_recommendation.md
    profile-plan-summary.json
    build-check.txt
    command-output.txt
```

## Notes

- `kcd1/README_FIRST.md` explains the artifact naming caveat before opening raw JSON.
- `tuning_summary.json` is the primary profile-vs-profile A/B evidence.
- `tuning_recommendation.json` and `.md` are regenerated from the fixed
  recommendation path, which avoids baseline-against-itself comparison.
- In recommendation artifacts, the selected profile may be the baseline. Some
  legacy JSON field names use baseline/tuned terminology, but the report should
  be read as selected profile versus comparison profile.
- `profile-plan-summary.json` records the profile explainability pass.
- `build-check.txt` records the local validation commands run for this artifact refresh.
- `command-output.txt` records the secondary fix-validation command context and
  points to the archived report outputs.
- Large raw run directories, migration streams, and machine-specific trace logs
  are intentionally excluded from this curated bundle.

## Export automation:

The script `scripts/artifacts/export_evidence_bundle.sh` creates a sanitized
export of `evidence-bundle/` in `/tmp` and runs a basic privacy scan. Reviewers
can use this to prepare release artifacts.
