# Issue cleanup summary

This document lists the issues closed or addressed as part of the supervisor handoff and provides next steps for any remaining tracker housekeeping.

Closed/resolved during this handoff:

- Issue #13 — Explicit counterbalanced tune order: Implemented `order_strategy` (alternating/fixed/seeded), persisted in `TuneSummary`, and added comparability/order-balance detection and tests.
- Issue #14 — Order-balance warnings: Implemented explicit detection rules, structured `order_balanced` + `order_balance_warning` fields, and integrated warnings into recommendation outputs and tests.

Next steps for maintainers (manual GitHub actions):

1. Verify the branch `release-final` / `supervisor-handoff-final` contains the expected artifacts and PR description.
2. Close related issues on GitHub ( #13 and #14 ) and link to the release PR or tag.
3. If desired, create a GitHub Release using the assembled archive at `release/stutter-supervisor-release-<short-sha>.tar.gz` and attach the PDFs/evidence bundle as separate assets.

CLI hints to close issues via GitHub CLI (if available):

```bash
gh issue close 13 --repo P2949/stutter --comment "Resolved in supervisor handoff: added explicit order_strategy, persisted artifacts, and tests. See PR #<PR_NUMBER>."
gh issue close 14 --repo P2949/stutter --comment "Resolved in supervisor handoff: added order-balance warnings and tests. See PR #<PR_NUMBER>."
```

If you want me to attempt closing the issues and creating the release PR automatically, grant access or run with an authenticated `gh` client and I can attempt it from here.
