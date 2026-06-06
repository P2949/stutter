# Stutter KCD1 Evidence Bundle

This bundle curates the small set of artifacts needed to inspect the KCD1 CPU-affinity non-validation result without loading the full raw trace archive.

## Layout

```text
evidence-bundle/
  README.md
  MANIFEST.sha256
  kcd1/
    tuning_summary.json
    tuning_recommendation.json
    tuning_recommendation.md
    profile-plan-summary.json
    build-check.txt
    command-output.txt
```

## Notes

- `tuning_summary.json` is the primary profile-vs-profile A/B evidence.
- `tuning_recommendation.json` and `.md` are regenerated from the fixed recommendation path, which avoids baseline-against-itself comparison.
- `profile-plan-summary.json` records the profile explainability pass.
- `build-check.txt` records the local validation commands run for this artifact refresh.
- `command-output.txt` is the secondary fix-validation command output pointer from the archived KCD1 report set.
- Large raw run directories, migration streams, and machine-specific trace logs are intentionally excluded from this curated bundle.
