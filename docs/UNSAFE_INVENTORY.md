# Unsafe Inventory

This inventory tracks non-eBPF `unsafe` usage in the main `stutter` crate. Each
row covers the unsafe blocks/functions currently in that file and names the
owner plus the migration target needed before the unsafe documentation gate can
become a hard architecture test.

## Status

This inventory is a safety/audit document for the current prototype. The listed
migration targets are hardening tasks and future refactor directions, not
required preconditions for the supervisor-review snapshot. The current gate
requires unsafe usage to be documented with `// SAFETY:` comments and tracked
here before new unsafe usage is accepted.

Generated baseline command:

```bash
rg -n "unsafe\s*(\{|fn|impl)" stutter/src --glob '*.rs'
```

## Production Code

| File | Count | Owner | Current purpose | Migration target |
| --- | ---: | --- | --- | --- |
| `stutter/src/actions/ioprio.rs` | 2 | actions | `ioprio_get`/`ioprio_set` syscalls. | Move to `actions/syscalls.rs` safe wrappers with one `// SAFETY:` contract per wrapper. |
| `stutter/src/actions/nice.rs` | 1 | actions | `setpriority` syscall wrapper inline in action code. | Move to `actions/syscalls.rs`. |
| `stutter/src/actions/uclamp.rs` | 2 | actions | `sched_getattr`/`sched_setattr` syscalls. | Move to `actions/syscalls.rs`. |
| `stutter/src/affinity.rs` | 7 | affinity/actions | `cpu_set_t` initialization/manipulation and `sched_getaffinity`/`sched_setaffinity`. | Split affinity module and isolate unsafe in `affinity/syscall.rs`. |
| `stutter/src/autotune/active_config/collector.rs` | 1 | autotune/actions | Reads current I/O priority with raw syscall. | Reuse `actions/syscalls.rs` I/O priority getter. |
| `stutter/src/autotune/emergency_restore/executors.rs` | 3 | autotune/actions | Emergency restore syscalls for nice/uclamp/ioprio. | Reuse shared syscall wrappers so restore paths share safety docs. |
| `stutter/src/community_rules/paths.rs` | 4 | community rules | Temporary environment mutation in helpers compiled for command/test workflows. | Keep gated as test/support utility or wrap env mutation in a documented helper. |
| `stutter/src/daemon/health.rs` | 2 | daemon | `statvfs` disk-space probe. | Move to small filesystem probe wrapper with `// SAFETY:` and fixture tests. |
| `stutter/src/daemon/overhead.rs` | 1 | daemon | `sysconf(_SC_CLK_TCK)` for runtime accounting. | Reuse a shared checked `clock_ticks_per_second()` helper. |
| `stutter/src/doctor.rs` | 3 | doctor/eBPF preflight | `geteuid`, `getrlimit`, and `MaybeUninit::assume_init`. | Move OS probes behind documented checked wrappers. |
| `stutter/src/ebpf/memlock.rs` | 2 | eBPF userspace loader | `getrlimit`/`setrlimit` for memlock. | Keep in loader wrapper with local `// SAFETY:` docs. |
| `stutter/src/ebpf/memory.rs` | 3 | eBPF userspace loader | `sysconf` memory/page-size probes. | Keep in loader wrapper with checked return handling and `// SAFETY:` docs. |
| `stutter/src/events.rs` | 1 | event decode | Byte casting for ABI event decode. | Centralize in `events/decode.rs` with size/alignment checks. |
| `stutter/src/events/decode.rs` | 5 | event decode | Unaligned byte reads for ABI event records. | Keep here as the audited decode boundary with per-block `// SAFETY:` comments. |
| `stutter/src/perf_counters.rs` | 6 | hardware probes | `gettid`, `read`, `ioctl`, `perf_event_open`, and `getrlimit`. | Split into `perf_counters/syscall.rs` and document each wrapper. |
| `stutter/src/recorder/retention.rs` | 2 | recorder | `statvfs` retention/free-space probe. | Share filesystem probe wrapper with daemon health. |
| `stutter/src/recorder/session.rs` | 4 | recorder | `clock_gettime`, `uname`, and C string conversion. | Existing `// SAFETY:` comments are present; move to recorder/session time/platform helpers during split. |
| `stutter/src/runtime_slices.rs` | 1 | runtime slices | `sysconf(_SC_CLK_TCK)`. | Reuse checked `clock_ticks_per_second()` helper. |
| `stutter/src/session/monitor_session/run_loop.rs` | 1 | session runtime | Unaligned read of event kind from ring-buffer bytes. | Route through `events/decode.rs` helper. |
| `stutter/src/wayland_probe.rs` | 3 | Wayland probe | `memfd_create`, `File::from_raw_fd`, and FFI block. | Split into `wayland_probe/ffi.rs` and `wayland_probe/memfd.rs` with documented wrappers. |

## Test/Fixture Code

| File | Count | Owner | Current purpose | Migration target |
| --- | ---: | --- | --- | --- |
| `stutter/src/cli/monitor/tests/bench.rs` | 3 | CLI tests | Environment mutation during parser tests. | Reuse a test env guard helper. |
| `stutter/src/community_rules/tests/commands.rs` | 4 | community-rules tests | Environment mutation around command tests. | Reuse a test env guard helper. |
| `stutter/src/config_file/tests/parse.rs` | 3 | config tests | Environment mutation around parser tests. | Reuse a test env guard helper. |
| `stutter/src/foreground/tests/command.rs` | 4 | foreground tests | Environment mutation/sanitization tests. | Reuse a test env guard helper. |
| `stutter/src/foreground/tests/hyprland.rs` | 12 | foreground tests | Environment mutation for provider fixtures. | Reuse a test env guard helper. |
| `stutter/src/foreground/tests/mod.rs` | 3 | foreground tests | Shared unsafe env restoration helper. | Document this helper or replace with safe test isolation API if Rust exposes one. |
| `stutter/src/foreground/tests/sway_parse.rs` | 5 | foreground tests | Environment mutation for Sway fixtures. | Reuse a test env guard helper. |
| `stutter/src/foreground/tests/x11_parse.rs` | 4 | foreground tests | Environment mutation for X11 fixtures. | Reuse a test env guard helper. |

## Hardening Order

1. Create action syscall wrappers and move action/autotune restore syscalls there.
2. Centralize `sysconf(_SC_CLK_TCK)` in one checked helper.
3. Centralize `statvfs` probes for recorder and daemon.
4. Keep event byte casting in `events/decode.rs` and add local `// SAFETY:`
   comments.
5. Split Wayland and perf-counter FFI/syscall modules.
6. Add an architecture test requiring `// SAFETY:` immediately before remaining
   non-eBPF unsafe blocks.
