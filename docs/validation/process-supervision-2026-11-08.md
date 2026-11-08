# Supervision deadline and identity regression checks

This is a partial W1 improvement, not complete process containment or product-service qualification.

The supervisor now uses nonblocking captured pipes with a bounded post-cleanup drain. A detached
child that clears `STUTTER_XTASK_OWNER` and retains a pipe cannot hang the reader join indefinitely:
the stage fails explicitly when EOF cannot be established. Tee output has the same immutable byte
cap as capture. Child polling errors still run cleanup. The direct-child parent-death hook checks
both `prctl` and the fork-to-hook parent-death race.

Orphan reconciliation opens a Linux pidfd before rechecking start ticks and signals that kernel
identity. It does not fall back to numeric-pid signaling when pidfds are unavailable. Owner marker
parsing rejects system pids, zero start ticks and trailing fields. CI shares a six-hour default
deadline across its stages (`STUTTER_XTASK_CI_TIMEOUT_SECS`, positive seconds); each child receives
the shorter of its stage budget and remaining campaign time. Cargo metadata capture is supervised.

Focused validation in the pinned Nix shell:

```sh
nix develop --command cargo test -p xtask -- process:: local_ci::tests::a_whole_campaign_deadline
```

Result: 14 passed, zero failed. Tests include a markerless detached pipe holder, cumulative campaign
timeout, changed start ticks, TERM-ignore, ordinary setsid escape, output flood, cancellation and an
unrelated sentinel. The deliberately uncontained test helper is explicitly killed by the fixture.

Still required: delegated cgroup/scope containment for marker-clearing descendants, crash-safe
containment/reconciliation independent of environment markers, bounded privileged-runner logs and
runner-wide deadlines. The markerless test proves bounded failure reporting, not orphan freedom.
No physical mutation, service deployment or authority change is part of this slice.
