# Stutter KCD1 Evidence Bundle

This bundle curates the small set of artifacts needed to inspect the KCD1 CPU-affinity non-validation result without loading the full raw trace archive.

Machine-local home paths and usernames in presentation artifacts are sanitized
with placeholders such as `<home>`, `<user>`, and `<repo>`.

The bundle supports a narrow claim: the archived KCD1 CPU-affinity profile was
not validated by the preliminary evidence, and the workflow could decline an
unsupported recommendation. It does not claim a general KCD1 performance result,
does not generalize to other machines or games, and preserves the caveat that the
archived paired A/B run was not counterbalanced.

See `EVIDENCE_LIMITATIONS.md` for the short claim-boundary note.

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
