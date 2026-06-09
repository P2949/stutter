# PRIME Display Path Diagnosis

`prime-display-path` is a recording preset for controlled display-path
experiments, especially dGPU render with iGPU/UHD630 scanout.

It enables low-to-medium overhead evidence that helps answer:

- did the render GPU and scanout GPU differ?
- did display-side fence waits line up with frame outliers?
- did KMS/pageflip timing worsen?
- did the compositor/presentation path add delay?
- did iGPU render/blitter activity appear near outliers?
- did cooperative DMABUF logs report modifier mismatch or copy-required hints?

The preset is not a photon-latency tool. It produces candidate attribution and
A/B estimates.

## Recording

Example:

```bash
stutter record --preset prime-display-path --mangohud-log /path/to/mango.csv
```

The preset enables:

- `display_topology.json`
- KMS timing
- DRM fence latency
- hwmon
- GPU engine sampling
- foreground window context
- runtime slices

Wayland presentation and DMABUF evidence need cooperative logs:

```bash
stutter record --preset prime-display-path \
  --wayland-presentation \
  --wayland-presentation-log /path/to/presentation.ndjson \
  --dmabuf-log /path/to/dmabuf.ndjson
```

The preset does not force Wayland presentation without a source because an empty
presentation stream would look like evidence when it is only missing data.

## Comparing Runs

Use the guided A/B comparison after recording a direct-display baseline and a
cross-GPU scanout test:

```bash
stutter compare display-path \
  --baseline /path/to/direct-run \
  --test /path/to/uhd630-run \
  --expect direct-to-offload \
  --strict
```

`--strict` downgrades confidence when comparability checks find serious issues,
such as a different render GPU, mismatched duration/frame counts, missing probe
availability in only one run, or different connector mode/EDID evidence.

Useful expectation values:

- `direct-to-offload`: baseline is direct render+scanout and test is PRIME/offload.
- `offload-to-direct`: baseline is PRIME/offload and test is direct render+scanout.
- `unknown`: no expected direction.

## Interpreting Output

The display-path diagnosis has two separate values:

- `suspicion_score`: how display-path-like the symptoms are.
- `confidence`: how much usable evidence was available.

Missing evidence means unknown. It does not prove the display path is healthy.
Copy-required, fence, KMS, and compositor signals are candidates until they are
checked against frame pacing, scheduler latency, and run comparability.
