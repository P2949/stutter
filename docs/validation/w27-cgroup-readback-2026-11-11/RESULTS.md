# W27: real cgroup readback on an owned, delegated scope (2026-11-11)

Read-only probes plus two transient user scopes created with `systemd-run --user --scope` (they vanish with their process; no
bystander cgroup, no root cgroup and no Stutter action was touched).

* The user manager (`user@1000.service`) has `cpu io memory pids` available and `cpu memory pids` delegated to its subtree;
  **`io` is not delegated**, so io.weight is unavailable to this account: the missing-subtree-controller denial is real on
  this host.
* A scope started with `-p CPUWeight=50 -p MemoryHigh=2G` reads back `cpu.weight = 50` and `memory.high = 2147483648`
  from the kernel's cgroup files: systemd-property placement with real readback of the effective value.
* A scope started with no properties exposes only `memory pids`: `cpu.weight`, `cpu.idle`, `cpu.max`, `io.weight` and
  `cpuset.cpus` do not exist there (the controller is not enabled for the child until a property requests it), and a direct
  write to `cpu.weight`/`cpu.idle` from the unprivileged process fails with `Permission denied`.

These are the facts the existing typed preflight (`cgroup_control`: parent delegation, effective value, owner) has to model. They
show a real isolated owned scope supports `cpu.weight` and `memory.high` through systemd properties, `io.weight` and `cpuset` are
denied here, and a process cannot widen its own cgroup. No new autonomy and no new Supported tuple: promotion needs the W24
benefit result for that exact tuple and separate graduation, and the earlier negative `cpu.weight`/`cpu.idle` pilots
(`../cpu-weight-pilot`) are preserved, not repeated.
