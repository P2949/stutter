# Configuration

## GNOME/KDE Wayland foreground attribution

`foreground_source = "gnome"` and `foreground_source = "kde"` are supported only through trusted helper JSON providers:

- `stutter-gnome-foreground --json`
- `stutter-kde-foreground --json`

The helpers must output a single JSON object with the foreground identity fields that are available:

```json
{
  "pid": 1234,
  "app_id": "org.example.App",
  "class": "Example",
  "title": "optional private title",
  "window_id": "optional compositor window id",
  "workspace": "optional workspace",
  "confidence": 0.95,
  "reason": "active foreground window from compositor helper"
}
```

Window titles are still redacted unless foreground title capture is explicitly enabled. `stutter` intentionally does not use GNOME Shell `Eval`, KWin script injection, or a generic Wayland foreground scrape.

## Tuning Proof Workflow

Use [docs/TUNING_WORKFLOW.md](TUNING_WORKFLOW.md) for the supported
advisor/tune/recommend loop. The workflow depends on repeated comparable runs;
underpowered or incomparable results must not be treated as validated fixes.
