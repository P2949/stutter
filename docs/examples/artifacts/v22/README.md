# stutter v22 public artifact examples

This directory intentionally contains only small, representative sanitized examples.

## Examples

| Directory                      | Purpose                                        |
| ------------------------------ | ---------------------------------------------- |
| `clean_baseline/`              | Quiet baseline run with no strong diagnosis.   |
| `game_thread_scheduler_delay/` | Game-thread scheduler-delay diagnosis example. |
| `low_quality_truncated/`       | Low-quality/truncated data-quality example.    |
| `display_timing_optional/`     | Minimal optional KMS/fence/Wayland stream examples. |

The larger regression corpus lives under:

```text
stutter/tests/fixtures/runs/
```

Do not duplicate every large validation fixture here unless repository size stays reasonable.
