# Tuning Workflow

The trusted loop is:

```text
diagnosis candidate -> structured fix hypothesis -> validation experiment -> A/B evidence -> fix verdict
```

Do not treat an advisor recommendation as proof. A fix is validated only when
the fix plan acceptance criteria pass against repeated, comparable baseline and
tune runs.

## Canonical Flow

```bash
stutter record --tree-pid <PID> --duration 180 --run-name baseline-a
stutter advisor --run baseline-a --json > advisor.json
stutter profile-plan --tree-pid <PID> --profile profiles.toml
stutter tune --tree-pid <PID> --profiles profiles.toml --runs 5 --baseline-profile baseline-online
stutter recommend --fix-plan advisor.json \
  --baseline baseline-a \
  --baseline baseline-b \
  --baseline baseline-c \
  --baseline baseline-d \
  --baseline baseline-e \
  --tune tune-dir \
  --html fix-validation.html
```

`advisor --json` includes fix plans inline. `recommend --fix-plan` accepts either
that advisor JSON or a standalone advisor fix-plan JSON.

## Preview Profile Matching Before Tuning

Before running a tuning experiment, inspect how each profile rule would match the
live process tree:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --top 20 \
  --highlight-comm RenderThread \
  --highlight-comm dxvk-submit
```

For apply-profile dry runs, the same explanation can be emitted with:

```bash
stutter apply-profile \
  --tree-pid <PID> \
  --profile profiles.toml \
  --dry-run \
  --explain
```

Use JSON output for artifacts:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --json \
  --output profile-plan.json
```

Profile rules are first-match-wins. `match_comm` checks both a thread's `comm`
and its process `process_comm`, so a broad rule such as `match_comm = ["Main"]`
may match worker threads whose own thread names are different.

## Verdicts

- `validated`: all required metrics exist, enough samples are present, required
  CIs exclude zero in the right direction, and guardrails pass.
- `rejected`: the primary metric regressed, the CI excludes zero in the wrong
  direction, or a guardrail exceeded its allowed regression.
- `underpowered`: samples or required confidence intervals are missing.
- `inconclusive`: samples are present but required CIs cross zero.
- `invalid_experiment`: comparability or drop-counter failures mean the A/B run
  cannot prove the fix.

Underpowered means do not apply as proof. Invalid experiment means repeat the
experiment with a comparable workload before deciding.
