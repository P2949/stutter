# Config Crate Migration Checklist

This document tracks ownership while configuration logic moves from the main
`stutter` crate into the shared `stutter-config` crate.

## Current Ownership

| Area | Current owner | Migration target | Notes |
| --- | --- | --- | --- |
| Shared path container | `stutter-config/src/model.rs` | `stutter-config` | Currently only stores `StutterPaths`; safe shared model already exists. |
| Layer model and resolver | `stutter-config/src/layer.rs`, `stutter-config/src/resolve.rs` | `stutter-config` | Keep as crate-owned shared merge primitives. |
| Monitor config aggregate | `stutter/src/config/model.rs` | Split | Pure data structs should move first; runtime defaults that reference probes, foreground providers, or OS behavior stay in main crate until isolated. |
| Config enums and constants | `stutter-config/src/types.rs` | `stutter-config` | `FocusSource`, `ForegroundSource`, `WaylandPresentationSource`, `CsvStreamTarget`, and `TARGET_PIDS_MAX` are owned by `stutter-config`; CLI derives are behind the optional `cli` feature and re-exported by `stutter/src/config/types.rs`. |
| Static validation | `stutter-config/src/validation.rs` | `stutter-config` | Pure range checks for nonzero intervals, `target.max_tasks`, alert/MangoHud bounds, and empty output strings live in `validate_static_config`. |
| Runtime/environment validation | `stutter/src/config/validation.rs`, eBPF loader/preflight | Main crate | eBPF map sizing, target map relationships, kernel limits, tracepoint availability, and filesystem/runtime checks stay in main crate. |
| Merge implementation | `stutter/src/config/merge.rs` | Main crate first, then split | File is large; split locally before moving shared pieces across crates. |
| Effective config provenance | `stutter/src/config/effective.rs`, `stutter/src/config/effective/*` | Main crate for now | Provenance depends on CLI/config-file behavior and should move only after type ownership is settled. |
| Config-file parsing bridge | `stutter/src/config_file/*` | Main crate bridge | Owns TOML/CLI mapping into `MonitorConfig`; may shrink once pure types live in `stutter-config`. |

## Field Groups Still Main-Crate-Only

- Target selection: `TargetConfig`, cgroup path handling, process filters, max task cap.
- Probe toggles and probe-specific options: eBPF, KMS, DRM fence, Wayland presentation, dmabuf, GPU engine, hwmon, MangoHud.
- Output and stream options: JSON stream, metrics, OTLP, CSV target.
- Safety and remote-control options: follow-exec, daemon/agent remote settings.
- eBPF sizing: ring buffer, map entry counts, and map factor knobs.
- UI and reporting options that are currently CLI-shaped.

## Migration Order

1. Move pure enums/constants behind a `stutter-config` feature strategy that does not force `clap` on non-CLI consumers.
2. Move pure config structs with plain defaults and re-export them from `stutter/src/config`.
3. Extract static validation into `validate_static_config(...)` in `stutter-config`.
4. Keep runtime validation in main crate as `validate_runtime_config(...)`.
5. Split `stutter/src/config/merge.rs` before moving reusable merge primitives.

## Acceptance Checklist

- [x] Main crate re-exports moved config types without CLI behavior changes.
- [x] `target.max_tasks` and simple nonzero/range validation live in `stutter-config`.
- [x] Runtime/kernel validation remains in main crate with explicit names.
- [ ] Config merge provenance stays testable after the split.
