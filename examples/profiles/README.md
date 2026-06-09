# stutter profile examples

These profiles are starting points for repeatable experiments, not guaranteed improvements.

Edit the CPU masks for the target machine before using the non-baseline profiles. The `baseline-online` profile uses `affinity = "online"` so it tracks the CPUs currently online on the host.

Example:

```bash
stutter tune --tree-pid <PID> --profiles examples/profiles/common-game-layouts.toml --runs 5 --baseline-profile baseline-online
```

## Inspect Before Tuning

Profile files are hypotheses. Before running a benchmark, inspect which tasks
each rule would match:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile examples/profiles/common-game-layouts.toml \
  --profile-name game-on-middle-compositor-low \
  --top 20
```

For multi-profile files, select the profile explicitly:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --profile-name tuned-profile-name
```

Without `--profile-name`, single-profile commands use the first profile in the
file for backwards compatibility.

For complex Proton/Wine games, use `--highlight-comm` for important threads:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --profile-name tuned-profile-name \
  --highlight-comm RenderThread \
  --highlight-comm dxvk-submit \
  --highlight-comm wineserver
```

This helps catch broad first-match-wins rules and `process_comm` matches before
collecting expensive A/B data.
