# Direct Scanout Evidence

Direct-scanout status is currently derived from cooperative presentation and
DMABUF hints rather than a universal compositor API.

The report value is one of:

- `yes`: available evidence consistently points to direct scanout or zero-copy.
- `no`: available evidence consistently points to compositing, copy-required, or
  no scanout-capable buffer.
- `mixed`: events disagree or the run changed state.
- `unknown`: the needed evidence was missing.

Accepted cooperative hints include:

- Wayland presentation `zero_copy=true`
- Wayland presentation `zero_copy=false`
- DMABUF `scanout_capable=true`
- DMABUF `scanout_capable=false`
- DMABUF `copy_required=true`
- source-specific `reason` values such as `overlay`, `modifier_mismatch`, or
  `not_fullscreen`

These hints are evidence about the compositor/buffer path, not photon latency.
Missing hints are unavailable evidence. They must not be rendered as direct
scanout success.

The recommended workflow is:

```bash
stutter record --preset prime-display-path \
  --wayland-presentation \
  --wayland-presentation-log /path/to/presentation.ndjson \
  --dmabuf-log /path/to/dmabuf.ndjson
```

Then compare controlled runs:

```bash
stutter compare display-path \
  --baseline /path/to/direct-run \
  --test /path/to/uhd630-run \
  --expect direct-to-offload \
  --strict
```
