# Cleanup Sequence Rules

> Internal maintenance note. This document records behavior-preserving refactor
> rules and maturity-report maintenance expectations.

One behavior-preserving refactor per commit.
No functional changes mixed with file moves.
Every moved module keeps old tests passing before new work starts.
Every removed allowlist entry gets its own commit.
No new `#[allow(...)]`.
No new oversized file allowlist.
