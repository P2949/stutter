# Controlled fixed-work KPI scenarios (2026-11-09)

A bounded, unprivileged study using the real KPI collector, the real independent-block experiment
manager and a supervised producer that really runs an integer fold (100000 steps, independently
checked checksum). Four counterbalanced pairs per scenario, fixed-N analysis at 95% confidence,
practical effect 20%, equivalence margin 5%, hard guardrail `completed_units` (no regression
allowed). Harness-supplied monitor ticks; no actuator; no installed-service or scheduler
qualification; **no tuning benefit is claimed**. `authorizes_mutation=false` and
`outstanding_owned_tokens=0` in every run. Raw blocks stay private in
`.llm-runs/kpi-scenarios-20261109/` and are verified by `verify_captures` (every interval is
reconstructed from the raw captures).

The point of the study is that the framework answers correctly when the truth is *not* a clean
positive: a real regression, a real improvement, a null difference, and an arm that looks faster
only because it did less work.

| scenario | what the treatment arm really does | improvement interval (95%, %) | decision |
|---|---|---|---|
| `identical` | the same work | [-12.0, +7.7], mean -2.2 | `Uncertain` (neither win nor loss; strict equivalence not proven, so not `Equivalent`) |
| `slower` | the work three times over | [-232.1, -179.5], mean -205.8 | `Harmful` |
| `faster` | baseline does it three times, treatment once | [+64.2, +72.0], mean +68.1 | `Beneficial` |
| `short-work` | half the work (faster on the primary metric) | [+41.2, +60.2], mean +50.7 | `InvalidExperiment`: the hard completed-units guardrail failed, so the apparent win is refused |

Each run checks its own expected classification and fails the study if the framework answers
otherwise (`ensure_expected`). The short-work scenario's first attempt (kept as
`short-work-first-attempt-wrong-expectation`) expected a pair-level refusal; the framework instead
analysed it and returned `InvalidExperiment`, the safe answer, so the study's expectation, not the
framework, was corrected.

SHA-256 of the retained results:

```
identical  summary 2c1b1013f8f1d2901fe3d4b0d8d0a16ce0158878a41057a17dd1100bcfa6bdb5
identical  evidence 8d430e2ad2bc3b3fc267f1188930474f9b25ab8005154f434219cb2e035b30d7
slower     summary 4aeb275a5788cdacd78f5816fb3d0d12f997f05bb324081ce9e60d1a5c46de3a
slower     evidence ffe6cd87834bccb2db561467849dc9a65040a2f1a6b8bcc2190a2e969a3a6d99
faster     summary 4976bbc8c2e70f28db3a9c06cb1f87bf6a0e8d8d2302d693a048f9a1f6a5c83a
faster     evidence b597d2bfde8cf135d517db96cc984621553c14a40b06a8c98c34959c172a029f
short-work summary 9a6ae91a21a22c36413a88d4fc527dede15365e766a15dd9d1b1e1110f12d1be
short-work evidence 31d8b529ff0a92e69101754ed80625e35917b60295fcaaeb2313e9ecf26585fd
```

Not shown by this study: a real game or service workload, the installed service, any actuator, or
the 2 ms frame gate. Those remain separate and deferred.
