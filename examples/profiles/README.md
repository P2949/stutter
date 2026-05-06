# stutter profile examples

These profiles are starting points for repeatable experiments, not guaranteed improvements.

Edit the CPU masks for the target machine before using the non-baseline profiles. The `baseline-online` profile uses `affinity = "online"` so it tracks the CPUs currently online on the host.

Example:

```bash
stutter tune --tree-pid <PID> --profiles examples/profiles/common-game-layouts.toml --runs 5 --baseline-profile baseline-online
```
