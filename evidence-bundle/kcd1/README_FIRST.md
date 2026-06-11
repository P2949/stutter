# KCD1 Evidence Artifacts - Read First

This directory contains the small, sanitized artifact set for the preliminary
KCD1 CPU-affinity non-validation result. It is intentionally much smaller than
the full raw run archive.

The supported claim is narrow: for the archived route, system, Proton stack, and
fixed-order paired A/B run, the tested CPU-affinity profile was not validated and
should not be recommended on the current evidence. The run was paired but not
counterbalanced, so it remains preliminary low-confidence evidence rather than a
final effect estimate.

In recommendation artifacts, the selected profile may be the baseline. Some
legacy JSON field names use baseline/tuned terminology, but the report should be
read as selected profile versus comparison profile. The human-readable
case-study report is authoritative for interpretation.

Recommended reading order:

1. `../README.md`
2. `../EVIDENCE_LIMITATIONS.md`
3. `tuning_recommendation.md`
4. `tuning_summary.json`
5. `profile-plan-summary.json`

Large raw run directories, migration streams, and machine-specific trace logs are
intentionally excluded from this curated bundle.
