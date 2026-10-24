# Calibration results

Campaign `core-c8b22a8-1792799432599`. Policy: retain thresholds when floors are sufficient; never optimize against one corpus.

| family | decision |
| --- | --- |
| gpu | retain_current_threshold |
| io | retain_current_threshold |
| irq | retain_current_threshold |
| scheduler | retain_current_threshold |

## Limitations

- family scheduler: precision None / recall Some(0.0) at the current threshold are below 0.9; the threshold is retained and the diagnosis stays Experimental
- family io: precision None / recall Some(0.0) at the current threshold are below 0.9; the threshold is retained and the diagnosis stays Experimental
- family irq: precision None / recall Some(0.0) at the current threshold are below 0.9; the threshold is retained and the diagnosis stays Experimental
- family gpu: precision None / recall Some(0.0) at the current threshold are below 0.9; the threshold is retained and the diagnosis stays Experimental

Diagnosis capabilities: remain Experimental for v0.1.0.
