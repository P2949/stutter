# DMABUF Path Log

`--dmabuf-log <path>` ingests a cooperative NDJSON stream from a compositor,
Gamescope build, wrapper, or game integration that can report buffer
format/modifier/import facts.

This log is optional supporting evidence. It can indicate that a display path
looks copy-prone, but it does not measure exact copy latency.

Each line is one JSON object:

```json
{"elapsed_ms":1000,"source":"gamescope","surface_role":"game","output_name":"DP-1","width":2560,"height":1440,"format":"XRGB8888","modifier":"LINEAR","allocation_driver":"amdgpu","import_driver":"i915","scanout_capable":false,"zero_copy":false,"copy_required":true,"reason":"modifier_mismatch","confidence":"medium"}
```

Fields:

- `elapsed_ms`: run-relative timestamp.
- `source`: producer name, such as `gamescope`, `compositor`, or `external_log`.
- `app_id`: client or game identity when known.
- `surface_role`: `game`, `gamescope_output`, `overlay`, or another producer
  role.
- `output_name`: compositor output name when known.
- `width`, `height`: buffer dimensions.
- `format`: DRM fourcc name when available.
- `modifier`, `modifier_name`: raw or named modifier.
- `planes`: plane count.
- `allocation_driver`, `allocation_card`: exporter/allocation side.
- `import_driver`, `import_card`: importer/display side.
- `linear`: true when the buffer is linear.
- `scanout_capable`: producer's best scanout-capability hint.
- `zero_copy`: producer's best no-copy/direct hint.
- `explicit_sync`: whether explicit sync was used.
- `copy_required`: producer's best copy-required hint.
- `reason`: short source-specific reason, such as `modifier_mismatch`.
- `confidence`: `high`, `medium`, or `low`.

If `confidence` is omitted, stutter uses `medium` when direct path hints such as
`copy_required`, `scanout_capable`, or `zero_copy` are present, otherwise `low`.

Reports aggregate this stream into `dmabuf_path` and display-path diagnosis
evidence. Missing DMABUF events mean the evidence is unavailable, not that the
path avoided copies.
