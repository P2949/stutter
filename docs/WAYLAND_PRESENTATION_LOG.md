# Wayland Presentation Log

`stutter --wayland-presentation --wayland-presentation-log <path>` ingests a
cooperative NDJSON stream. This is required because stutter cannot observe
arbitrary Wayland client presentation feedback unless the client, compositor,
Gamescope, or a wrapper emits it.

`stutter wayland-probe --duration 30 --output DP-1 --fullscreen --out-dir ./run`
is a feature-gated self-test client (`--features wayland-probe`). It creates a
small Wayland surface, requests `presentation-time` feedback for its own commits,
and writes `wayland_presentation_events.json` in the output directory. Those
events are useful as a compositor/output baseline, not as direct evidence for a
game surface.

Each line is one JSON object:

```json
{"commit_ns":123456789000,"presented_ns":123456797400,"output_name":"DP-1","refresh_ns":6944444,"sequence":99182,"zero_copy":true,"discarded":false,"source":"gamescope"}
```

Fields:

- `commit_ns`: monotonic timestamp for the associated `wl_surface.commit`.
- `presented_ns`: monotonic timestamp reported by presentation feedback.
- `commit_to_present_ns`: optional precomputed duration. When omitted, stutter
  derives it from `presented_ns - commit_ns`.
- `output_name`: compositor output name, such as `DP-1`.
- `refresh_ns`: output refresh period from the feedback source when known.
- `sequence`: presentation sequence number when known.
- `zero_copy`: direct-scanout/zero-copy hint when the source can expose it.
- `discarded`: true when feedback reported the commit was discarded.
- `source`: `gamescope`, `external_log`, or `self_test`.
- `app_id`: client or game identity when known.
- `surface_role`: `game`, `gamescope_output`, `overlay`, or `self_test` when
  known.
- `flags`: optional source-specific flags.
- `confidence`: optional `high`, `medium`, or `low` override.

Missing presentation events are unavailable evidence, not proof that presentation
timing was healthy. `source=self_test` measures stutter's own probe surface, not
the game surface.

Reports aggregate events by `source` and `surface_role`. Gamescope events with
`surface_role=game` or `surface_role=gamescope_output` are treated as cooperative
compositor evidence and can become a `compositor/presentation queue delay`
candidate when commit-to-present delay lands near frame outliers. This remains a
candidate attribution until compared with KMS, DRM fence, and scheduler evidence.
