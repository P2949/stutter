# Report View Roadmap

The rich interactive HTML report remains owned by the main `stutter` crate for
now. `stutter-report` provides a basic self-contained HTML renderer for migrated
`ReportModel` data; future work may either move the rich HTML pipeline into
`stutter-report` or keep it as main-crate CLI integration.

Near-term work must improve views from existing artifacts before adding new probes.

In scope:
- PSI pressure timeline from interval.json
- compositor/frame-pacing view from MangoHud frame events + task classes
- clearer diagnosis evidence/explanation
- clearer missing evidence sections
- HTML improvements for spike clusters, foreground context, and frame outliers

Out of scope:
- new eBPF tracepoints
- new kernel streams
- new required artifacts
- compositor-specific live probes
- DRM fence telemetry
